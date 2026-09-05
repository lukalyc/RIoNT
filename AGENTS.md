# AGENTS.md — Working Rules for Coding Agents

Project-specific rules that agents must follow when changing this codebase.
Human contributors should follow them too.

## Versioning procedure (ALWAYS follow when making changes)

**Every change that lands in a commit must carry a version bump in
`Cargo.toml`.** The HUD reads the version at compile time
(`env!("CARGO_PKG_VERSION")` in `src/ui/mod.rs`), so `Cargo.toml` is the
single source of truth — never hardcode a version string anywhere else.

### 1. Pick the bump level (Semantic Versioning: MAJOR.MINOR.PATCH)

| Change | Bump | Examples |
| --- | --- | --- |
| Behavior-affecting bug fix, small polish, test/doc update | **PATCH** (0.3.0 → 0.3.1) | fixing a toast TTL, correcting a hint line |
| New user-facing feature or capability | **MINOR** (0.3.0 → 0.4.0) | a new palette command, a new card type, a new keybinding, new config keys |
| Breaking change to the user contract | **MAJOR** (0.3.0 → 1.0.0) | keymap incompatibility, config format that old versions can't read, removed commands/features |

When in doubt between PATCH and MINOR, choose MINOR.

### 2. Where to bump

- `Cargo.toml` → `version = "x.y.z"` — **required, this is the only place
  the version lives in code.**
- `README.md` — update the headline line (`vX.Y.Z presents …`) and the
  ASCII mockup's HUD line if it shows a version.

### 3. How

- Make the bump **in the same commit** as the change it belongs to. Do not
  leave the bump for a later "housekeeping" commit; every commit should be
  identifiable by the version it ships.
- Multiple changes in one commit: ONE bump for the most significant change.
- Multiple unrelated commits in a session: bump per commit (PATCH is fine
  for each if that's all it warrants).

### 4. Verify

- `cargo build` then check the HUD renders the new version (the harness's
  `T1g version matches Cargo.toml` check enforces this — run
  `python test/harness.py` per README's Testing section before committing).
- `git grep` the old version string afterwards; it should only remain in
  historical/changelog contexts, never in live code or HUD text.

### Rationale

RIONT is a pit tool: when someone reports a screenshot or a bug, the HUD
version is the fastest way to know what binary they run. A stale version
makes every other report ambiguous.
