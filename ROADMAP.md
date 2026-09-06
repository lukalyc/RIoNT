# RIONT Roadmap

## Mission

RIONT fills a specific gap in the FRC NetworkTables tooling landscape:

- **AdvantageScope** owns data analysis — graphs, logging, replay.
- **Elastic** owns the pit-side and driver display.
- **RIONT** owns fast NetworkTables **navigation and topic viewing**:
  find a topic quickly, see its value clearly, right now, without
  configuring anything.

RIONT should do that one job excellently. It should never try to become
the next AdvantageScope or the next Elastic. Features below are filtered
through that identity; anything that drifts toward analysis or
dashboards belongs in another tool.

## Non-goals

RIONT deliberately does not do these. Revisit only with a strong,
season-tested reason:

- Graphs, plots, sparklines of any kind (AdvantageScope's domain)
- Data recording, log export, log replay (AdvantageScope's domain)
- Driver/pit display layouts, match-viewing dashboards (Elastic's
  domain)
- Alerting, monitoring, threshold watches
- Robot control beyond lightweight value writes
- Anything team-specific — RIONT is team-agnostic

## On the roadmap

### Viewing fidelity — the core investment

1. **Struct decoding expansion.** Decode and display
   `ChassisSpeeds`, `Twist2d`, and `SwerveModuleStates` instead of
   `<N bytes>`. The schema-aware decoder already handles `struct:Pose2d`;
   each new type is a small, well-contained addition. Decode to *see*
   the data — visualization is optional and separate.
2. **Raw/struct inspector.** Unknown struct types currently render as
   `<N bytes>`. Show a hex view plus the advertised `structSchema`
   (parsed field names/order) so the data is legible instead of
   opaque. **Must-have** — this is the single clearest gap in topic
   viewing today.

### Viewing — field

3. **Swerve module vectors on the field card.** Draw the four module
   state vectors around the robot triangle when `SwerveModuleStates`
   are available. Pure viewing of existing NT data; valuable to teams
   even if the author never uses it.

### Connection

4. **Connection quality detail in the HUD.** Improve what `COMM`
   communicates about link health beyond connected/reconnecting — exact
   metrics to be decided during design.

### UX / configuration (config-only by policy)

5. **Tree/watchlist split sizing via config.** A `config.json` key for
   the left-column width. Deliberately no UI for it — it must not
   clutter the main interface.
6. **Customizable color themes via config.** Hex values for the UI
   colors (watchlist, borders, HUD, tree) so users can personalize the
   look. Config-file only, same policy as the split sizing.
7. **Colorblind-safe theme.** Far down the line — not urgent.

### Mouse tolerance

8. **Mouse support.** Click-to-focus, scrolling. Keyboard remains the
   primary interaction; this exists for users who want it, not for the
   author.

### Infrastructure

9. **CI + release binaries.** GitHub Actions building Windows/Linux/
   macOS binaries on tag push, so teammates can run RIONT without a
   Rust toolchain.
10. **Wide-viewport harness coverage.** The 97-check suite currently
    tests at a 120x36 viewport only; a real rendering bug already
    escaped that way. Add wide/short layouts to the harness.

## Parked — revisit after a full season of use

These are not committed. Real usage over a season decides whether they
earn a place:

- **Per-card view options** (precision, units, compact/expanded).
  Leans AdvantageScope; if it ever happens it must be a hover +
  keybinding menu on a watchlist card — never persistent UI.
- **Recently-changed view** (topics mutated in the last N seconds).
  Leans monitoring; may prove its worth as navigation with reps.

## Rejected

Recorded so they are not re-proposed every few months:

- **Type-scoped search** (`/double kP`). Collides with Space-to-pin;
  search stays simple.
- **Quick-jump slots.** Overlaps presets; the same "needs season reps"
  question applies, and the answer today is no.
- **Target auto-discovery.** Team-agnostic tool; the Connection Picker
  is sufficient.
