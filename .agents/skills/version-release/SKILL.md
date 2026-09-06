---
name: version-release
description: Step-by-step release procedure for RIONT — cutting a versioned release (bump, changelog rotation, tag, CI binaries). Use this skill whenever the user mentions releasing, cutting a release, bumping the version, publishing binaries, or tagging; also proactively when a batch of Unreleased changelog work is ready to ship.
---

# Release Procedure

RIONT versions **releases, not commits**. Work lands in `CHANGELOG.md`'s
`## [Unreleased]` section; a release rotates that section into a version,
tags it, and CI attaches binaries. `AGENTS.md` holds the policy; this
skill is the walkthrough. The two must never disagree — if they do,
AGENTS.md wins and this skill needs fixing.

The HUD renders the version from `Cargo.toml` at compile time
(`env!("CARGO_PKG_VERSION")`), so `Cargo.toml` is the single source of
truth: never hardcode a version string in `src/`.

## Step 0 — Preconditions

1. Working tree clean, on `master`, up to date with `origin/master`.
2. `## [Unreleased]` in `CHANGELOG.md` is non-empty (a release with no
   content is a mistake — the script rejects it).
3. CI is green on the latest commit (`.github/workflows/ci.yml`).

## Step 1 — Pick the bump level (or let the script do it)

Read the Unreleased bullets and classify:

| Unreleased content | Bump | Examples |
| --- | --- | --- |
| Bug fixes, polish, docs, tests only | **PATCH** (0.5.4 → 0.5.5) | toast TTL fix, README cleanup |
| New user-facing feature or capability | **MINOR** (0.5.0 → 0.6.0) | new palette command, new card type, new keybinding, new config keys, `RIONT_CONFIG` |
| Breaking user-contract change | **MAJOR** (0.5.0 → 1.0.0) | keymap incompatibility, config format old versions can't read, removed commands |

When torn between PATCH and MINOR, choose MINOR.

## Step 2 — Run the release script

```
scripts/release.sh <patch|minor|major|x.y.z> "one-line summary"
# e.g.
scripts/release.sh minor "test pyramid, RIONT_CONFIG, CI release binaries"
```

The script (idempotent, safe to inspect before pushing):

1. Rotates `## [Unreleased]` → `## [X.Y.Z] - <today>` and inserts a fresh
   empty `## [Unreleased]`.
2. Bumps `Cargo.toml`, refreshes `Cargo.lock`.
3. Updates the README headline/ASCII-mockup HUD version string.
4. Runs `cargo build` (refreshes the lockfile, proves the tree compiles).
5. Commits as `Version X.Y.Z: <summary>` and tags `vX.Y.Z`.

## Step 3 — Push and let CI build the binaries

```
git push --follow-tags
```

`.github/workflows/release.yml` fires on `v*` tags: it builds Windows,
Linux and macOS release binaries, packages each with README + CHANGELOG,
and attaches them to the GitHub Release for the tag. Teammates download
from the release page — no Rust toolchain required.

## Step 4 — Verify

1. `cargo test` — the `hud_online_shows_comm_code_uptime_and_cargo_version`
   test proves the HUD shows the new version (dynamic compare, nothing to
   hand-edit).
2. `conda run -n nt-tui-test python test/harness.py` — all contract
   checks pass; the HUD-version check reads Cargo.toml dynamically.
3. `git grep` the OLD version string: it may remain only in
   `CHANGELOG.md` history — never in live code or HUD text.
4. After the push: the GitHub release page shows the binaries and the
   CHANGELOG body.

## Common mistakes seen in this repo

- Bumping the version in a feature commit — versions move ONLY via
  `scripts/release.sh`. Feature commits add `[Unreleased]` bullets.
- Cutting a release with an empty Unreleased section.
- Editing the version in README by hand without running the script —
  headline and ASCII-mockup lines are two separate strings and the script
  keeps them in sync.
- Forgetting `--follow-tags`: the tag never reaches origin, no binaries
  are built, the release page stays empty.
