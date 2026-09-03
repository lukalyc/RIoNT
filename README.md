# nt-tui

Keyboard-only NetworkTables (NT4) inspector for FRC. Rust + ratatui.
Priorities: **latency, keyboard flow, zero visual noise.** Monochrome by
default; color is reserved for state that demands attention (green =
live/editable, yellow = retrying, red = error/down).

## Build

```
cargo build --release
target\release\nt-tui.exe 9986
```

(If `cargo` isn't on your PATH: `$env:PATH += ";$HOME\.cargo\bin"`.)

## Usage

```
nt-tui 9986            # team number -> 10.99.86.2
nt-tui 172.22.11.2     # direct IP (USB tether)
nt-tui 127.0.0.1:5810  # explicit host:port (e.g. robot simulation)
nt-tui                 # last successfully connected target, else 172.22.11.2
```

Connects on TCP 5810 (NT4 WebSocket) and auto-reconnects forever. The last
target is remembered in `.nt-tui-target` next to where you launch from.

## Screens

Three screens, all keyboard-driven:

- **Tree** (default): dual-pane collapsible topic tree + type inspector.
- **Matrix** (`v` or `W` or `1-9`): auto-packed telemetry grid — every cell
  shows a shortened topic name, bold value, Hz and an inline sparkline.
  No dragging or resizing: 1-4 columns adapt to your terminal width.
- **Zoom** (`z` in the matrix): the selected cell explodes into a
  full-width multi-row ASCII plot of the last ~10s. `z` again returns.

### Building the matrix in under 3 seconds

- `/` fuzzy search, then **`space`/`tab` toggles `[x]` on each match**
  (lazygit-style batch staging), then `Enter` — all staged topics land in
  the matrix at once.
- `W` on a directory in the tree adds its **entire subtree** as a wildcard:
  topics the robot publishes later are adopted automatically.
- `1`-`9` load workspace presets from `.nt-views.json` (below).

### Workspace presets (views as code)

`.nt-views.json` next to where you run nt-tui — commit it to your team repo
so everyone gets the same views:

```json
[
  {"name": "Swerve",  "topics": ["Swerve/*", "SmartDashboard/Battery Voltage", "SmartDashboard/Gyro Angle"]},
  {"name": "Shooter", "topics": ["SmartDashboard/Shooter RPM", "SmartDashboard/kP", "SmartDashboard/kI", "SmartDashboard/kD"]},
  {"name": "Vision",  "topics": ["limelight-front/*"]}
]
```

A trailing `/*` means "every topic under this subtree". Presets are listed
in the empty-matrix screen and load with `1`-`9` in file order.

## Keymap

| Key | Tree | Matrix | Zoom |
| --- | --- | --- | --- |
| `j` `k` / up/down | move / scroll inspector | move one row | — |
| `h` `l` / left/right | toggle folder fold | move one column | — |
| `g` / `G` | top / bottom | first / last cell | — |
| `tab` | tree <-> inspector | — | — |
| `e` / `Enter` | edit value (bool/int/double/string) | edit selected cell | — |
| `z` | — | zoom cell into full plot | close zoom |
| `v` | open matrix | back to tree | back to matrix |
| `W` | add topic / subtree to matrix | — | — |
| `/` | fuzzy search; `space` stages, `Enter` jumps or adds staged | — | — |
| `s` / `d` | snapshot / diff (`~` changed `+` added `-` removed) | — | — |
| `c` | connect overlay (team number or `host[:port]`) | | |
| `R` | reconnect now (status shows progress) | | |
| `1`-`9` | load workspace preset | | |
| `q` / `Ctrl-C` | quit | | |

## Architecture

```
src/main.rs          CLI (team/IP resolve), terminal setup, main select() loop
src/app.rs           App state + all key handling. Pure logic, no rendering.
src/nt/client.rs     Async NT4 task: owns the socket, never blocks the UI.
src/nt/store.rs      Topic store: values, Hz windowing, sparkline history.
src/ui/mod.rs        Layout + rendering (tree/inspector/matrix/zoom).
src/ui/tree.rs       Collapsible topic tree model (rebuilt per frame).
src/ui/sparkline.rs  Single-row ASCII waveform.
src/ui/plot.rs       Multi-row ASCII plot for zoom mode.
```

Threading model: the UI thread (`tokio` main task) only renders and handles
keys. All socket IO lives in the client task and reaches the UI through
unbounded MPSC channels (`NtUpdate` downstream, `ClientCommand` upstream),
batched at 50 ms. The render loop ticks at ~120 Hz but ratatui's diffing
means only changed cells hit the terminal.

Live metrics per topic: publish rate (Hz, sliding window) and a rolling
history for sparklines/plots. Header shows RTT, summed stream Hz, topic
count, and robot uptime (delta of server timestamps).

## Protocol notes

The client speaks NT4 over WebSocket (subprotocol
`networktables.first.wpi.edu`, path `/nt/<client-name>`): JSON control
messages as text frames, MessagePack value frames
`[pubuid, timestamp_us, type, value]` as binary frames (validated against
ntcore 2026.2.2 source and a real ntcore server). Subscribe-all with fast
periodic, RTT/clock sync via `-1` timestamp echo, publish with automatic
retransmission, auto-reconnect with retry counter, 5s connect timeout.

## Testing

```
conda activate nt-tui-test
python test/server.py    # fake robot: real ntcore NT4 server on 5814
python test/harness.py   # 69-check end-to-end suite (headless TUI + pyte)
```

The harness drives the TUI via stdin keystroke scripts, renders through
pyte, mutates values from a second ntcore client, and exercises search
staging, matrix presets, zoom, snapshot/diff, edit round-trips, retarget
and reconnect flows.
