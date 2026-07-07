# standalone-tray

A system tray in a plain Wayland window.

`standalone-tray` is a StatusNotifierHost (SNI/AppIndicator host) that displays
tray icons inside an ordinary application window — no layer-shell, no panel.
The window can be tiled, floated, embedded, moved or hidden by any Wayland
compositor like any other application. It is meant for compositors (such as
EWM) where you want a tray without running a desktop panel like Waybar: a
panel permanently reserves screen space, while a regular window can be
summoned and dismissed like any other. The point is to keep as much screen
as possible while still having the convenience of `nm-applet`,
`blueman-applet` and friends.

It also shows a volume and a battery indicator next to the tray icons.

## Features

- **StatusNotifier host**: registers a `StatusNotifierWatcher` and
  `StatusNotifierHost`, so applications like `nm-applet` and `blueman-applet`
  find it automatically.
- **Live updates**: icons, tooltips, status and menus update without
  restarting; items appear and disappear as applications register and exit.
- **Clicks**: left click activates the item (or opens its menu for menu-only
  items), middle click sends `SecondaryActivate`, right click opens the
  context menu (DBusMenu, rendered as a native GTK popover).
- **Volume widget**: icon + percentage for the default PipeWire/PulseAudio
  sink. Left click toggles mute, scrolling changes the volume in 5% steps.
- **Battery widget**: icon + percentage from UPower, with time-to-empty /
  time-to-full in the tooltip.

Widgets hide themselves when their backend is unavailable (no audio server,
no battery), so the same binary works on desktops.

## Installation

Dependencies:

- GTK 4 development libraries (`libgtk-4-dev` on Debian/Ubuntu,
  `gtk4-devel` on Fedora, `gtk4` on Arch)
- a Rust toolchain (https://rustup.rs)

Runtime (all optional):

- `wpctl` (WirePlumber) and `pactl` for the volume widget
- UPower for the battery widget

Build and install:

```console
$ cargo install --path .
```

or just build a release binary at `target/release/standalone-tray`:

```console
$ cargo build --release
```

## Usage

Run the binary; there is no configuration:

```console
$ standalone-tray
```

It immediately starts hosting StatusNotifierItems. Note that only one
StatusNotifierWatcher can own the bus name at a time, so stop other trays
(Waybar's tray module, etc.) before starting it.

Set `TRAY_DEBUG=1` to log tray events and widget updates to stderr.

## License

MIT
