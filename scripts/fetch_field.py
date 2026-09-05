#!/usr/bin/env python
"""Generate a RIONT field map programmatically from official vector sources.

When Choreo (https://github.com/SleipnirGroup/Choreo) publishes a season
field, its SVG is a scale drawing of the REAL field (walls, obstacles and
tape lines) in field coordinates. This script converts that vector drawing
into RIONT's field JSON — no hand-tracing, no guessing:

    python scripts/fetch_field.py 2026      # Choreo season -> fields/2026-game.json
    python scripts/fetch_field.py --svg f.svg --spec s.json --name 2026-game

It can also convert plain PathPlanner-format field JSONs:

    python scripts/fetch_field.py --from-file field.json --name custom

Activate the result in config.json:

    "field": { "walls_file": "fields/2026-game.json", "alliance": "blue" }

...or drop the file in fields/ named exactly like a built-in map
(e.g. fields/2026-tba.json) and the app picks it up automatically.

HOW IT WORKS (SVG mode):
  - the SVG is walked with full affine-transform tracking (translate,
    scale, matrix, rotate), so traced drawings in pixel space still land
    in meters
  - <rect> -> 4-point polyline, <circle>/<ellipse> -> 24-gon,
    <polygon>/<polyline> -> as-is, <line> -> 2 points, <path> -> M/L/H/V/
    C/Q sampled (curves at 8 segments)
  - shapes are classified by their ancestor group id: tape/line groups and
    game-piece marks (algae, fuel) become "marks" (rendered dimmer);
    everything structural (perimeter, obstacles) becomes "walls"
  - tiny shapes (< 12 cm across) and off-field shapes are dropped; the
    result is rounded to millimeters

Requires Python 3.9+, stdlib only.
"""
from __future__ import annotations

import argparse
import json
import math
import pathlib
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIELDS_DIR = ROOT / "fields"
SVG_NS = "{http://www.w3.org/2000/svg}"

# Choreo bundled season fields (verified vector sources).
#
# CAUTION — these SVGs are Choreo's UI illustrations, not CAD plans. The
# 2026 drawing is dimension-accurate (wall rect 16.59 x 8.12 vs a 16.541 x
# 8.069 field) and safe to convert. The 2025 drawing is STYLIZED: its
# outline measures 17.6 x 8.1 m and the reef is drawn off-center — do NOT
# ship a conversion of it; the built-in approximation is more accurate.
# Re-verify each season's SVG against the official field dimensions before
# trusting the output (the wall rect should match fieldLength/fieldWidth).
CHOREO_BASE = "https://raw.githubusercontent.com/SleipnirGroup/Choreo/main/src/components/field/svg/fields"
SEASONS = {
    "2026": {
        "svg": "FieldImage2026.svg",
        "spec": "2026-field.json",  # extents come from the spec (field-size)
        "game": "Rebuilt",
        "name": "2026-rebuilt",
    },
    # 2024 CRESCENDO has no Choreo SVG; the built-in approximation remains.
}

MARK_KEYS = ("tape", "line", "algae", "fuel")  # ancestor-id -> rendered dimmer
MIN_SIZE_M = 0.12  # skip smaller-than-a-can shapes (game piece dots, bolts)
MARGIN_M = 0.6  # drop shapes fully outside the field + margin
CURVE_SEGMENTS = 8
GON_POINTS = 24


# ---------------------------------------------------------------------------
# affine transforms
# ---------------------------------------------------------------------------

def mat(a=1.0, b=0.0, c=0.0, d=1.0, e=0.0, f=0.0):
    return (a, b, c, d, e, f)


def mat_mul(m, n):
    """m . n — apply n first, then m (SVG nesting order)."""
    a1, b1, c1, d1, e1, f1 = m
    a2, b2, c2, d2, e2, f2 = n
    return (
        a1 * a2 + c1 * b2, b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2, b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1, b1 * e2 + d1 * f2 + f1,
    )


def apply(m, x, y):
    a, b, c, d, e, f = m
    return (a * x + c * y + e, b * x + d * y + f)


