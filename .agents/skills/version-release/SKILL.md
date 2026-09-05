---
name: version-release
description: Step-by-step release procedure for RIONT — bump the version, update the changelog, sync the README, and verify. Use this skill whenever a change is about to be committed or committed work needs a version bump, whenever the user mentions releasing, bumping the version, updating the changelog, or cutting a release, and also proactively after finishing ANY code or doc change in this repo, even if the user didn't ask for version bookkeeping.
---

# Version Release Procedure

Every change-carrying commit in RIONT ships a version bump, a changelog
entry, and (when relevant) README sync — all in the **same commit**. The
HUD renders the version from `Cargo.toml` at compile time
(`env!("CARGO_PKG_VERSION")`), so `Cargo.toml` is the single source of
truth: never hardcode a version string in `src/`.

Why this matters: RIONT is a pit tool. When someone reports a bug with a
screenshot, the HUD version is the fastest way to know which binary they
run. A stale version makes every report ambiguous. The changelog is the
same promise in text form.

`AGENTS.md` holds the policy; this skill is the walkthrough. The two
must never disagree — if they do, AGENTS.md wins and this skill needs
fixing.

## Step 1 — Pick the bump level

Read the diff (or recall what changed) and classify it:

| Change | Bump | Examples from this repo |
| --- | --- | --- |
| Behavior fix, small polish, docs/tests only | **PATCH** (0.5.1 → 0.5.2) | toast TTL fix, README cleanup |
| New user-facing feature or capability | **MINOR** (0.5.0 → 0.6.0) | new keybinding, new card type, new palette command, new config keys |
| Breaking user-contract change | **MAJOR** (0.5.0 → 1.0.0) | keymap incompatibility, config format old versions can't read, removed commands |

When torn between PATCH and MINOR, choose MINOR.

## Step 2 — Bump `Cargo.toml`

Change `version = "x.y.z"` under `[package]`. Nothing else in `src/`
needs touching — the HUD picks it up at compile time.

## Step 3 — Add the changelog entry

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com):
newest version at the TOP, dated today, with `### Added` /
`### Changed` / `### Fixed` / `### Removed` subsections as applicable.

Write for a pit user, not for reviewers:

- Good: "Overlay groups let you compare odometry and a vision estimate
  on one field."
- Bad: "Refactored paint_field_canvas to accept a member list."

The entry must cover everything in the commit, not just the headline.

## Step 4 — Sync the README (only if the change affects it)

Skip cleanly if the change is invisible to users. Otherwise update:

1. **Headline line** (top of file): `vX.Y.Z presents …` — always bump
   the number here.
2. **ASCII mockup HUD line**: `RIONT vX.Y.Z  [COMM: …` — bump the number.
3. **Feature sections**: if the change altered documented behavior —
   keymap table, Field Visualization bullets, Connection Picker prose,
   config examples, Editing/presets — update those sections to match
   reality. A README that documents yesterday's keymap is worse than
   none.

## Step 5 — Sync the harness expectation

`test/harness.py` has one hardcoded version string, in the check named
`T1g version matches Cargo.toml` (near the top of `main()`). Update it
to the new version. This check is the safety net that catches a
forgotten bump — do not weaken or remove it.

## Step 6 — Verify

1. Kill any leftover dashboard (a running `riont.exe` locks the build
   output with a confusing "Access is denied (os error 5)"):
   `taskkill //F //IM riont.exe`
2. `cargo build` — must be clean.
3. `cargo test` — must pass.
4. Run the end-to-end suite: `python test/harness.py` (see README's
   Testing section for the conda env). 97+ checks must pass, including
   T1g.
5. `git grep` the OLD version string. It may remain in `CHANGELOG.md`
   history only — never in live code, README headline, or HUD text.

## Step 7 — Commit

One commit containing: the change itself, the `Cargo.toml` bump, the
changelog entry, the README sync (if any), and the harness expectation.
Commit message convention in this repo names the version, e.g.
`Version 0.5.1: humanized HUD disconnect reasons`.

## Common mistakes seen in this repo

- Rebuilding without killing `riont.exe` first, then misreading the
  lock error as a code problem.
- Running the harness against a stale binary after editing `Cargo.toml`
  — always `cargo build` before `python test/harness.py`.
- Updating the README headline but forgetting the ASCII mockup HUD
  line (they are separate strings).
- Shipping a feature as "part of 0.x" without a bump because "it's
  just one commit" — every commit is a release candidate; version it.
