# RIONT — Robot Inspection Over Network Tables

RIONT (Robot Inspection Over Network Tables) is a keyboard-only NetworkTables
(NT4) dashboard for FRC. Rust + ratatui.

v0.2.0 presents a persistent two-zone layout: a **35% left control column**
(collapsible Topic Tree + a passive bottom-left Inspector Dock) and a **65%
full-height Watchlist Canvas** that auto-packs pinned topics into a
responsive, type-aware card matrix — under a Driver-Station style HUD.

```
 RIONT v0.2.0     [COMM: ONLINE (10.99.86.2)]  [CODE: RUNNING]  [UPTIME: 00:04:12]   163 topics
┌ TOPIC TREE (35% W, 70% H) ┐┌ WATCHLIST CANVAS (65% W, 100% H) ─────────────────────────────┐
│ > [-] limelight-front     │ ┌─ limelight-front/tv ─────────────┐ ┌─ Swerve/FL_Angle ─────┐ │
│       botpose_wpiblue     │ │ 1.0000                           │ │ 182.4°                │ │
│   > * tv     [double]     │ │ double   50.0 Hz   Δ 20ms        │ │ double   50.0 Hz      │ │
│   [+] limelight-rear      │ └──────────────────────────────────┘ └───────────────────────┘ │
├───────────────────────────┤ ┌─ StateMachine/ActiveState ───────┐ ┌─ Battery/Voltage ─────┐ │
│ INSPECTOR DOCK (35% W,    │ │ "INTAKING"                       │ │ 12.42 V               │ │
│ 30% H) — passive          │ │ string   0.1 Hz    Δ 14.2s       │ │ double   20.0 Hz      │ │
└───────────────────────────┘ └──────────────────────────────────┘ └───────────────────────┘ │
```

## Build

```
cargo build --release
target\release\riont.exe 9986
```

