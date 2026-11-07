# standalone-tray

A system dashboard for Wayland, in an ordinary application window.

Live system graphs, an htop-style process table, world clocks, a calendar, and
tray applets, all in one GTK 4 window. Tile it, float it, make it fullscreen, or
hide it until you need it.

Built for minimal Wayland window managers, where you assemble your own desktop
instead of using a full desktop environment. Keep system status in a window you
can summon and dismiss, managed like any other application, without a panel
reserving screen space.

## What's inside

- **System graphs** for CPU, RAM, network traffic, and physical disk I/O. They
  update every second and keep two minutes of history. Swap usage appears next
  to the RAM reading.
- **A live process table** with CPU and RAM usage, resident memory, PID, user,
  state, thread count, nice level, and command. Sort by any column, filter by
  command, user, or PID, and resize the columns. Readings update every two seconds.
- **Local and world clocks**, with seconds and a date for each location. The
  local clock shows the system timezone name, such as `Europe/Paris`, and its UTC
  offset. World clocks cover UTC, New York, California (Los Angeles), India
  (Kolkata), and Australia (Sydney), following each location's daylight saving
  rules.
- **A month calendar** with week numbers, month navigation, and a Today button.
- **A system tray** for SNI/AppIndicator applications such as `nm-applet` and
  `blueman-applet`, with native context menus and live icon updates.
- **Volume and battery indicators**: default-sink volume and mute status, plus
  battery percentage and charging information. They hide when their backends
  are unavailable.

The dark layout grows with the window. Drag the divider between the graphs and
process table to give either more space; smaller windows scroll.

## Install

You need Linux, a Rust toolchain, and the GTK 4 development libraries:

- Debian/Ubuntu: `libgtk-4-dev`
- Fedora: `gtk4-devel`
- Arch: `gtk4`

From this checkout:

```sh
cargo install --path . --locked
```

Or build a release binary at `target/release/standalone-tray`:

```sh
cargo build --release --locked
```

Optional runtime dependencies:

- `wpctl` (WirePlumber) and `pactl` for volume control.
- UPower for battery status.
- SNI/AppIndicator applets for the tray icons you want to use.

System readings come directly from Linux `/proc` and `/sys`. World clocks use
GLib and the system timezone data. No separate monitoring service is required.

## Use

```sh
standalone-tray
```

Add that command to your compositor's startup configuration to launch the
dashboard with your session. There are no command-line settings or configuration
files; the clock locations are currently defined in [src/calendar.rs](src/calendar.rs).

Only one `StatusNotifierWatcher` can own the session bus name at a time. Stop
other tray hosts, such as Waybar's tray module, before starting standalone-tray.

### Tray and audio controls

- Left click a tray icon to activate it, or open its menu for menu-only items.
- Middle click sends the item's secondary activation action.
- Right click opens the item's context menu.
- Left click the volume indicator to toggle mute.
- Scroll over the volume indicator to adjust it in 5% steps.
- Hover over the battery indicator for charging or remaining-time information.

The process table is read-only. Click its headings to change the sort order and
use the search field to narrow the list. Hover over a command to see the full
command line.

## Reading the numbers

The CPU graph shows usage across all cores on a fixed 0–100% scale. Process CPU
follows htop's convention: 100% means one fully busy core, so a multithreaded
process can exceed 100%. A new process needs a full sampling interval before its
CPU usage is available.

The RAM graph also uses a fixed 0–100% scale. Used RAM excludes memory available
for reuse, including reclaimable cache. Process RAM is resident memory (RSS),
shown both as a size and a percentage of total physical RAM.

Network graphs combine all interfaces except loopback, including virtual
interfaces. Disk graphs combine whole physical devices, excluding partitions and
device-mapper layers to avoid counting the same I/O twice. Both graphs scale to
the visible history; solid lines show receive/read and dashed lines show
send/write.

Unavailable graph readings leave gaps. Processes that exit or cannot be read are
skipped.

## Development

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Graphical tests are ignored by default. Run each check separately on a test
display:

```sh
cargo test --locked clocks_fit_compact_sidebar -- --ignored
cargo test --locked live_view_sorts_and_filters_across_refreshes -- --ignored
```

Set `TRAY_DEBUG=1` to log tray events and volume/battery updates to stderr.
The UPower watcher example is available with `cargo run --example battery_watch`.

## License

MIT. See [LICENSE](LICENSE).