def parse_transform(s):
    m = mat()
    for name, args in re.findall(r"(\w+)\s*\(([^)]*)\)", s or ""):
        vals = [float(v) for v in re.split(r"[\s,]+", args.strip()) if v]
        if name == "translate":
            t = mat(e=vals[0], f=vals[1] if len(vals) > 1 else 0.0)
        elif name == "scale":
            t = mat(a=vals[0], d=vals[1] if len(vals) > 1 else vals[0])
        elif name == "matrix":
            t = mat(*vals)
        elif name == "rotate":
            ang = math.radians(vals[0])
            ca, sa = math.cos(ang), math.sin(ang)
            r = mat(a=ca, b=sa, c=-sa, d=ca)
            if len(vals) == 3:
                cx, cy = vals[1], vals[2]
                r = mat_mul(mat_mul(mat(e=cx, f=cy), r), mat(e=-cx, f=-cy))
            t = r
        else:
            continue
        m = mat_mul(m, t)
    return m


# ---------------------------------------------------------------------------
# SVG shape extraction
# ---------------------------------------------------------------------------

def path_points(d, m):
    """Sample an SVG path (M/L/H/V/C/Q/S/Z, absolute + relative)."""
    tokens = re.findall(r"([MLHVCSQZTAmlhvcsqzta])|(-?\d*\.?\d+(?:e-?\d+)?)", d)
    seq = [(t[0], float(t[1]) if t[1] else None) for t in tokens]
    out: list[tuple[float, float]] = []
    i = 0
    cur = (0.0, 0.0)
    start = cur
    cmd = None

    def nums(count):
        nonlocal i
        vals = []
        while len(vals) < count and i < len(seq):
            if seq[i][0]:
                break
            vals.append(seq[i][1])
            i += 1
        return vals

    while i < len(seq):
        if seq[i][0]:
            cmd = seq[i][0]
            i += 1
        if cmd is None:
            break
        rel = cmd.islower()
        c = cmd.upper()
        if c == "M":
            (x, y) = nums(2)
            if rel:
                x += cur[0]
                y += cur[1]
            cur = (x, y)
            start = cur
            out.append(apply(m, *cur))
            cmd = "l" if rel else "L"  # implicit lineto
        elif c == "L":
            (x, y) = nums(2)
            if rel:
                x += cur[0]
                y += cur[1]
            cur = (x, y)
            out.append(apply(m, *cur))
        elif c == "H":
            (x,) = nums(1)
            cur = (cur[0] + x if rel else x, cur[1])
            out.append(apply(m, *cur))
        elif c == "V":
            (y,) = nums(1)
            cur = (cur[0], cur[1] + y if rel else y)
            out.append(apply(m, *cur))
        elif c == "C":
            (x1, y1, x2, y2, x, y) = nums(6)
            if rel:
                x1 += cur[0]; y1 += cur[1]; x2 += cur[0]; y2 += cur[1]
                x += cur[0]; y += cur[1]
            for s in range(1, CURVE_SEGMENTS + 1):
                t = s / CURVE_SEGMENTS
                mt = 1 - t
                bx = mt**3 * cur[0] + 3 * mt**2 * t * x1 + 3 * mt * t**2 * x2 + t**3 * x
                by = mt**3 * cur[1] + 3 * mt**2 * t * y1 + 3 * mt * t**2 * y2 + t**3 * y
                out.append(apply(m, bx, by))
            cur = (x, y)
        elif c in ("Q", "S"):
            (x1, y1, x, y) = nums(4)
            if rel:
                x1 += cur[0]; y1 += cur[1]; x += cur[0]; y += cur[1]
            for s in range(1, CURVE_SEGMENTS + 1):
                t = s / CURVE_SEGMENTS
                mt = 1 - t
                bx = mt**2 * cur[0] + 2 * mt * t * x1 + t**2 * x
                by = mt**2 * cur[1] + 2 * mt * t * y1 + t**2 * y
                out.append(apply(m, bx, by))
            cur = (x, y)
        elif c == "Z":
            if out:
                out.append(apply(m, *start))
            cur = start
        else:  # A (arcs) — not used by these drawings; skip args conservatively
            nums(7)
    return out


