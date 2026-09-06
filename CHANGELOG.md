# Changelog

All notable changes to RIONT are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). See `AGENTS.md` for the
release procedure: work lands in `[Unreleased]`; a version is cut from
it by `scripts/release.sh` when the batch is ready to ship.

## [Unreleased]

## [0.6.0] - 2026-09-06

### Added

- **CI**: GitHub Actions on every push/PR — rustfmt + clippy (warnings
  are errors), `cargo test` on Windows/Linux/macOS, and the end-to-end
  contract harness on Linux (`.github/workflows/ci.yml`).
- **Release binaries**: pushing a `v*` tag builds Windows/Linux/macOS
  packages (Intel + Apple Silicon) and attaches them to the GitHub
  release, so teammates don't need a Rust toolchain
  (`.github/workflows/release.yml`) — ROADMAP item 9.
- **`scripts/release.sh`**: cuts a release from the changelog's
  `## [Unreleased]` section (rotate, bump, tag) — one command.
- `RIONT_CONFIG` environment variable: run RIONT against an alternate
  `config.json` path — hermetic test runs and portable installs. The
  end-to-end harness uses it so tests never touch your real config.

### Changed

- Test pyramid overhaul (for agents and humans):
  - 42 new in-process Rust tests (`cargo test`, src/tests_tui.rs) render
    the real UI into a TestBackend and assert on keystrokes → state →
    buffer in milliseconds — no subprocess, no network, no sleeps.
  - `test/harness.py` rewritten as a contract harness: every check now
    synchronizes by polling for the expected state (no fixed sleeps),
    asserts on cross-process contracts (server-received values, HUD state
    transitions, config side effects) instead of exact UI wording, and
    fails fast with a full-screen dump on the first failure. A full run
    dropped from minutes to ~11 s and no longer breaks when UI copy is
    reworded.

### Changed

- **Versioning workflow**: versions identify releases, not commits.
  Per-commit version bumps are replaced by `[Unreleased]` changelog
  bullets; the version moves exactly once per release, via
  `scripts/release.sh`. Codebase is now rustfmt-clean and clippy-clean
  (warnings denied in CI).

## [0.5.5] - 2026-09-06

### Added

- `ROADMAP.md` — mission, non-goals, and prioritized features. Records
  what RIONT is (fast NT navigation and topic viewing) and what it
  deliberately is not, so scope creep is prevented rather than debated.

## [0.5.4] - 2026-09-05

### Added

- `version-release` agent skill (`.agents/skills/version-release/`) — a
  step-by-step walkthrough of the versioning procedure; AGENTS.md remains
  the policy source.

## [0.5.3] - 2026-09-05

### Changed

- README rewrite: the layout is described in plain terms (no panel
  percentage arithmetic); the ASCII mockup matches the real UI.

## [0.5.2] - 2026-09-05

### Added

- `CHANGELOG.md` — release history at a glance, maintained per the
  versioning procedure in `AGENTS.md`.

## [0.5.1] - 2026-09-05

### Fixed

- HUD disconnect reasons are humanized: raw OS error strings ("…d it.
  (os error 10061)") are mapped to short labels (connection refused,
  timed out, host unreachable, …). The full error still reaches toasts;
  the HUD renders the state keyword plus a quiet dim attempt/cause line.

## [0.5.0] - 2026-09-05

### Added

- Overlay groups: several pose topics can share ONE composite field card
  (`o` on a hovered field card toggles membership, persisted in config).
  Per-member marker colors + legend, per-member trails, and a full
  member legend with live x/y/theta in the enlarged view.
- Composite cards collapse into a single Tab stop; `x` unpins the whole
  group with the previous view stashed in the `u` undo slot.

## [0.4.1] - 2026-09-05

### Fixed

- Sticky pose cards: pose sources that publish empty arrays between
  estimates (Limelight with no target) no longer flicker the field card
  on/off. Empty estimates render the field without the robot marker;
  the enlarged view shows "no pose estimate".

## [0.4.0] - 2026-09-05

### Added

- A lone field card expands to the entire watchlist canvas (shrinks
  back when other topics are pinned).
- Enlarged field view: `f` on a hovered field card opens a
  near-fullscreen field popup with a live x/y/theta readout; the same
  key closes it.

## [0.3.2] - 2026-09-05

### Fixed

- Field card canvas is centered on the wall union bbox — the perimeter
  was invisible because wall rects overshoot the nominal field extents
  and Canvas drops segments with out-of-bounds endpoints.

## [0.3.1] - 2026-09-05

### Fixed

- Alliance half tinting (blue/red) on field cards; removed redundant
  side text labels. Robot color follows `FMSInfo/IsRedAlliance`
  (neutral cyan when absent).

## [0.3.0] - 2026-09-05

### Added

- Field visualization: pose topics render as braille field cards (walls,
  robot triangle, trail). Conservative auto-detection (exact Limelight
  pose names, exact `struct:Pose2d`); manual force-pose opt-in for
  ambiguous topics; `struct:Pose2d` decoding end to end.
- Programmatic field maps: `scripts/fetch_field.py` converts Choreo's
  official vector field SVGs to RIONT field JSON (2026 Rebuilt shipped);
  field maps load from `fields/<map>.json` or `config.json walls_file`.
- Watchlist persistence (`last_view`) and one-level undo (`u`) for
  preset loads / Clear All.
- HUD shows disconnect reason + retry attempt count.
- Config reload on `$EDITOR` exit; corrupt-config backup guard.

### Changed

- Input: `j`/`k` are typeable in Search/Palette/Connect; Ctrl-C cancels
  text entry like Esc; tree Enter edits topic rows.
- Honest feedback: offline publishes warn "queued" instead of claiming
  "published"; glob unpins and preset loads report true card counts.
- Severity-scaled toast TTLs; search matches stored by name (no stale
  indices); inspector dock mirrors the effective active topic.

### Fixed

- Removed bare-digit quick-connect in the Connection Picker (it
  hijacked the first keystroke of typed IPs).

## [0.2.0] - 2026-09-03

### Added

- Two-zone dashboard layout: Topic Tree + Inspector Dock (35%) and
  Watchlist Canvas (65%) with height-first card packing under a
  Driver-Station style HUD.
- Fuzzy search (`/`), command palette (`:`), connection picker (`c`),
  workspace presets (`1-9`), inline value editing (`e`).
- NT4 client: subscribe-all, clock-synced publishes, auto-reconnect,
  SSH robot-code restart.
