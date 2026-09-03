# nt-tui

Keyboard-only NetworkTables (NT4) inspector for FRC. Rust + ratatui.
Priorities: **latency, keyboard flow, zero visual noise.** No color; plain
`bold`/`dim`/`reverse` modifiers only.

## Build

```
cargo build --release
target\release\nt-tui.exe 9986
```

(If `cargo` isn't on your PATH: `$env:PATH += ";$HOME\.cargo\bin"`.)

## Usage

```
nt-tui 9986        # team number -> 10.99.86.2
nt-tui 172.22.11.2 # direct IP (USB tether)
nt-tui             # falls back to 172.22.11.2
```

Connects on TCP 5810 (NT4 WebSocket) and auto-reconnects forever.

## Keymap

| Key | Action |
| --- | --- |
| `j` `k` / arrows | move cursor (tree / watchlist / scroll inspector) |
| `g` / `G` | top / bottom |
| `h` `l` / left/right | collapse / expand directory |
| `Tab` / `Shift+Tab` | cycle focus: tree -> inspector -> watchlist |
| `Space` | pin/unpin topic under cursor to the watchlist HUD |
| `e` / `Enter` | edit value of writable topic (bool/int/double/string) |
| `/` | fuzzy search any topic path (type fragments, `j/k`, `Enter` jumps) |
| `s` | take a value snapshot |
| `d` | diff current values against snapshot (~/+/-) |
| `R` | restart robot code (SSH hook, see roadmap) |
| `q` / `Ctrl-C` | quit |

## Architecture

```
src/main.rs          CLI (team/IP resolve), terminal setup, main select() loop
src/app.rs           App state + all key handling. Pure logic, no rendering.
src/nt/client.rs     Async NT4 task: owns the socket, never blocks the UI.
src/nt/store.rs      Topic store: values, Hz windowing, sparkline history.
src/ui/mod.rs        Layout + monochrome rendering.
src/ui/tree.rs       Collapsible topic tree model (rebuilt per frame).
src/ui/sparkline.rs  ASCII waveform renderer.
proto/               NT4 protobuf (compiled via protox, no protoc needed)
```

Threading model: the UI thread (`tokio` main task) only renders and handles
keys. All socket IO lives in the client task and reaches the UI through
unbounded MPSC channels (`NtUpdate` downstream, `ClientCommand` upstream),
batched at 50 ms. The render loop ticks at ~120 Hz but ratatui's diffing
means only changed cells hit the terminal.

Live metrics per topic: publish rate (Hz, 2 s sliding window), staleness
(`STALE` flag when age > 1 s), and a rolling sparkline for numeric streams.
Header shows RTT (from NT4 ping/pong), summed stream Hz, topic count, and
robot uptime (delta of server timestamps; NT4 robot timestamps are us since
boot on the roboRIO FPGA clock).

## Protocol notes / assumptions

The client speaks NT4: length-prefixed frames on 5810, first frame is the
JSON server hello, then protobuf `MessageFrame`s. Two details were written
against the spec but should be validated against WPILib on the first real
robot connect:

- `Value.topic_name` carries the topic directly (field 12).
- `pubuid` is `uint64` in ClientPublish/ServerPublish.

Both live in `src/nt/client.rs` and `proto/NetworkTables.proto` — trivial to
adjust if the robot rejects a handshake.

## Roadmap (framework hooks already in place)

- `Shift+R`: robot code restart over SSH (russh/ssh2); currently surfaces
  the intent via `ClientCommand` and the status bar.
- Process-vs-OS uptime split: needs a server-side topic or NT4 server role.
- Persistent watchlist across runs (file next to the exe).
- Tree rebuild caching with versioning if topic counts get large.
