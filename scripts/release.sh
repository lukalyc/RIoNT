#!/usr/bin/env bash
# RIONT release script — cut a versioned release from [Unreleased].
#
# Usage:
#   scripts/release.sh <patch|minor|major|x.y.z> "one-line summary" [flags]
#
# Flags:
#   -y, --yes      cut the release WITHOUT the confirmation prompt
#                  (non-interactive — for scripts and coding agents)
#   --dry-run      verify preconditions and preview the release notes,
#                  then exit without changing anything
#
# What it does:
#   1. Verifies preconditions: clean tree, on master, up to date, and a
#      non-empty [Unreleased] section in CHANGELOG.md.
#   2. Rotates "## [Unreleased]" -> "## [X.Y.Z] - <today>" (plus a fresh
#      empty Unreleased section) and bumps Cargo.toml.
#   3. Updates the README headline + ASCII-mockup HUD version strings (if present).
#   4. cargo build --workspace (refreshes Cargo.lock, checks zero warnings)
#      and cargo test --workspace.
#   5. Commits "Version X.Y.Z: <summary>" and tags vX.Y.Z.
#
# Nothing is pushed. Finish with: git push --follow-tags
# (release.yml then builds and attaches the release binaries).
#
# AGENT INVOCATION (non-interactive shells get EOF at the confirm prompt
# and abort silently):
#   scripts/release.sh minor "summary" --yes          # cut
#   scripts/release.sh 0.9.0 "summary" --dry-run      # verify only

set -euo pipefail

cd "$(dirname "$0")/.."

BUMP=""
SUMMARY=""
ASSUME_YES=no
DRY_RUN=no

while [[ $# -gt 0 ]]; do
  case "$1" in
    -y|--yes) ASSUME_YES=yes ;;
    --dry-run) DRY_RUN=yes ;;
    -*) echo "ERROR: unknown flag '$1' (supported: -y/--yes, --dry-run)" >&2; exit 2 ;;
    *)
      if [[ -z "$BUMP" ]]; then BUMP="$1"
      elif [[ -z "$SUMMARY" ]]; then SUMMARY="$1"
      else echo "ERROR: unexpected argument '$1'" >&2; exit 2
      fi
      ;;
  esac
  shift
done

if [[ -z "$BUMP" || -z "$SUMMARY" ]]; then
  echo "usage: scripts/release.sh <patch|minor|major|x.y.z> \"one-line summary\" [-y|--yes] [--dry-run]" >&2
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
COUNT=$(grep -c '^## \[Unreleased\]' CHANGELOG.md)
[[ "$COUNT" -eq 1 ]] || { echo "ERROR: CHANGELOG.md has $COUNT '## [Unreleased]' headings — exactly one is required (fix the structure first)." >&2; exit 1; }
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

PYTHON=$(command -v python3 || command -v python || true)
[[ -n "$PYTHON" ]] || { echo "ERROR: python3 or python is required." >&2; exit 1; }

TODAY=$(date +%F)

echo "release: $OLD -> $NEW  ($SUMMARY)"
awk '/^## \[Unreleased\]/{f=1; print; next} /^## /{if (f) exit} f' CHANGELOG.md | grep -Ev '^\s*$' \
  | sed 's/^/    | /'

if [[ "$DRY_RUN" == yes ]]; then
  echo "dry run: preconditions pass, would cut v$NEW. Nothing was changed."
  exit 0
fi

if [[ "$ASSUME_YES" != yes ]]; then
  read -r -p "Cut release v$NEW? [y/N] " CONFIRM
  [[ "$CONFIRM" =~ ^[Yy]$ ]] || { echo "aborted."; exit 1; }
fi

# --- rotate the changelog, bump Cargo.toml, update README version ----------

"$PYTHON" - "$OLD" "$NEW" "$TODAY" <<'PYEOF'
import re, sys

old, new, today = sys.argv[1], sys.argv[2], sys.argv[3]

with open("CHANGELOG.md", encoding="utf-8", newline="") as fh:
    cl = fh.read()
nl = "\r\n" if "\r\n" in cl else "\n"
rotated = f"## [{new}] - {today}"
cl_new = cl.replace("## [Unreleased]", f"## [Unreleased]{nl}{nl}{rotated}", 1)
if cl_new == cl:
    sys.exit("ERROR: failed to rotate ## [Unreleased] in CHANGELOG.md")
with open("CHANGELOG.md", "w", encoding="utf-8", newline="") as fh:
    fh.write(cl_new)

with open("Cargo.toml", encoding="utf-8", newline="") as fh:
    cargo = fh.read()
cargo_new, n = re.subn(
    rf'(?m)^(version\s*=\s*"){re.escape(old)}(")$',
    rf"\g<1>{new}\g<2>",
    cargo,
    count=1,
)
if n != 1:
    sys.exit(f"ERROR: failed to bump version {old} -> {new} in Cargo.toml")
with open("Cargo.toml", "w", encoding="utf-8", newline="") as fh:
    fh.write(cargo_new)

with open("README.md", encoding="utf-8", newline="") as fh:
    readme = fh.read()
readme_new = readme.replace(f"RIONT v{old}", f"RIONT v{new}")
if readme_new != readme:
    with open("README.md", "w", encoding="utf-8", newline="") as fh:
        fh.write(readme_new)
PYEOF

# --- build, test -----------------------------------------------------------

echo "--- cargo build --workspace (refresh Cargo.lock) ---"
cargo build --workspace
echo "--- cargo test --workspace ---"
cargo test --workspace

# --- commit + tag ----------------------------------------------------------

git add Cargo.toml Cargo.lock CHANGELOG.md README.md
git commit -m "Version $NEW: $SUMMARY"
git tag -a "v$NEW" -m "RIONT $NEW — $SUMMARY"

echo ""
echo "release v$NEW cut."
echo "next:  git push --follow-tags   (CI then builds the release binaries)"