def ngon(cx, cy, rx, ry, m, n=GON_POINTS):
    return [apply(m, cx + rx * math.cos(2 * math.pi * k / n),
                  cy + ry * math.sin(2 * math.pi * k / n)) for k in range(n)]


def shape_points(el, m):
    """Element -> list of polylines (already transformed)."""
    tag = el.tag.replace(SVG_NS, "")
    if tag == "rect":
        x, y = float(el.get("x", 0)), float(el.get("y", 0))
        w, h = float(el.get("width", 0)), float(el.get("height", 0))
        pts = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
        return [[apply(m, *p) for p in pts]]
    if tag == "circle":
        return [ngon(float(el.get("cx", 0)), float(el.get("cy", 0)),
                     float(el.get("r", 0)), float(el.get("r", 0)), m)]
    if tag == "ellipse":
        return [ngon(float(el.get("cx", 0)), float(el.get("cy", 0)),
                     float(el.get("rx", 0)), float(el.get("ry", 0)), m)]
    if tag in ("polygon", "polyline"):
        pts = []
        for pair in re.findall(r"(-?\d*\.?\d+)[,\s]+(-?\d*\.?\d+)", el.get("points", "")):
            pts.append(apply(m, float(pair[0]), float(pair[1])))
        if tag == "polygon" and len(pts) > 2:
            pts.append(pts[0])
        return [pts] if pts else []
    if tag == "line":
        pts = [(float(el.get("x1", 0)), float(el.get("y1", 0))),
               (float(el.get("x2", 0)), float(el.get("y2", 0)))]
        return [[apply(m, *p) for p in pts]]
    if tag == "path":
        return [path_points(el.get("d", ""), m)]
    return []


def bbox(polys):
    xs = [p[0] for poly in polys for p in poly]
    ys = [p[1] for poly in polys for p in poly]
    return (min(xs), min(ys), max(xs), max(ys))


def is_mark(ancestors: str) -> bool:
    a = ancestors.lower()
    return any(k in a for k in MARK_KEYS)


def svg_to_field(svg_text: str, length: float, width: float) -> dict:
    root = ET.fromstring(svg_text)
    defs = {el.get("id"): el for el in root.iter() if el.get("id")}
    xlink = "{http://www.w3.org/1999/xlink}href"

    def walk(el, m, ancestors, depth=0):
        for ch in el:
            tag = ch.tag.replace(SVG_NS, "")
            if tag == "use":
                # Shapes are often defined once in <defs> and placed with
                # <use href="#id" x y transform>: resolve the reference.
                href = ch.get("href") or ch.get(xlink) or ""
                target = defs.get(href.lstrip("#")) if href else None
                if target is not None and depth < 8:
                    gm = mat_mul(m, parse_transform(ch.get("transform")))
                    x, y = float(ch.get("x", 0) or 0), float(ch.get("y", 0) or 0)
                    if x or y:
                        gm = mat_mul(gm, mat(e=x, f=y))
                    ids = " ".join(filter(None, [ch.get("id"), href.lstrip("#")]))
                    yield from walk(target, gm, f"{ancestors} {ids}", depth + 1)
                continue
            gm = mat_mul(m, parse_transform(ch.get("transform")))
            ids = " ".join(filter(None, [ch.get("id"), ch.get("class")]))
            chain = f"{ancestors} {ids}"
            if tag == "g":
                yield from walk(ch, gm, chain, depth)
            elif tag in ("rect", "circle", "ellipse", "polygon", "polyline", "line", "path"):
                for poly in shape_points(ch, gm):
                    yield poly, is_mark(chain), ids

    walls: list[list[list[float]]] = []
    marks: list[list[list[float]]] = []
    seen: set[bytes] = set()
    for poly, mark, _ in walk(root, mat(), ""):
        if len(poly) < 2:
            continue
        x0, y0, x1, y1 = bbox([poly])
        if x1 < -MARGIN_M or y1 < -MARGIN_M or x0 > length + MARGIN_M or y0 > width + MARGIN_M:
            continue  # fully off-field
        if (x1 - x0) < MIN_SIZE_M and (y1 - y0) < MIN_SIZE_M:
            continue  # smaller than MIN_SIZE_M in both axes
        rounded = [[round(x, 3), round(y, 3)] for (x, y) in poly]
        key = json.dumps(rounded, sort_keys=True).encode()
        if key in seen:
            continue
        seen.add(key)
        (marks if mark else walls).append(rounded)

    return {"walls": walls, "marks": marks}


