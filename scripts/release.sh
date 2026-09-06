#!/usr/bin/env bash
# RIONT release script — cut a versioned release from [Unreleased].
#
# Usage: scripts/release.sh <patch|minor|major|x.y.z> "one-line summary"
#
# What it does:
#   1. Verifies preconditions: clean tree, on master, up to date, and a
#      non-empty [Unreleased] section in CHANGELOG.md.
#   2. Rotates "## [Unreleased]" -> "## [X.Y.Z] - <today>" (plus a fresh
#      empty Unreleased section) and bumps Cargo.toml.
#   3. Updates the README headline + ASCII-mockup HUD version strings.
#   4. cargo build (refreshes Cargo.lock, proves the tree compiles) and
#      cargo test.
#   5. Commits "Version X.Y.Z: <summary>" and tags vX.Y.Z.
#
# Nothing is pushed. Finish with: git push --follow-tags
# (release.yml then builds and attaches the release binaries).

set -euo pipefail

cd "$(dirname "$0")/.."

BUMP="${1:-}"
SUMMARY="${2:-}"

if [[ -z "$BUMP" || -z "$SUMMARY" ]]; then
  echo "usage: scripts/release.sh <patch|minor|major|x.y.z> \"one-line summary\"" >&2
  exit 2
fi

# --- preconditions ---------------------------------------------------------

[[ -z "$(git status --porcelain)" ]] || { echo "ERROR: working tree is not clean — commit or stash first." >&2; exit 1; }
[[ "$(git branch --show-current)" == "master" ]] || { echo "ERROR: releases are cut from master (you are on: $(git branch --show-current))." >&2; exit 1; }
git fetch origin --quiet
# origin must be contained in local master: behind is an error (rebase

# first); local commits ahead are fine — the release commit joins them
# in the same push.
git merge-base --is-ancestor origin/master HEAD || { echo "ERROR: master is behind origin/master — pull --rebase first." >&2; exit 1; }

grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "ERROR: CHANGELOG.md has no ## [Unreleased] section." >&2; exit 1; }
# Non-empty Unreleased: content between the Unreleased heading and the next
# version heading, excluding blank lines and the Added/Changed/... headers.
UNRELEASED_BODY=$(awk '/^## \[Unreleased\]/{f=1;next} /^## /{f=0} f' CHANGELOG.md | grep -Ev '^\s*$|^### ' || true)
[[ -n "$UNRELEASED_BODY" ]] || { echo "ERROR: [Unreleased] is empty — nothing to release. Land work with changelog bullets first." >&2; exit 1; }

OLD=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[[ "$OLD" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "ERROR: cannot parse current version from Cargo.toml ($OLD)." >&2; exit 1; }

# --- compute the new version ----------------------------------------------

IFS='.' read -r MA MI PA <<< "$OLD"
case "$BUMP" in
  patch) NEW="$MA.$MI.$((PA + 1))" ;;
  minor) NEW="$MA.$((MI + 1)).0" ;;
  major) NEW="$((MA + 1)).0.0" ;;
  *) NEW="$BUMP" ;;
esac
[[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "ERROR: bad version '$NEW'." >&2; exit 1; }
if git rev-parse "v$NEW" >/dev/null 2>&1 || git ls-remote --tags origin "v$NEW" | grep -q .; then
  echo "ERROR: tag v$NEW already exists." >&2
  exit 1
fi

TODAY=$(date +%F)

echo "release: $OLD -> $NEW  ($SUMMARY)"
awk '/^## \[Unreleased\]/{f=1} /^## / && !f{exit} f' CHANGELOG.md | grep -Ev '^\s*$' \
  | sed 's/^/    | /'

read -r -p "Cut release v$NEW? [y/N] " CONFIRM
[[ "$CONFIRM" =~ ^[Yy]$ ]] || { echo "aborted."; exit 1; }

# --- rotate the changelog --------------------------------------------------

python3 - "$NEW" "$TODAY" <<'PYEOF'
import re, sys
new, today = sys.argv[1], sys.argv[2]
s = open("CHANGELOG.md").read()
rotated = f"## [{new}] - {today}"
s = s.replace("## [Unreleased]", f"## [Unreleased]\n\n_Nothing yet — see the latest release notes above._\n\n{rotated}", 1)
# Drop the placeholder if the fresh Unreleased stays empty (it does right
# after rotation): keep the section header but no filler text.
s = s.replace(f"## [Unreleased]\n\n_Nothing yet — see the latest release notes above._\n\n{rotated}",
              f"## [Unreleased]\n\n{rotated}")
open("CHANGELOG.md", "w").write(s)
PYEOF

# --- bump Cargo.toml -------------------------------------------------------

sed -i "0,/^version = \"$OLD\"/s//version = \"$NEW\"/" Cargo.toml

# --- README version strings (headline/mockup use the RELEASED version) ----

sed -i "s/RIONT v$OLD/RIONT v$NEW/g" README.md

# --- build, test -----------------------------------------------------------

echo "--- cargo build (refresh Cargo.lock) ---"
cargo build --quiet
echo "--- cargo test ---"
cargo test --quiet 2>&1 | tail -2

# --- commit + tag ----------------------------------------------------------

git add Cargo.toml Cargo.lock CHANGELOG.md README.md
git commit -m "Version $NEW: $SUMMARY"
git tag -a "v$NEW" -m "RIONT $NEW — $SUMMARY"

echo ""
echo "release v$NEW cut."
echo "next:  git push --follow-tags   (CI then builds the release binaries)"
