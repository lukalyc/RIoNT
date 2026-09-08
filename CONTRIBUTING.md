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

RIONT is a Cargo workspace: the product binary plus reusable engine crates
(also consumed by other applications via git-tag dependencies — see
"Reusing the engine" below).

```
crates/riont-nt4/    Async NT4 client task: reconnect-forever sessions, clock sync,
                     publish retransmission. NtUpdate / ClientCommand channels.
                     Product-agnostic — no app logic, no SSH.
crates/riont-store/  Topic store: values, metadata, Hz windowing, pose trails;
                     conservative pose classification + struct:Pose2d decoding.
crates/riont-field/  Field geometry: built-in + PathPlanner JSON maps, wall
                     classification, braille-card math.
src/main.rs          CLI (team/IP resolve), terminal setup, editor suspend/resume
src/app.rs           App state, key handling, palette/toasts. Pure logic, no rendering.
src/config.rs        ~/.config/riont/config.json: targets, presets, SSH settings.
src/ops.rs           Product-side background ops (SSH robot-code restart).
src/ui/mod.rs        Layout: HUD, tree, inspector dock, watchlist card matrix, overlays.
src/ui/tree.rs       Collapsible topic tree model (rebuilt per frame).
scripts/             release.sh, screenshot.py, fetch_field.py (season maps)
test/                E2E contract harness + fake-robot NT4 server
```

The WebSocket client identifies itself as `riont` (connect path
`/nt/riont`).

Threading model: the UI thread only renders and handles keys. All socket
IO lives in the `riont-nt4` client task and reaches the UI through
unbounded MPSC channels (`NtUpdate` downstream, `ClientCommand` upstream),
batched at 50 ms. The render loop ticks at ~120 Hz, but ratatui's diffing
means only changed cells hit the terminal.

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

`test/probe_echo.py` and `test/probe_echo2.py` are standalone
diagnostics (not part of the harness) that establish the publish
verification premises: an ntcore server never echoes a client's own
publish back to it (probe_echo.py), and a read-back requires a second,
separate client connection (probe_echo2.py). They additionally need
`websocket-client` and `msgpack` in the same environment
(`pip install websocket-client msgpack`).

Rule of thumb: if a change only rewords UI copy or adjusts geometry,
`cargo test` is the arbiter — the harness must not need editing for that.

### Regenerating the README screenshots

`scripts/screenshot.py` boots the real binary against the fake robot with
a pre-seeded watchlist and renders the captured screens to
`docs/screenshot-*.png`:

```
conda run -n nt-tui-test python scripts/screenshot.py
```

## Reusing the engine

The engine crates are MIT-licensed and product-agnostic. An application in
another repository (e.g. a team-private tool) can depend on them by git
tag — the public repo needs no auth:

```toml
[dependencies]
riont-nt4    = { git = "https://github.com/lukalyc/riont.git", tag = "v0.6.0" }
riont-store  = { git = "https://github.com/lukalyc/riont.git", tag = "v0.6.0" }
riont-field  = { git = "https://github.com/lukalyc/riont.git", tag = "v0.6.0" }
```

Pin to release tags (never track `master`), and bump deliberately — the
engine's semver signals breaking changes. Crates stay in lockstep with the
repo version; `scripts/release.sh` bumps them together.

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

**Zero-warning rule: RIONT must not be committed if the build has
warnings.** A `cargo build` that emits warnings (unused variables,
unreachable patterns, dead code, …) is not done — fix them or `#[allow]`
with a comment explaining why, and verify with `cargo build 2>&1 | grep
warning` before committing. CI enforces the same bar for clippy.

## Agent-facing rules

`.agents/AGENTS.md` holds the working rules for coding agents (testing
order, changelog discipline, release procedure) — keep it in sync with
this document when workflows change.
