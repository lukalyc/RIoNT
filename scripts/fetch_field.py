#!/usr/bin/env python
"""Fetch an official FRC field map and make it loadable by RIONT.

THE quick path for a new season (e.g. 2026):

  1. PathPlanner publishes the field JSON for the new game
     (https://github.com/pathplanner/PathPlanner - fields live in the repo
     and inside the PathPlanner app; AdvantageScope bundles equivalents).
  2. Run one of:
        python scripts/fetch_field.py 2026                # try known URLs
        python scripts/fetch_field.py --url <json-url>    # any direct URL
        python scripts/fetch_field.py --from-file f.json --name 2026-game
                                                          # manual download
  3. The script validates + normalizes to RIONT's format, writes
     fields/<name>.json, and prints the config.json snippet:

        "field": { "walls_file": "fields/2026-game.json", ... }

  4. Restart RIONT (or edit config via the palette's Open Configuration and
     save) - the field card renders the new map. No rebuild ever needed.

RIONT field JSON schema (meters, blue-alliance origin):
  { "game": "<name>", "fieldLength": <m>, "fieldWidth": <m>,
    "walls": [ [[x, y], [x, y], ...], ... ] }

Requires: Python 3.9+. Network fetch uses urllib (no pip deps).
"""
from __future__ import annotations

import argparse
import json
import math
import pathlib
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIELDS_DIR = ROOT / "fields"

# Candidate source URLs per year, tried in order. These point at the
# PathPlanner repo's bundled field files; if a path 404s (repo layout
# changes between seasons), update the entry or pass --url / --from-file.
URL_CANDIDATES: dict[str, list[str]] = {
    "2024": [
        "https://raw.githubusercontent.com/pathplanner/PathPlanner/main/pathplannerlib/src/main/native/resources/2024-crescendo.json",
        "https://raw.githubusercontent.com/pathplanner/PathPlanner/main/pathplannerlib/src/main/resources/2024-crescendo.json",
    ],
    "2025": [
        "https://raw.githubusercontent.com/pathplanner/PathPlanner/main/pathplannerlib/src/main/resources/2025-reefscape.json",
    ],
    "2026": [],  # fill in when PathPlanner publishes the 2026 field
}


def _finite(x) -> bool:
    return isinstance(x, (int, float)) and math.isfinite(x)


def load_source(text: str) -> dict:
    """Accept PathPlanner-style JSON and normalize to RIONT's schema.

    PathPlanner field files use fieldLength/fieldWidth (meters) and a
    'walls' array of polylines. Some variants nest polylines as
    {points: [...]} or use field-dimension keys in inches — those are
    converted. Anything unparseable raises with a clear message.
    """
    data = json.loads(text)

    # PathPlanner ships walls in meters under 'walls' (list of polylines).
    walls = data.get("walls")
    if walls is None and "field" in data and isinstance(data["field"], dict):
        # AdvantageScope-style wrapper
        inner = data["field"]
        walls = inner.get("walls")
        data = {**data, **inner}

    if not isinstance(walls, list) or not walls:
        raise SystemExit(
            "no 'walls' array found. Expected PathPlanner field JSON: "
            '{"fieldLength": m, "fieldWidth": m, "walls": [[[x,y],...],...]}. '
            "If the source uses a different layout, convert it by hand and "
            "place it in fields/ manually."
        )

    norm_walls: list[list[list[float]]] = []
    for poly in walls:
        if isinstance(poly, dict):  # {points: [[x,y],...]} variant
            poly = poly.get("points", [])
        pts = [[float(p[0]), float(p[1])] for p in poly]
        if len(pts) >= 2:
            norm_walls.append(pts)

    length = float(data.get("fieldLength") or data.get("length") or 0)
    width = float(data.get("fieldWidth") or data.get("width") or 0)
    if not (_finite(length) and length > 1.0 and _finite(width) and width > 1.0):
        raise SystemExit(
            f"implausible field dimensions ({length} x {width}) — source is "
            "probably not in meters. Convert manually to the RIONT schema."
        )

    game = data.get("game") or data.get("name") or "unnamed"
    return {
        "game": str(game),
        "fieldLength": length,
        "fieldWidth": width,
        "walls": norm_walls,
    }


def save(field: dict, name: str) -> pathlib.Path:
    FIELDS_DIR.mkdir(exist_ok=True)
    out = FIELDS_DIR / f"{name}.json"
    out.write_text(json.dumps(field, indent=2), encoding="utf-8")
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("year", nargs="?", help="season year, e.g. 2026")
    ap.add_argument("--url", help="direct URL of a field JSON")
    ap.add_argument("--from-file", help="local field JSON to convert")
    ap.add_argument("--name", help="output name (default: <year>-game)")
    args = ap.parse_args()

    text: str | None = None
    if args.from_file:
        text = pathlib.Path(args.from_file).read_text(encoding="utf-8")
    elif args.url:
        print(f"fetching {args.url} ...")
        text = urllib.request.urlopen(args.url, timeout=30).read().decode("utf-8")
    elif args.year:
        for url in URL_CANDIDATES.get(args.year, []):
            try:
                print(f"trying {url} ...")
                text = urllib.request.urlopen(url, timeout=30).read().decode("utf-8")
                break
            except Exception as e:  # noqa: BLE001 - try the next candidate
                print(f"  failed: {e}")
        if text is None:
            print(
                f"\nNo working source URL for {args.year}. When PathPlanner "
                "publishes the field:\n"
                "  1. download the field JSON\n"
                f"  2. python scripts/fetch_field.py --from-file <file> --name {args.year}-game\n"
                "(or add the raw URL to URL_CANDIDATES in this script)"
            )
            return 1
    else:
        ap.error("give a YEAR, --url, or --from-file")

    name = args.name or (f"{args.year}-game" if args.year else "custom")
    field = load_source(text)
    out = save(field, name)
    rel = out.relative_to(ROOT)
    print(f"\nwrote {out}  ({field['game']}: {field['fieldLength']} x {field['fieldWidth']} m,")
    print(f"         {len(field['walls'])} wall polylines)")
    print("\nActivate it — config.json:")
    print(json.dumps(
        {"field": {"walls_file": str(rel), "alliance": "blue"}},
        indent=2,
    ))
    print("\nThen restart RIONT (or re-save config via the palette's Open Configuration).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
