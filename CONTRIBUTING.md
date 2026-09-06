# Contributing to RIONT

Thanks for contributing. RIONT has a narrow mission — see `ROADMAP.md`
for what it is and what it deliberately is not; feature proposals should
fit it.

## Building from source

```
cargo build --release
target/release/riont 9986
```

(Windows, if `cargo` isn't on PATH: `$env:PATH += ";$HOME\.cargo\bin"`.)

## Code layout

```
src/main.rs          CLI (team/IP resolve), terminal setup, editor suspend/resume
src/app.rs           App state, key handling, palette/toasts. Pure logic, no rendering.
src/config.rs        ~/.config/riont/config.json: targets, presets, SSH settings.
src/nt/client.rs     Async NT4 task + background SSH restart. Owns the socket, never blocks the UI.
src/nt/store.rs      Topic store: values, metadata, Hz windowing, pose trails.
src/pose.rs          Conservative pose classification + struct:Pose2d decoding.
src/field.rs         Field maps (built-ins + PathPlanner JSON loader) + card math.
src/ui/mod.rs        Layout: HUD, tree, inspector dock, watchlist card matrix, overlays.
src/ui/tree.rs       Collapsible topic tree model (rebuilt per frame).
scripts/             release.sh, screenshot.py, fetch_field.py (season maps)
test/                E2E contract harness + fake-robot NT4 server
```

The WebSocket client identifies itself as `riont` (connect path
`/nt/riont`).

Threading model: the UI thread only renders and handles keys. All socket
IO lives in the client task and reaches the UI through unbounded MPSC
channels (`NtUpdate` downstream, `ClientCommand` upstream), batched at
50 ms. The render loop ticks at ~120 Hz, but ratatui's diffing means only
changed cells hit the terminal.

Per-topic telemetry: publish rate (Hz, 2 s sliding window) and Δ since
last change. The client measures RTT/clock offset internally — used only
for clock-synced publishes, never shown as a headline metric.

## NT4 protocol notes

The client speaks NT4 over WebSocket (subprotocol
`networktables.first.wpi.edu`, path `/nt/<client-name>`): JSON control
messages as text frames, MessagePack value frames
`[pubuid, timestamp_us, type, value]` as binary frames (validated against
ntcore 2026.2.2 source and a real ntcore server). Subscribe-all with fast
periodic, clock sync via `-1` timestamp echo, publish with automatic
retransmission, auto-reconnect with retry counter, 5 s connect timeout.

## Testing

Two tiers, run the fast one first:

```
cargo test                                       # tier 1: in-process, milliseconds
conda run -n nt-tui-test python test/harness.py  # tier 2: end-to-end contract harness
```

**Tier 1 (`cargo test`)** — logic and rendering, fully hermetic:
`src/tests_tui.rs` drives keystrokes through `App::handle_key`, feeds NT
values through `App::apply_values`, renders into a ratatui `TestBackend`,
and asserts on the buffer — publish commands, toasts, tree/palette/picker
behavior, field cards. Tests never touch your `~/.config/riont`.

**Tier 2 (`test/harness.py`)** — the real binary against a real ntcore
server (`test/server.py`). It verifies only cross-process CONTRACTS:
values the fake robot receives after an edit round-trip, HUD state
transitions, watchlist counts, config-file side effects. It synchronizes
by polling for expected state (never fixed sleeps), fails fast on the
first failure with a full-screen dump, and runs in ~15 s. Set
`RIONT_HARNESS_CONTINUE=1` to collect all failures instead of stopping at
the first.

Environment setup for tier 2 (Python 3.11):

```
conda create -n nt-tui-test python=3.11
conda run -n nt-tui-test pip install pyntcore pyte
```

Rule of thumb: if a change only rewords UI copy or adjusts geometry,
`cargo test` is the arbiter — the harness must not need editing for that.

### Regenerating the README screenshots

`scripts/screenshot.py` boots the real binary against the fake robot with
a pre-seeded watchlist and renders the captured screens to
`docs/screenshot-*.png`:

```
conda run -n nt-tui-test python scripts/screenshot.py
```

## Releases

Versions identify releases, not commits:

- Every user-visible change adds a bullet under `## [Unreleased]` in
  `CHANGELOG.md` **in the same commit** as the change. Never bump the
  version yourself.
- Cut a release with `scripts/release.sh <patch|minor|major> "summary"` —
  it rotates `[Unreleased]` into the new version, bumps `Cargo.toml`, and
  tags `vX.Y.Z`.
- `git push --follow-tags` makes CI build Windows/Linux/macOS binaries and
  attach them to the GitHub release. If the release workflow doesn't
  appear within a minute, re-push the tag alone (`git push origin vX.Y.Z`)
  — a combined branch+tag push can drop the tag event.

CI (`.github/workflows/ci.yml`) runs rustfmt + clippy (warnings denied),
`cargo test` on three OSes, and the E2E harness on every push/PR. Run
`cargo fmt` and `cargo clippy --all-targets` before pushing.

## Agent-facing rules

`.agents/AGENTS.md` holds the working rules for coding agents (testing
order, changelog discipline, release procedure) — keep it in sync with
this document when workflows change.
