//! Which apps you use, to suggest the recommended ones (recent and frequent apps that are
//! closed and not pinned).
//!
//! Every time a window of an app opens, the app gets one point. Points lose half of their value
//! every week ("frecency"), so an app used often last month and an app opened an hour ago both
//! rank high. Only desktop-entry ids are stored (no window titles, no paths).
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

const HALF_LIFE_SECS: f64 = 7.0 * 24.0 * 3600.0;
/// Entries below this (decayed) score are forgotten when saving.
const PRUNE_BELOW: f64 = 0.05;
/// Entries below this score are never recommended.
const MIN_SCORE: f64 = 0.2;
const MAX_ENTRIES: usize = 300;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Rec {
    score: f64,
    /// Unix time of the last update, in seconds.
    last: u64,
    /// Windows opened in total (informational).
    count: u64,
}

impl Rec {
    fn decayed(&self, now: u64) -> f64 {
        let dt = now.saturating_sub(self.last) as f64;
        self.score * 0.5f64.powf(dt / HALF_LIFE_SECS)
    }
}

pub struct Usage {
    path: PathBuf,
    map: HashMap<String, Rec>,
    dirty: bool,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `$XDG_DATA_HOME/gnome-shell/application_state` (default `~/.local/share/gnome-shell/...`).
fn gnome_state_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(".local/share"));
    base.join("gnome-shell/application_state")
}

/// Value of `name="..."` inside one XML tag.
fn xml_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

/// Reads GNOME Shell's `application_state` (`<application id=".." score=".." last-seen=".."/>`).
/// Its scores are open-ended, so they are squashed with a logarithm into our scale; the last-seen
/// time keeps the usual weekly decay, so only apps used in the last weeks stay recommendable.
fn parse_gnome_state(xml: &str) -> HashMap<String, Rec> {
    let mut out = HashMap::new();
    for part in xml.split("<application ").skip(1) {
        let Some(tag) = part.split('>').next() else {
            continue;
        };
        let (Some(id), Some(score), Some(last)) =
            (xml_attr(tag, "id"), xml_attr(tag, "score"), xml_attr(tag, "last-seen"))
        else {
            continue;
        };
        let (Ok(score), Ok(last)) = (score.parse::<f64>(), last.parse::<u64>()) else {
            continue;
        };
        if !id.ends_with(".desktop") || !score.is_finite() || score <= 0.0 {
            continue;
        }
        out.insert(
            id.to_string(),
            Rec {
                score: ((1.0 + score).ln() / 2.0).clamp(1.0, 5.0),
                last,
                count: 0,
            },
        );
    }
    out
}

