# Configuration

GlassDock is configured in its settings window (stored in `settings.json`), with a few environment
variables and the pinned-apps JSON.

## Environment variables

Set them for the dock process. In Hyprland the simplest way is `env = NAME,value` in
`hyprland.conf`, placed **before** the `exec-once` line that starts the dock, or exported in the
shell that runs `glassdock-launch`.

| Variable | Default | Meaning |
| --- | --- | --- |
| `GLASSDOCK_OUTPUTS` | all outputs | Comma-separated output names (`hyprctl monitors`) on which to show the dock. Outputs that are not listed get no dock. |
| `GLASSDOCK_SETTINGS` | `$XDG_CONFIG_HOME/glassdock/settings.json` | Settings file (see below). |
| `GLASSDOCK_USAGE` | `$XDG_STATE_HOME/glassdock/usage.json` (`~/.local/state/...`) | App usage history behind the recommended apps. |
| `GLASSDOCK_NAMESPACE` | `glassdock` | Layer-shell namespace of the dock. Change it if a compositor rule you already have matches another name. |
| `GLASSDOCK_SOCKET` | `$XDG_RUNTIME_DIR/glassdock.sock` | Control socket used by `glassdockctl` and by the settings window to tell the dock to reload. |
| `GLASSDOCK_PINNED` | `$XDG_CONFIG_HOME/glassdock/pinned.json` (`~/.config/glassdock/pinned.json`) | Path of the pinned-apps file. |
| `GLASSDOCK_LANG` | derived from `LC_ALL`, `LC_MESSAGES`, `LANG` | `en` or `es`. Only the dock's own menu text is translated; app names come from `.desktop` files. |
| `GLASSDOCK_DEBUG` | unset | Any value turns on verbose logging of outputs, input and capture events on stderr. |
| `GLASSDOCK_BIN` | next to `glassdock-launch`, then `$PATH` | Which binary `glassdock-launch` runs. |
| `__EGL_VENDOR_LIBRARY_FILENAMES` | Mesa's vendor file if present | Standard glvnd variable. Set it yourself to choose another GPU; GlassDock will not override it. |

Example:

```ini
env = GLASSDOCK_OUTPUTS,eDP-1,DP-1
env = GLASSDOCK_LANG,en
exec-once = glassdock-launch
```

## Settings

`~/.config/glassdock/settings.json` is written by the settings window (`glassdock settings`) and
can be edited by hand. Every key is optional, missing or invalid values fall back to the default,
and numbers are clamped to their range:

```json
{
  "auto_hide": true,
  "live_glass": true,
  "magnification": true,
  "zoom": 1.78,
  "recommended": true,
  "recommended_count": 3
}
```

| Key | Default | Range | Meaning |
| --- | --- | --- | --- |
| `auto_hide` | `true` | | Hide the dock until the pointer touches the bottom edge. `false` keeps it visible and reserves its height (78 px) so windows are not placed under it. |
| `live_glass` | `true` | | The compositor blurs the backdrop every frame (needs the `glassdock-live` layer rule with `blur = on` and `ignore_alpha = 0.18`); moving content shows through the glass. `false`: refracted snapshot, captured whenever the dock appears. A smaller `decoration:blur:size` (e.g. 4 with 2 passes) keeps more detail visible. |
| `magnification` | `true` | | Icons grow under the pointer. |
| `zoom` | `1.78` | 1.25 to 2.0 | Peak magnification. A lower value also makes the dock surface shorter. |
| `recommended` | `true` | | Show the recommended apps after the divider. |
| `recommended_count` | `3` | 1 to 5 | How many of them. |

The running dock notices a changed file within two seconds; `glassdockctl reload` applies it at
once (the settings window does that for you).

### Recommended apps and the usage history

Recommended apps are closed apps that are not pinned, ranked by "frecency": every window an app
opens adds one point, and points halve every seven days. The history is
`~/.local/state/glassdock/usage.json` (desktop-entry ids, a score and a timestamp; no titles, no
paths). Use *Clear usage history* in the settings window to start over (it empties the file). If the
file does not exist yet and GNOME Shell's `~/.local/share/gnome-shell/application_state` does, it is
used once as the starting point, so on such a machine deleting the file by hand imports GNOME's
history again instead of starting empty.

## Pinned apps

`~/.config/glassdock/pinned.json` is a JSON array of desktop-entry ids, in dock order:

```json
[
  "kitty.desktop",
  "firefox.desktop",
  "org.gnome.Nautilus.desktop",
  "code.desktop"
]
```

- Ids are the file names under `/usr/share/applications`, `~/.local/share/applications` and the
  Flatpak and Snap export directories.
- Entries that are not installed are skipped, not deleted.
- If the file does not exist, GlassDock **seeds** it in memory with the first *installed* app of
  each common category (terminal, browser, file manager, editor, ...). It is only written when you
  change something from the dock.
- A corrupt file is not touched at start-up: the dock runs with the seed. If you then change the
  pinned apps from the dock, the old file is first copied to `pinned.json.bak`.
- You normally never edit the file by hand: drag icons to reorder them, and use the right-click menu
  (pin, unpin) and the `+` button.

## Monitor layout

With `monitor = ,preferred,auto,1` Hyprland places external monitors in the order it detects them,
which can change from one session to the next, so the pointer may suddenly cross to a different
screen. Pin the positions:

```ini
monitor = eDP-1,    preferred, 0x0,    1.6
monitor = HDMI-A-1, preferred, 1600x0, 1
monitor = DP-1,     preferred, 3520x0, 1
```

The x offsets are in *logical* pixels: the width of a 2560 px panel at scale 1.6 is 1600. Monitor
names come from `hyprctl monitors`.

## Minimize

Hyprland has no minimize. The dock's menu moves the window to the special workspace `minimized`.
To do the same from the keyboard:

```ini
bind = SUPER,       H, movetoworkspacesilent, special:minimized
bind = SUPER SHIFT, H, togglespecialworkspace, minimized
```

## Layer rules

Do **not** apply a `layerrule = blur` to the `glassdock` namespace. Compositor blur is applied under
the whole layer surface, including its transparent shadow margin, and shows up as a dark halo around
the dock. The dock does its own refraction. If you copied a blur rule for another dock, scope it to
that dock's namespace.

## Appearance constants

Sizes and timings are constants in `src/dock.rs` (`ICON`, `GAP`, `ZOOM`, `ZOOM_RANGE`,
`HIDE_DELAY`, `K_SCALE`, `K_REVEAL`, `K_LABEL`) and the material is in `shaders/glass.frag`
(`BEVEL`, `FOLD`, `DISP`, ...). They are compiled in. To experiment with the shader without
rebuilding:

```bash
glassdockctl shader /path/to/glassdock/shaders
```

The dock recompiles `glass.vert` and `glass.frag` from that directory and keeps the previous
shader if the new one does not compile.