# ---------------------------------------------------------------------------
# entry points
# ---------------------------------------------------------------------------

def load_source_json(text: str) -> dict:
    data = json.loads(text)
    walls = data.get("walls")
    if not isinstance(walls, list) or not walls:
        raise SystemExit("no 'walls' array — expected PathPlanner field JSON")
    length = float(data.get("fieldLength") or data.get("length") or 0)
    width = float(data.get("fieldWidth") or data.get("width") or 0)
    if not (length > 1.0 and width > 1.0):
        raise SystemExit(f"implausible field dimensions ({length} x {width}) — not meters?")
    return {"game": str(data.get("game") or "unnamed"), "fieldLength": length,
            "fieldWidth": width, "walls": walls, "marks": []}


def fetch_url(url: str) -> str:
    print(f"fetching {url} ...")
    return urllib.request.urlopen(url, timeout=30).read().decode("utf-8")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("year", nargs="?", help="season with a Choreo SVG: 2025, 2026")
    ap.add_argument("--svg", help="local Choreo season SVG to convert")
    ap.add_argument("--length", type=float, help="field length in meters (SVG mode)")
    ap.add_argument("--width", type=float, help="field width in meters (SVG mode)")
    ap.add_argument("--from-file", help="local PathPlanner-style field JSON")
    ap.add_argument("--url", help="direct URL of a field JSON")
    ap.add_argument("--name", help="output name (default: <year>-game)")
    args = ap.parse_args()

    if args.year:
        season = SEASONS.get(args.year)
        if season is None:
            print(f"no Choreo SVG for {args.year}. Known seasons: {', '.join(SEASONS)}.")
            print("When Choreo publishes the field, add it to SEASONS here, or use "
                  "--svg/--spec manually.")
            return 1
        svg_text = fetch_url(f"{CHOREO_BASE}/{season['svg']}")
        if "spec" in season:
            spec = json.loads(fetch_url(f"{CHOREO_BASE}/{season['spec']}"))
            length, width = spec["field-size"]
        else:
            length, width = season["length"], season["width"]
        name = args.name or season["name"]
        geo = svg_to_field(svg_text, length, width)
        field = {"game": season["game"], "fieldLength": length, "fieldWidth": width, **geo}
    elif args.svg:
        if not (args.length and args.width):
            ap.error("--svg needs --length and --width (meters)")
        svg_text = pathlib.Path(args.svg).read_text(encoding="utf-8")
        name = args.name or "custom"
        geo = svg_to_field(svg_text, args.length, args.width)
        field = {"game": name, "fieldLength": args.length, "fieldWidth": args.width, **geo}
    elif args.from_file:
        field = load_source_json(pathlib.Path(args.from_file).read_text(encoding="utf-8"))
        name = args.name or "custom"
    elif args.url:
        field = load_source_json(fetch_url(args.url))
        name = args.name or "custom"
    else:
        ap.error("give a YEAR, --svg, --from-file, or --url")

    FIELDS_DIR.mkdir(exist_ok=True)
    out = FIELDS_DIR / f"{name}.json"
    out.write_text(json.dumps(field, indent=1), encoding="utf-8")
    print(f"\nwrote {out}")
    print(f"  {field['game']}: {field['fieldLength']} x {field['fieldWidth']} m, "
          f"{len(field['walls'])} wall/obstacle polylines, {len(field['marks'])} marks")
    print("\nActivate it — config.json:")
    print(json.dumps({"field": {"walls_file": f"fields/{name}.json"}}, indent=2))
    print("\n(or name the file after a built-in map, e.g. fields/2026-tba.json, and\n"
          " the app loads it automatically — no config edit needed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
