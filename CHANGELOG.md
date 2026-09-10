# Changelog

All notable changes to RIONT are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). See `AGENTS.md` for the
release procedure: work lands in `[Unreleased]`; a version is cut from
it by `scripts/release.sh` when the batch is ready to ship.

## [Unreleased]

### Added

- **Swerve module vectors on the field card.** When the robot publishes
  a `struct:SwerveModuleStates` topic, field cards draw each module's
  vector from the footprint corner: direction = steer angle relative to
  the robot heading, length proportional to wheel speed. Drawn when
  exactly one such topic exists — zero or several, and nothing is drawn
  (RIONT never guesses which to trust). Skipped on cards too small to
  read.
- **Struct decoding expansion.** `struct:ChassisSpeeds` (vx, vy, omega),
  `struct:Twist2d` (dx, dy, dtheta) and `struct:SwerveModuleStates`
  (per-module angle + speed) now render as named fields instead of
  `<N bytes>`, alongside the existing `struct:Pose2d` decoder. Malformed
  payloads degrade to the raw-bytes display.
- **Undecoded struct topics are readable in the inspector dock.** Topics
  typed `struct:…` that RIONT cannot decode (a custom WPILib struct, not
  Pose2d) used to show only `<N bytes>`. The dock now also lists the
  robot's advertised `structSchema` flattened to its leaf fields (name
  + type, in declaration order) and a hex view of the raw bytes — 8 per
  row with byte offsets, first 64 bytes — so a payload can be checked
  against the robot's struct definition on the bench. A malformed or
  missing schema falls back to the raw string; decoded structs keep
  their plain value display.

- **Watchlist groups folder pins into their own columns.** Pinning two
  folders (e.g. `LeftShooterHead` and `RightShooterHead`) no longer
  jumbles their cards together: each folder's cards live in their own
  column under a muted `─ Folder (n) ─` header, where n is the live
  topic count (grows as the robot publishes new topics under the
  folder). A folder bigger than one column continues in the next with
  its header repeated. Columns beyond the pane width — long loose-pin
  lists too — scroll horizontally with h/l (the old hard 3-column cap
  with its clipped tail is gone). Explicitly pinned single topics keep
  the fill-to-brim packing (no header).

### Fixed

- **Red-alliance field view mirrors the robot heading.** The mirrored
  view flipped the robot's POSITION but not its heading, so red-view
  operators saw the robot facing the wrong way. Red view at heading θ
  now renders exactly like blue view at −θ.

- **Crashes leave evidence behind instead of vanishing.** When RIONT
  panicked on a double-clicked Windows launch, the console closed with
  the panic message and the crash was undiagnosable. Panics are now
  appended to `riont-debug.log` (next to the session logs the NT4
  engine already writes) with the message, source location and a
  backtrace, then still printed to stderr as before.
- **Watchlist folder columns no longer overflow with long array
  values.** A folder whose cards hold long arrays (values that wrap at
  narrow widths) had its cards height-measured at a wider width than
  the columns actually render at, so more cards were packed into a
  column than fit and the bottom cards were cut off. Card heights are
  now measured at the width the columns really render at (the packing
  iterates until the column count and the measurement agree), so every
  card in a column fits.
- **The watchlist scrolls vertically to follow the cursor.** With more
  pinned cards than fit in three columns, the overflow was appended to
  the last column and silently clipped while j/k kept moving the cursor
  down into the invisible region — pinned cards were never visible
  again. The cursor's column now scrolls so the active card is always
  on screen (the vertical counterpart of the existing column scroll);
  entering a different column with h/l restarts from its top.

### Changed

- **The robot on field cards is now drawn as a rectangle with a center
  orientation arrow** (was: a triangle). The arrow stays inside the
  footprint — center to the front-edge midpoint with a small folding
  head. The footprint size is configurable — `field.robot_length_m` /
  `field.robot_width_m` in `~/.config/riont/config.json` (defaults
  0.9 × 0.9 m; include bumpers if you want the true footprint). Small
  cards draw an adaptive compact glyph: the full rectangle+arrow needs
  ~8 braille dots of footprint to read; below that RIONT steps down to
  rect+tick, then to a chevron, always keeping position and heading
  accurate.

### Fixed

- **CODE no longer reads STOPPED while the robot code is running.** The
  old heuristic called code "stopped" whenever no topic value changed
  for 500 ms — but NT4 pushes only CHANGED values, so a running robot
  with static telemetry (parked arm, idle on the bench) was reported
  STOPPED against a green Driver Station. CODE now also treats the
  robot's ntcore server answering RIONT's 1-second RTT ping as proof
  the robot program is alive; STOPPED now requires both quiet frames
  AND a dead ping. Field-verified failure: connected over USB tether,
  Driver Station green, RIONT insisting CODE: STOPPED for the whole
  session.
