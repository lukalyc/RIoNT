# AGENTS.md — Working Rules for Coding Agents

Project-specific rules that agents must follow when changing this codebase.
Human contributors should follow them too.

## Release procedure (CHANGELOG discipline per commit; version at release)

A step-by-step walkthrough of cutting a release is available as an agent
skill: `.agents/skills/version-release/SKILL.md`. Keep the two in sync
(AGENTS.md wins on any disagreement).

The product scope lives in `ROADMAP.md` — read its Mission and
Non-goals sections before proposing or implementing any feature. If a
feature idea is listed under Non-goals or Rejected, do not build it.

**Commits carry changelog entries, not version bumps.** The version is
bumped exactly once per release, by `scripts/release.sh` — never by hand
in a feature commit, and never by an agent.

The HUD reads the version at compile time (`env!("CARGO_PKG_VERSION")`
in `src/ui/mod.rs`), so `Cargo.toml` is the single source of truth —
never hardcode a version string anywhere else.

### 1. Every commit (during work)

- Do NOT touch `Cargo.toml`'s version.
- If the commit changes anything user-visible (behavior, UI, config
  keys, docs), add a bullet under `## [Unreleased]` at the TOP of
  `CHANGELOG.md`, in the same commit. Use the existing Added / Changed /
  Fixed / Removed subsections. Write it for a pit user, not for
  reviewers ("overlay groups let you compare odometry and vision on one
  field", not "refactored paint_field_canvas").
- Test/docs/CI-only commits with no user impact may skip the bullet —
  but when in doubt, add one.
- README: update feature sections if the change altered documented
  behavior (keymap, config examples, prose). The headline/mockup version
  string stays at the last released version until a release is cut.

### 2. Cutting a release (deliberate, batches many commits)

Run `scripts/release.sh <patch|minor|major|x.y.z> "summary line"`. It:

1. Verifies the tree is clean, on `master`, and up to date, and that
   `## [Unreleased]` is non-empty (an empty release is rejected).
2. Picks the bump level from the Unreleased content (or the level you
   pass): bug fixes → PATCH, new user-facing features → MINOR, breaking
   user-contract changes → MAJOR. When in doubt between PATCH and
   MINOR, choose MINOR.
3. Rotates `## [Unreleased]` → `## [X.Y.Z] - <today>`, bumps `Cargo.toml`,
   refreshes `Cargo.lock`, updates the README headline/mockup version.
4. Commits as `Version X.Y.Z: <summary>`, tags `vX.Y.Z`.

Then `git push --follow-tags` — CI builds and attaches Windows/Linux/
macOS release binaries to the GitHub release (`.github/workflows/
release.yml`). The tagged commit is the binary people run; the HUD
version on any screenshot therefore identifies its exact release.

### 3. Verify (every commit, release, and CI run)

Run BOTH test tiers (see "Testing workflow" below):

- `cargo test` — in-process tests must pass; the
  `hud_online_shows_comm_code_uptime_and_cargo_version` test enforces
  that the HUD matches `Cargo.toml` (read dynamically — no hardcoded
  string to update anywhere).
- `cargo build`, then `python test/harness.py` — the end-to-end contract
  harness (auto-builds if the binary is stale; fail-fast with a full-screen
  dump on the first failure).

## Testing workflow for agents (follow this order)

The test pyramid has two tiers. Choosing the right one is the difference
between seconds and minutes of feedback:

1. **`cargo test` FIRST, always.** Logic, state and rendering are covered
   in-process (unit tests in `pose.rs`/`store.rs`/`field.rs`/`config.rs`,
   full-TUI tests in `src/tests_tui.rs` using a TestBackend). Milliseconds,
   deterministic, hermetic. Most bugs are found and fixed here.
2. **`python test/harness.py` for the integration contract.** Only the
   real binary against a real ntcore server proves the socket, the wire
   protocol, reconnects and persistence. The harness:
   - synchronizes by polling (never fixed sleeps), so it does not flake
     on slow machines — do not "fix" flakiness by adding sleeps;
   - asserts only cross-process contracts (server-received values, HUD
     transitions, config side effects), never exact UI wording, colors or
     geometry — a wording change must NEVER require harness edits;
   - fails FAST at the first failed check with a full-screen dump — fix
     that one check's cause, do not mass-adjust assertions;
   - auto-builds the debug binary if missing.
3. **Never weaken a check to make a run green.** If the harness fails,
   either the code regressed or the CONTRACT changed. Both need a
   deliberate edit with a reason — and an `[Unreleased]` changelog bullet
   if user-visible. UI-copy/geometry assertions belong in `cargo test`
   (TestBackend), not in the harness.
4. Env setup: `conda create -n nt-tui-test python=3.11 &&
   /home/<you>/anaconda3/envs/nt-tui-test/bin/pip install pyntcore pyte`,
   then `conda run -n nt-tui-test python test/harness.py`.

### Rationale

RIONT is a pit tool: when someone reports a screenshot or a bug, the HUD
version is the fastest way to know what binary they run. Versions
therefore identify *releases* (tagged, CI-built binaries), not commits —
so every pushed tag must produce runnable artifacts, and the changelog
must read as release history. Per-commit `[Unreleased]` bullets keep the
traceability without version spam.