impl Usage {
    /// `$GLASSDOCK_USAGE`, otherwise `$XDG_STATE_HOME/glassdock/usage.json`.
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os("GLASSDOCK_USAGE").filter(|p| !p.is_empty()) {
            return PathBuf::from(p);
        }
        crate::config::state_dir().join("usage.json")
    }

    /// Like `load`, but the very first time (no file yet) it starts from the app history that
    /// GNOME Shell keeps for its own app picker, if there is one: a Hyprland session on a machine
    /// that also has GNOME then does not begin with nothing to recommend. Nothing is read from
    /// GNOME again afterwards, and nothing is ever written there.
    pub fn load_or_seed(path: &Path) -> Usage {
        let mut u = Usage::load(path);
        if !path.exists() {
            if let Ok(xml) = std::fs::read_to_string(gnome_state_path()) {
                u.map = parse_gnome_state(&xml);
                u.dirty = !u.map.is_empty();
                let _ = u.save(now_secs());
            }
        }
        u
    }

    /// Loads the history; a missing or corrupt file starts empty.
    pub fn load(path: &Path) -> Usage {
        let map = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<HashMap<String, Rec>>(&t).ok())
            .unwrap_or_default();
        Usage {
            path: path.to_path_buf(),
            map,
            dirty: false,
        }
    }

    /// One more window of `key` (a desktop-entry id) opened at `now`.
    pub fn record(&mut self, key: &str, now: u64) {
        if !key.ends_with(".desktop") {
            return;
        }
        let r = self.map.entry(key.to_string()).or_insert(Rec {
            score: 0.0,
            last: now,
            count: 0,
        });
        r.score = r.decayed(now) + 1.0;
        r.last = now;
        r.count += 1;
        self.dirty = true;
    }

    /// Removes one app from the history (the "remove from recommendations" menu item).
    pub fn forget(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub fn score(&self, key: &str, now: u64) -> f64 {
        self.map.get(key).map_or(0.0, |r| r.decayed(now))
    }

    /// Best apps first (highest frecency, then most recent), skipping the ones `skip` rejects
    /// (running, pinned, not installed...). At most `limit` keys.
    pub fn ranked(&self, now: u64, limit: usize, skip: impl Fn(&str) -> bool) -> Vec<String> {
        let mut v: Vec<(&String, f64, u64)> = self
            .map
            .iter()
            .map(|(k, r)| (k, r.decayed(now), r.last))
            .filter(|(k, s, _)| *s >= MIN_SCORE && !skip(k))
            .collect();
        // Total order: score, then recency, then the key (so equal entries never flip).
        v.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.2.cmp(&a.2))
                .then(a.0.cmp(b.0))
        });
        v.into_iter().take(limit).map(|(k, _, _)| k.clone()).collect()
    }

    /// Writes the history if it changed (atomically). Old, tiny entries are dropped.
    pub fn save(&mut self, now: u64) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        self.map.retain(|_, r| r.decayed(now) >= PRUNE_BELOW);
        if self.map.len() > MAX_ENTRIES {
            let mut keys: Vec<(String, f64)> = self.map.iter().map(|(k, r)| (k.clone(), r.decayed(now))).collect();
            keys.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (k, _) in keys.into_iter().take(self.map.len() - MAX_ENTRIES) {
                self.map.remove(&k);
            }
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string(&self.map).map_err(io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        self.dirty = false;
        Ok(())
    }

    /// Empties the history file (the "clear usage history" button). It writes an empty history
    /// instead of deleting the file: with no file, the next start would import GNOME's history
    /// again, and the clear would be undone.
    pub fn clear_file(path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, "{}")?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 3600;

    fn empty() -> Usage {
        Usage::load(
            &std::env::temp_dir()
                .join("glassdock-tests")
                .join("usage-does-not-exist.json"),
        )
    }

    #[test]
    fn frequent_beats_rare_and_recent_beats_old() {
        let mut u = empty();
        let t0 = 1_000_000u64;
        for i in 0..6 {
            u.record("often.desktop", t0 + i * 60);
        }
        u.record("once.desktop", t0);
        u.record("fresh.desktop", t0 + 20 * DAY);
        let now = t0 + 20 * DAY + 60;
        let r = u.ranked(now, 5, |_| false);
        // `fresh` (1 point, today) outranks the old `often` (6 points, 20 days ago: ~0.9) and `once`.
        assert_eq!(r[0], "fresh.desktop");
        assert!(u.score("often.desktop", now) > u.score("once.desktop", now));
    }

    #[test]
    fn skip_limit_and_non_desktop_keys() {
        let mut u = empty();
        let t = 5_000_000u64;
        u.record("a.desktop", t);
        u.record("b.desktop", t + 1);
        u.record("c.desktop", t + 2);
        u.record("class:Weird", t); // ignored: not a desktop entry
        let r = u.ranked(t + 10, 2, |k| k == "c.desktop");
        assert_eq!(r, vec!["b.desktop", "a.desktop"]);
        assert_eq!(u.ranked(t + 10, 10, |_| false).len(), 3);
    }

    #[test]
    fn old_history_decays_away() {
        let mut u = empty();
        u.record("old.desktop", 0);
        assert!(u.ranked(60 * DAY, 3, |_| false).is_empty());
    }

    #[test]
    fn forget_and_roundtrip() {
        let dir = std::env::temp_dir()
            .join("glassdock-tests")
            .join(format!("usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("usage.json");
        let mut u = Usage::load(&p);
        let t = now_secs();
        u.record("x.desktop", t);
        u.record("y.desktop", t);
        u.save(t).unwrap();
        let mut u2 = Usage::load(&p);
        assert_eq!(u2.ranked(t, 5, |_| false).len(), 2);
        u2.forget("x.desktop");
        u2.save(t).unwrap();
        assert_eq!(Usage::load(&p).ranked(t, 5, |_| false), vec!["y.desktop"]);
        let mut bad = std::fs::File::create(dir.join("bad.json")).unwrap();
        std::io::Write::write_all(&mut bad, b"not json").unwrap();
        assert!(Usage::load(&dir.join("bad.json")).ranked(t, 5, |_| false).is_empty());
    }

    #[test]
    fn imports_gnome_shell_history_once() {
        let xml = r#"<?xml version="1.0"?>
<application-state>
  <context id="">
    <application id="firefox.desktop" score="22647.6" last-seen="2000000"/>
    <application id="kitty.desktop" score="157.0" last-seen="1990000"/>
    <application id="broken.desktop" score="oops" last-seen="1"/>
    <application id="not-an-app" score="9" last-seen="1"/>
    <application id="unused.desktop" score="0" last-seen="1"/>
  </context>
</application-state>"#;
        let m = parse_gnome_state(xml);
        assert_eq!(m.len(), 2, "only valid desktop entries with a positive score");
        assert!(m["firefox.desktop"].score > m["kitty.desktop"].score);
        assert!(m["firefox.desktop"].score <= 5.0);
        assert_eq!(m["kitty.desktop"].last, 1_990_000);
    }

    #[test]
    fn seed_is_only_used_when_there_is_no_history_file() {
        let dir = std::env::temp_dir()
            .join("glassdock-tests")
            .join(format!("seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("gnome-shell")).unwrap();
        let t = now_secs();
        std::fs::write(
            dir.join("gnome-shell/application_state"),
            format!(r#"<application id="a.desktop" score="50" last-seen="{t}"/>"#),
        )
        .unwrap();
        std::env::set_var("XDG_DATA_HOME", &dir);
        let p = dir.join("usage.json");
        let u = Usage::load_or_seed(&p);
        assert_eq!(u.ranked(t, 5, |_| false), vec!["a.desktop"]);
        assert!(p.exists(), "the seed is saved");
        // An existing (even empty) history wins over GNOME's.
        std::fs::write(&p, "{}").unwrap();
        assert!(Usage::load_or_seed(&p).ranked(t, 5, |_| false).is_empty());
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn clearing_keeps_an_empty_file_so_gnome_is_not_imported_again() {
        let dir = std::env::temp_dir()
            .join("glassdock-tests")
            .join(format!("clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("usage.json");
        let mut u = Usage::load(&p);
        u.record("x.desktop", now_secs());
        u.save(now_secs()).unwrap();
        Usage::clear_file(&p).unwrap();
        assert!(p.exists(), "the file stays, so load_or_seed never re-seeds");
        assert!(Usage::load(&p).ranked(now_secs(), 5, |_| false).is_empty());
    }
}