- **The last-connected target is persisted, not the launch target.**
  Every successful connect wrote the CLI launch target to config.json,
  so after using the connection picker the next launch silently went
  back to a stale address (observed: RIONT re-trying a closed
  simulation after the operator had moved to the robot over USB).
- **The HUD names the target it is retrying while offline.**
  DISCONNECTED / RECONNECTING now show the address, so a stale target
  is visible at a glance instead of reading like a ghost connection.
- **RUNTIME counter no longer freezes after a reconnect.** The old
  uptime (derived from a monotonic maximum of robot timestamps) could
  never recover once a reconnecting robot's clock restarted below it.
  The HUD now measures RUNTIME locally — how long the current connection
  has been up. It freezes on disconnect and restarts from zero on
  reconnect, including after a robot-code restart.
- **Edits now update the displayed value.** The NT4 server never sends a
  client's own publish back to it, so the tree and inspector kept showing
  the pre-edit value even though the write landed on the robot. RIONT now
  applies the edit locally (local echo) the moment it publishes.
- **Edits are verified by read-back.** After every publish RIONT opens a
  short-lived second NT4 connection and reads the topic back — the
  robot's ntcore instance IS the server, so a matching read-back proves
  the robot accepted the write. A confirmed edit stays silent; if the
  robot reads back a different value or nothing at all within 3 s, a
  `[WARN]` toast says the edit was not confirmed. A disconnect during
  the window reports nothing.

### Changed

- Zero-warning policy documented: RIONT must not be committed if the
  build emits warnings (see CONTRIBUTING.md / AGENTS.md). Two latent
  warnings in the nt4 engine fixed.

## [0.7.4] - 2026-09-07

### Fixed

- **NT4 engine: publish-during-connect livelock.** A write arriving
  during the connect handshake aborted it and was re-queued — and the
  re-queued write interrupted the next handshake too, an infinite
  reconnect churn in which no write ever landed. Connect-phase writes
  are now buffered and flushed once the session is up.
- **Pubuids no longer reset per session.** The server keys publishers
  by (client name, pubuid): a reconnecting client re-publishing
  pubuid 1 was ignored as a duplicate.
- The debug log appends across sessions and records each session's end
  reason (reconnect churn was invisible before).

## [0.7.3] - 2026-09-07

### Fixed

- **NT4 engine: publishes are now robust against connect-time races.**
  Includes: the debug log now appends across sessions (reconnects no
  longer wipe the log) and the session end reason is recorded.
  A lost publish *declare* was never re-sent, so every value frame from
  an undeclared publisher was dropped by the server regardless of
  timestamp. Retransmissions now re-declare the publisher alongside
  each re-encoded value frame.

## [0.7.2] - 2026-09-07

### Fixed

- **NT4 engine: publishes are now robust against connect-time races.**
  Includes: the debug log now appends across sessions (reconnects no
  longer wipe the log) and the session end reason is recorded.
  Three compounding drop causes fixed: publishes before clock sync
  carried timestamp 0 (servers drop those); retransmissions resent the
  same frozen frame forever; and a lost publish *declare* was never
  re-sent, so every value frame from an undeclared publisher was
  dropped. Retransmissions now re-declare the publisher and re-encode
  the value with the current timestamp — writes land within ~300 ms of
  connect regardless of timing.

## [0.7.1] - 2026-09-07

### Fixed

- **NT4 engine: writes within the first second after connecting were
  silently dropped.** A publish before clock sync carries timestamp 0,
  which ntcore servers ignore — the client then reported "no
  round-trip; robot did not confirm". The engine now requests clock
  sync immediately at connect, so every publish carries a real
  timestamp. — the client then reported "no
  round-trip; robot did not confirm". The engine now requests clock
  sync immediately at connect, so every publish carries a real
  timestamp.

## [0.7.0] - 2026-09-06

### Added

- **Engine crates**: RIONT restructured as a Cargo workspace —
  `riont-nt4` (NT4 protocol client), `riont-store` (topic store + pose
  classification) and `riont-field` (field maps + geometry) are now
  reusable, product-agnostic crates. The SSH robot-code restart moved out
  of the client into the app (`src/ops.rs`). Other projects can depend on
  the engine by git tag (see CONTRIBUTING.md).
- MIT `LICENSE` (matching the licensing of other common FRC tools).
- Screenshots in the README (connected session, enlarged field view,
  command palette), with `scripts/screenshot.py` to regenerate them;
  `CONTRIBUTING.md` now holds the build/architecture/protocol/testing
  details.

### Changed

- **2026-rebuilt is the default field map** (was 2025-reefscape). Cycle
  order is unchanged: 2024 → 2025 → 2026 → wrap.
- README restructured: features + keymap up front, screenshots as the
  visual record, concise usage; build/testing/architecture details moved
  to CONTRIBUTING.md.

### Fixed

- A minimal `config.json` (e.g. the README example) no longer fails to
  parse: `field.length_m` / `width_m` now fall back to the season
  defaults instead of invalidating the whole config.

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