(If `cargo` isn't on your PATH: `$env:PATH += ";$HOME\.cargo\bin"`.)

## Usage

```
riont 9986            # team number -> 10.99.86.2
riont 172.22.11.2     # direct IP (USB tether)
riont 127.0.0.1:5810  # explicit host:port (e.g. robot simulation)
riont                 # last successfully connected target, else 172.22.11.2
```

Connects on TCP 5810 (NT4 WebSocket) and auto-reconnects forever. The last
target is remembered in `.riont-target` next to where you launch from.

## Layout

- **Top HUD (1 line):** `COMM` (green `ONLINE <ip>` / flashing amber
  `RECONNECTING...` / red `DISCONNECTED`), `CODE` (`RUNNING` while robot
  frames stream within 500 ms, `STOPPED` when the connection is alive but
  the user loop went quiet, `--` offline), and `UPTIME` — rendered
  `HH:MM:SS` from the **robot's server clock**: on a roboRIO NT4
  timestamps are FPGA µs since boot, so the latest value timestamp IS the
  uptime; epoch-based off-robot servers fall back to the first→last delta.
  Topic count rounds it out. No raw RTT, no global Hz.
- **Topic Tree (35% W, 70% H):** collapsible namespaces with inline values,
  type tags (green = editable), and an amber `*` on topics pinned to the
  watchlist. Folding is silent — the tree state is its own feedback. The
  selected row inverts the FULL row (tag, value, rate included), so the
  highlight never clips content. Active pane border: amber (tree) /
  cyan (watchlist); inactive panes use muted grey `#3C3836`.
- **Inspector Dock (35% W, 30% H):** strictly passive — instantly mirrors
  the full path, type, flags, rate, Δ and raw value of the tree cursor. No
  Tab navigation needed to read metadata.
- **Watchlist Canvas (65% W, 100% H):** every pinned topic becomes a
  bordered card. **Height-first stacking:** column 1 fills 100% of the
  available height (using each card's real rendered height — arrays wrap
  to a second sub-row) before column 2 is instantiated, up to 3 columns;
  capacity recalculates live on terminal resize.

### Card adapters

- **Numbers:** large bright-cyan readout.
- **Booleans:** high-visibility badges — bright green `[ TRUE ]`, muted red
  `[ FALSE ]` (beam breaks and limit switches are readable across the pit).
- **Strings:** quoted `"INTAKING"` with strict single-line ellipsis.
- **Arrays:** cyan bracket grouping; dense arrays (e.g. Limelight 6-DOF)
  wrap onto a second sub-row with an ellipsis cap.
- **Telemetry neutrality:** rates and Δ timers render in muted grey — no
  `[STALE]` flags, no red alarms for slow-but-alive topics. Low frequency
  is normal for setpoints and state machines.

## Keymap

| Key | Tree | Watchlist |
| --- | --- | --- |
| `j` `k` / up-down | move | move row |
| `h` `l` / left-right | toggle folder fold | move card |
| `g` / `G` | top / bottom | first / last card |
| `Space` | pin topic to watchlist (dir = whole subtree) | unpin active card |
| `x` | — | dismiss active card |
| `e` / Enter | edit value (bool/int/double/string) | edit active card |
| `Tab` | -> watchlist | -> tree |
| `/` | fuzzy search; `Space` pins a match, `Enter` jumps to it | |
| `c` / `Shift+C` | **Connection Picker** (below) | |
| `:` / `Ctrl-P` | **Command Palette** (below) | |
| `1`-`9` | load workspace preset | |
| `q` / `Ctrl-C` | quit | |

### Connection Picker (`c`)

Select and connect only — zero management options:

```
┌─ [CONNECT TARGET] ──────────────────────────┐
│   connect to: ... (free text input)         │
│ > [1] Simulation       127.0.0.1:5810       │
│   [2] USB Tether       172.22.11.2          │
│   [3] Team 118         10.1.18.2            │
│ [Enter] Connect   [j/k] Select   [Esc]      │
└─────────────────────────────────────────────┘
```

Saved targets come from `config.json`, most recently used first. Digits
are ordinary input (IPs start with them — quick-jump-on-digit would hijack
address typing); select with `j`/`k`/arrows, `Enter` connects the typed
address or the highlighted entry, `Esc` cancels.

### Command palette (`:` or `Ctrl+P`)

Universal searchable action runner — every entry names its subsystem:

- `Settings: Open Configuration` — opens `config.json` in `$VISUAL`/
  `$EDITOR` (notepad on Windows, `vi` elsewhere); the TUI suspends and
  restores around the editor.
- `Settings: View Settings` — read-only summary (targets, presets, SSH).
- `Settings: Add Robot Target` — focused single-input prompt
  (`IP/Team (e.g. "Practice 10.99.86.2" or "118"): [ ]`); validates,
  appends to config, toasts `Added Team 118`.
- `Settings: Remove Robot Target` — picker; `Enter` removes, toasts.
- `Watchlist: Save Active as Preset` / `Load Preset` / `Clear All`.
- `NetworkTables: Reconnect Socket` — **pure client action**: drops the
  socket and re-handshakes. Never queries a topic.
- `System: Restart Robot Code` — **primary method: SSH**. Dispatches a
  background `ssh <user>@<robot_ip> <restart_cmd>` (defaults:
  `admin@<robot_ip> /usr/local/frc/bin/frcRunRobot.sh restart`), fully
  decoupled from NetworkTables. The outcome reports as a toast, e.g.
  `[SUCCESS] robot code restarted in 1.42s` or `[ERROR] restart failed: …`.
- `Copy Active Topic Path` — system clipboard (arboard).

The three concerns stay architecturally separate (SRP): the **Connection
Picker** selects, the **Palette** triggers, **Settings** persists.

### Settings & persistence

Everything lives in `~/.config/riont/config.json`:

```json
{
  "last_target": "10.1.18.2",
  "saved_targets": [
    {"name": "Simulation", "ip": "127.0.0.1:5810"},
    {"name": "USB Tether", "ip": "172.22.11.2"},
    {"name": "Team 118",    "ip": "10.1.18.2"},
    {"name": "Team 9986",   "ip": "10.99.86.2"}
  ],
  "presets": {
    "swerve": ["/Swerve/FL_Angle", "/Swerve/FR_Angle"],
    "vision": ["/limelight-front/tv", "/limelight-front/botpose_wpiblue"]
  },
  "system": {
    "ssh_user": "admin",
    "restart_cmd": "/usr/local/frc/bin/frcRunRobot.sh restart"
  }
}
```

The old `.nt-views.json` preset file is still honored when the config has
no presets. Presets load with `1`-`9` or via the palette's Load Preset
picker.

### Editing

`e` opens an inline prompt at the bottom of the screen:

```
Set limelight-front/tv:  [ 1.0000_ ]
```

`Enter` publishes over NT4 (with automatic retransmission of the first
value frame); `Esc` cancels without writing.

### Workspace presets (views as code)

Presets live in `config.json` under `"presets"` (name → topic list) and are
saved from the palette (`Watchlist: Save Active as Preset`). For bulk edits
use `Settings: Open Configuration` — commit the file to your team repo so
everyone gets the same views:

```json
{
  "presets": {
    "Swerve": ["Swerve/*", "SmartDashboard/Battery Voltage"],
    "Vision": ["limelight-front/*"]
  }
}
```

A trailing `/*` means "every topic under this subtree" — topics the robot
starts publishing later are adopted automatically. Load with `1`-`9` or via
the palette's Load Preset picker.

## Architecture

```
src/main.rs          CLI (team/IP resolve), terminal setup, editor suspend/resume
src/app.rs           App state, key handling, palette/toasts. Pure logic, no rendering.
src/config.rs        ~/.config/riont/config.json: targets, presets, SSH settings.
src/nt/client.rs     Async NT4 task + background SSH restart. Owns the socket, never blocks the UI.
src/nt/store.rs      Topic store: values, metadata, Hz windowing.
src/ui/mod.rs        Layout: HUD, tree, inspector dock, watchlist card matrix, overlays.
src/ui/tree.rs       Collapsible topic tree model (rebuilt per frame).
```

The WebSocket client identifies itself as `riont` (connect path
`/nt/riont`).

Threading model: the UI thread (`tokio` main task) only renders and handles
keys. All socket IO lives in the client task and reaches the UI through
unbounded MPSC channels (`NtUpdate` downstream, `ClientCommand` upstream),
batched at 50 ms. The render loop ticks at ~120 Hz but ratatui's diffing
means only changed cells hit the terminal.

Per-topic telemetry: publish rate (Hz, 2 s sliding window) and Δ since last
change. The client measures RTT/clock offset internally — used only for
clock-synced publishes, never shown as a headline metric.

## Protocol notes

The client speaks NT4 over WebSocket (subprotocol
`networktables.first.wpi.edu`, path `/nt/<client-name>`): JSON control
messages as text frames, MessagePack value frames
`[pubuid, timestamp_us, type, value]` as binary frames (validated against
ntcore 2026.2.2 source and a real ntcore server). Subscribe-all with fast
periodic, clock sync via `-1` timestamp echo, publish with automatic
retransmission, auto-reconnect with retry counter, 5s connect timeout.

## Testing

```
conda activate nt-tui-test
python test/server.py    # fake robot: real ntcore NT4 server on 5814
python test/harness.py   # 81-check end-to-end suite (headless TUI + pyte)
python test/smoke.py     # visual smoke dump of the RIONT layout
```

The harness drives the TUI via stdin keystroke scripts, renders through
pyte, mutates values from a second ntcore client, and exercises search
jump/pin, watchlist packing + `x` removal, the command palette, inline edit
round-trips, retarget and reconnect flows.
