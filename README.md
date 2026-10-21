# TrafficBar

A TrafficMonitor-style network meter written in Rust. It embeds itself directly in the
Windows taskbar, to the left of the notification area (outside the `^` overflow), and
shows download / upload speed, plus optional CPU and memory.

- Single portable `TrafficBar.exe`, no runtime DLLs.
- Starts with Windows (HKCU `Run` key, toggle in the menu or in the config).
- Units switch automatically: `B/s → KB/s → MB/s → GB/s`, TrafficMonitor-style decimals
  (`8.92 KB/s`, `10.4 KB/s`, `5.50 MB/s`). Bits, short mode, fixed units or fully custom tiers.
- JetBrains Mono by default; any font, size, weight, colors, layout, offsets.
- Hot-reloaded config: edit `TrafficBar.toml` (created next to the exe on first run), save, done.
- Smooth per-pixel transparency, with automatic fallback to TrafficMonitor's color-key mode.
- Windows 11 taskbar and the classic Windows 10 / ExplorerPatcher taskbar.
- Survives Explorer restarts.

## Use

1. Put `TrafficBar.exe` in a permanent folder (e.g. `%LOCALAPPDATA%\TrafficBar`) — the
   autostart entry points at wherever it is when it runs.
2. Run it. Install [JetBrains Mono](https://www.jetbrains.com/lp/mono/) if you haven't;
   otherwise Windows falls back to another monospace font.
3. Right-click the meter (or the tray icon) for: edit settings, network adapter, speed
   unit, bits / short mode, start with Windows, exit. Double-click opens Task Manager.

## Build

On Windows with Rust installed: `cargo build --release` → `target\release\TrafficBar.exe`.

Cross-compile from Linux: `rustup target add x86_64-pc-windows-gnu`, install
`mingw-w64`, then `cargo build --release --target x86_64-pc-windows-gnu`.

## Layout of the code

| File | What |
|---|---|
| `src/taskbar.rs` | Finds `Shell_TrayWnd`, reparents the widget, positions it next to `TrayNotifyWnd` (Win11) or shrinks `MSTaskSwWClass` (classic) — same approach as TrafficMonitor |
| `src/render.rs` | GDI layout and drawing, per-pixel alpha via `UpdateLayeredWindow` or color key |
| `src/stats.rs` | `GetIfTable2` per-adapter counters, CPU and memory |
| `src/format.rs` | Speed formatting / unit tiers (unit-tested) |
| `src/config.rs` | TOML config with defaults and in-place editing that keeps comments |
| `src/app.rs` | Windows, timers, tray icon, menu, hot reload, autostart |
