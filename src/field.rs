//! Field geometry for the pose field card.
//!
//! Coordinates are meters, blue-alliance origin (blue driver-station wall
//! at x = 0, +x into the field, +y to the left), matching Limelight
//! `botpose_wpiblue` and WPILib odometry frames.
//!
//! Maps come from three sources, in priority order:
//!   1. `config.field.walls_file` — external JSON (PathPlanner format),
//!      THE path for new seasons: drop in the official file, no rebuild.
//!   2. `config.field.map` — a built-in map name (see BUILTIN_MAPS).
//!   3. The default built-in (`2025-reefscape`).
//!
//! BUILT-IN interior obstacles are APPROXIMATIONS (see each const). The
//! perimeter is exact. Official geometry arrives via `scripts/fetch_field.py`
//! + `walls_file`; a season swap should never require editing this file.
//!
//! Alliance flip is RENDERING-ONLY and USER-DRIVEN: the app never infers
//! an alliance. When the user sets `config.field.alliance = "red"` via
//! the palette, the card mirrors x -> length - x before drawing; stored
//! values are untouched.

use crate::nt::store::NtValue;
use serde::Deserialize;

/// A field map: extents + wall polylines (meters, blue origin).
#[derive(Debug, Clone)]
pub struct FieldMap {
    /// Map KEY (BUILTIN_MAPS entry / file stem) — used by Cycle Map.
    pub name: String,
    /// Display title from the JSON's "game" field (may equal name).
    pub game: String,
    pub length_m: f64,
    pub width_m: f64,
    pub walls: Vec<Vec<(f64, f64)>>,
    /// Game-line/tape marks — same geometry, drawn dimmer than walls.
    pub marks: Vec<Vec<(f64, f64)>>,
}

/// Built-in maps, in cycle order (palette `Field: Cycle Map`).
pub const BUILTIN_MAPS: [&str; 3] = ["2024-crescendo", "2025-reefscape", "2026-rebuilt"];

/// 2024 CRESCENDO: 16.54 x 8.21 m. Perimeter exact; subwoofer/amp/stage
/// are approximations pending official PathPlanner geometry.
const CRESCENDO_WALLS: &[&[(f64, f64)]] = &[
    // Perimeter (exact).
    &[
        (0.0, 0.0),
        (16.54, 0.0),
        (16.54, 8.21),
        (0.0, 8.21),
        (0.0, 0.0),
    ],
    // Subwoofer: protrusion on the blue driver-station wall (approx).
    &[(0.0, 2.9), (1.2, 2.9), (1.2, 5.3), (0.0, 5.3)],
    // Amp: corner structure, blue-left (approx).
    &[(0.0, 6.6), (1.2, 6.6), (1.2, 8.21), (0.0, 8.21)],
    // Stage: center scoring structure (approx hexagon).
    &[
        (6.1 + 1.0, 4.105),
        (6.1 + 0.5, 4.105 + 0.87),
        (6.1 - 0.5, 4.105 + 0.87),
        (6.1 - 1.0, 4.105),
        (6.1 - 0.5, 4.105 - 0.87),
        (6.1 + 0.5, 4.105 - 0.87),
        (6.1 + 1.0, 4.105),
    ],
];

/// 2025 REEFSCAPE: 16.54 x 8.21 m. Perimeter exact; reef/processor/cage
/// posts are approximations pending official PathPlanner geometry.
const REEFSCAPE_WALLS: &[&[(f64, f64)]] = &[
    // Perimeter (exact).
    &[
        (0.0, 0.0),
        (16.54, 0.0),
        (16.54, 8.21),
        (0.0, 8.21),
        (0.0, 0.0),
    ],
    // Reef: hexagonal scoring structure at field center (approx).
    &[
        (8.27 + 1.10, 4.105),
        (8.27 + 0.55, 4.105 + 0.953),
        (8.27 - 0.55, 4.105 + 0.953),
        (8.27 - 1.10, 4.105),
        (8.27 - 0.55, 4.105 - 0.953),
        (8.27 + 0.55, 4.105 - 0.953),
        (8.27 + 1.10, 4.105),
    ],
    // Processor: wall station on the blue half (approx).
    &[(3.0, 0.0), (3.0, 0.8), (4.2, 0.8), (4.2, 0.0)],
    // Cage posts near each driver-station wall (approx).
    &[(1.2, 3.4), (1.2, 4.8)],
    &[(15.34, 3.4), (15.34, 4.8)],
];

/// 2026 season: the official map ships as fields/2026-rebuilt.json
/// (generated from Choreo's vector drawing by scripts/fetch_field.py).
/// This perimeter-only const is the fallback when that file is absent.
const TBA_2026_WALLS: &[&[(f64, f64)]] = &[&[
    (0.0, 0.0),
    (16.54, 0.0),
    (16.54, 8.21),
    (0.0, 8.21),
    (0.0, 0.0),
]];

/// PathPlanner-style field JSON:
/// `{ "game": "...", "fieldLength": m, "fieldWidth": m,
///    "walls": [ [[x,y],[x,y],...], ... ] }` — meters, blue origin.
/// Snake-case aliases accepted for hand-written files.
#[derive(Debug, Deserialize)]
#[allow(non_snake_case)] // PathPlanner field JSON uses camelCase keys
struct FieldJson {
    #[serde(default, alias = "game_name")]
    game: Option<String>,
    #[serde(alias = "field_length")]
    fieldLength: f64,
    #[serde(alias = "field_width")]
    fieldWidth: f64,
    #[serde(default)]
    walls: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    marks: Vec<Vec<[f64; 2]>>,
}

impl FieldMap {
    pub fn builtin(name: &str) -> Option<FieldMap> {
        let (length_m, width_m, walls) = match name {
            "2024-crescendo" => (16.54, 8.21, CRESCENDO_WALLS),
            "2025-reefscape" => (16.54, 8.21, REEFSCAPE_WALLS),
            "2026-rebuilt" => (16.541, 8.0692, TBA_2026_WALLS),
            _ => return None,
        };
        Some(FieldMap {
            name: name.to_string(),
            game: name.to_string(),
            length_m,
            width_m,
            walls: walls.iter().map(|p| p.to_vec()).collect(),
            marks: Vec::new(),
        })
    }

    /// Parse a field JSON (PathPlanner format). Rejects garbage rather
    /// than guessing: extents must be positive, walls must be non-empty
    /// polylines with finite coordinates.
    pub fn from_json(text: &str, name: &str) -> Result<FieldMap, String> {
        let f: FieldJson = serde_json::from_str(text).map_err(|e| format!("field json: {}", e))?;
        if !(f.fieldLength.is_finite() && f.fieldLength > 1.0)
            || !(f.fieldWidth.is_finite() && f.fieldWidth > 1.0)
        {
            return Err("field json: implausible extents".into());
        }
        if f.walls.is_empty() {
            return Err("field json: no walls".into());
        }
        let walls: Vec<Vec<(f64, f64)>> = f
            .walls
            .into_iter()
            .map(|poly| poly.into_iter().map(|p| (p[0], p[1])).collect())
            .collect();
        let marks: Vec<Vec<(f64, f64)>> = f
            .marks
            .into_iter()
            .map(|poly| poly.into_iter().map(|p| (p[0], p[1])).collect())
            .collect();
        if walls
            .iter()
            .chain(marks.iter())
            .any(|p| p.is_empty() || p.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()))
        {
            return Err("field json: non-finite wall coordinates".into());
        }
        Ok(FieldMap {
            name: name.to_string(),
            game: f.game.unwrap_or_else(|| name.to_string()),
            length_m: f.fieldLength,
            width_m: f.fieldWidth,
            walls,
            marks,
        })
    }
}

/// Resolve the active field map from config. Returns the map plus an
/// optional warning (walls_file failed to read/parse) for the caller to
/// toast — the built-in fallback always succeeds. Priority:
///   1. config.field.walls_file (explicit external JSON)
///   2. fields/<map>.json next to the launch directory — the drop-in slot
///      for generated maps (scripts/fetch_field.py)
///   3. the built-in const for that name (documented approximations)
pub fn resolve(config: &crate::config::Config) -> (FieldMap, Option<String>) {
    if let Some(path) = &config.field.walls_file {
        if !path.is_empty() {
            return match std::fs::read_to_string(path) {
                Ok(text) => match FieldMap::from_json(&text, path) {
                    Ok(m) => (m, None),
                    Err(e) => (builtin_map(config), Some(e)),
                },
                Err(e) => (
                    builtin_map(config),
                    Some(format!("walls_file {}: {}", path, e)),
                ),
            };
        }
    }
    if let Ok(text) = std::fs::read_to_string(format!("fields/{}.json", config.field.map)) {
        if let Ok(m) = FieldMap::from_json(&text, &config.field.map) {
            return (m, None);
        }
    }
    (builtin_map(config), None)
}

/// The built-in map selected by `config.field.map` (default REEFSCAPE
/// when the name is unknown). Does NOT consider walls_file — that is
/// resolve()'s job.
fn builtin_map(config: &crate::config::Config) -> FieldMap {
    FieldMap::builtin(&config.field.map)
        .or_else(|| FieldMap::builtin(&crate::config::FieldSettings::default().map))
        .expect("default map name is built in")
}

/// Force-pose reading for topics the user explicitly opted in via
/// `Field: Toggle Pose View on Active Card`. Interpretation is
/// Limelight-style for double arrays (x, y, ..., yaw_deg) and native for
/// decoded `Pose2d` values; everything else is refused rather than guessed.
pub fn forced_reading(value: &NtValue) -> Option<crate::pose::PoseReading> {
    match value {
        NtValue::DoubleArray(v) if v.len() >= 6 => Some(crate::pose::PoseReading {
            x: v[0],
            y: v[1],
            radians: v[5].to_radians(),
            source: crate::pose::PoseSource::Limelight,
        }),
        NtValue::Pose2d { x, y, radians } => Some(crate::pose::PoseReading {
            x: *x,
            y: *y,
            radians: *radians,
            source: crate::pose::PoseSource::Pose2dStruct,
        }),
        _ => None,
    }
}

/// Visual role of a converted wall polyline: the field perimeter, an
/// obstacle on one alliance half, or nothing (filtered upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallKind {
    Perimeter,
    BlueHalf,
    RedHalf,
}

/// Classify a polyline by its bbox. A shape spanning ~the whole field is
/// the perimeter; anything else belongs to whichever half its centroid
/// sits in (field maps are blue-origin, so left = blue).
pub fn wall_kind(bbox: (f64, f64, f64, f64), length: f64, width: f64) -> WallKind {
    let (x0, y0, x1, y1) = bbox;
    if x1 - x0 > 0.9 * length && y1 - y0 > 0.9 * width {
        return WallKind::Perimeter;
    }
    if (x0 + x1) / 2.0 < length / 2.0 {
        WallKind::BlueHalf
    } else {
        WallKind::RedHalf
    }
}

/// Canvas bounds (x, y) that contain the WHOLE field with letterboxing,
/// preserving field aspect on the braille dot grid. On standard 1:2
/// character cells the braille grid is square in dot-space, so the drawn
/// x-span/y-span ratio must equal `cols / (2 * rows)`.
pub fn fit_bounds(cols: usize, rows: usize, length: f64, width: f64) -> ((f64, f64), (f64, f64)) {
    let grid_aspect = (cols.max(1) as f64) / (2.0 * rows.max(1) as f64);
    let mut x_span = length;
    let mut y_span = length / grid_aspect;
    if y_span < width {
        // Wide card: letterbox x instead of cropping the field.
        y_span = width;
        x_span = width * grid_aspect;
    }
    let cx = length / 2.0;
    let cy = width / 2.0;
    (
        (cx - x_span / 2.0, cx + x_span / 2.0),
        (cy - y_span / 2.0, cy + y_span / 2.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_reading_accepts_double6_and_pose2d() {
        let p = forced_reading(&NtValue::DoubleArray(vec![1.0, 2.0, 0.0, 0.0, 0.0, 90.0])).unwrap();
        assert!((p.radians - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        let p = forced_reading(&NtValue::Pose2d {
            x: 3.0,
            y: 4.0,
            radians: 0.5,
        })
        .unwrap();
        assert_eq!((p.x, p.y), (3.0, 4.0));
    }

    #[test]
    fn forced_reading_refuses_short_arrays_and_scalars() {
        // A double[3] (Field2d-style object or odometry scalar) is NOT enough.
        assert!(forced_reading(&NtValue::DoubleArray(vec![1.0, 2.0, 3.0])).is_none());
        assert!(forced_reading(&NtValue::Double(1.0)).is_none());
        assert!(forced_reading(&NtValue::Str("1,2".into())).is_none());
    }

    #[test]
    fn fit_bounds_contain_field_with_letterbox() {
        for (cols, rows) in [(30usize, 14usize), (60, 10), (20, 20), (10, 5)] {
            let ((x0, x1), (y0, y1)) = fit_bounds(cols, rows, 16.54, 8.21);
            // Whole field always visible.
            assert!(x0 <= 0.0 && x1 >= 16.54, "{cols}x{rows}");
            assert!(y0 <= 0.0 && y1 >= 8.21, "{cols}x{rows}");
            // Aspect preserved: x_span/y_span == cols/(2*rows).
            let span_ratio = (x1 - x0) / (y1 - y0);
            let want = cols as f64 / (2.0 * rows as f64);
            assert!((span_ratio - want).abs() < 1e-9, "{cols}x{rows}");
        }
    }

    #[test]
    fn builtins_resolve() {
        for name in BUILTIN_MAPS {
            let m = FieldMap::builtin(name).unwrap_or_else(|| panic!("{}", name));
            assert_eq!(m.name, name);
            assert!(!m.walls.is_empty());
            assert!(m.length_m > 1.0 && m.width_m > 1.0);
        }
        assert!(FieldMap::builtin("1992-maize-maze").is_none());
    }

    #[test]
    fn parses_pathplanner_style_json() {
        let text = r#"{
            "game": "NextSeason",
            "fieldLength": 16.54,
            "fieldWidth": 8.21,
            "walls": [[[0,0],[16.54,0],[16.54,8.21],[0,8.21],[0,0]]]
        }"#;
        let m = FieldMap::from_json(text, "file.json").unwrap();
        // The map KEY stays the file/builtin name; "game" is display-only.
        assert_eq!(m.name, "file.json");
        assert_eq!(m.game, "NextSeason");
        assert_eq!((m.length_m, m.width_m), (16.54, 8.21));
        assert_eq!(m.walls[0].len(), 5);
    }

    #[test]
    fn json_rejects_garbage() {
        assert!(FieldMap::from_json("not json", "f").is_err());
        assert!(FieldMap::from_json(
            r#"{"fieldLength": 0.0, "fieldWidth": 8.21, "walls": [[[0,0],[1,1]]]}"#,
            "f"
        )
        .is_err());
        assert!(FieldMap::from_json(r#"{"fieldLength": 16.54, "fieldWidth": 8.21}"#, "f").is_err());
        assert!(FieldMap::from_json(
            r#"{"fieldLength": 16.54, "fieldWidth": 8.21, "walls": [[[0,0],[NaN,1]]]}"#,
            "f"
        )
        .is_err());
    }

    #[test]
    fn resolve_falls_back_to_builtin_on_bad_file() {
        let mut cfg = crate::config::Config::with_defaults();
        cfg.field.walls_file = Some("definitely/missing.json".into());
        let (m, warn) = resolve(&cfg);
        assert!(warn.is_some());
        assert!(!m.walls.is_empty());
    }

    #[test]
    fn wall_kind_classifies_perimeter_and_halves() {
        let (l, w) = (16.54, 8.21);
        // Full-span polyline = perimeter.
        assert_eq!(wall_kind((0.0, 0.0, l, w), l, w), WallKind::Perimeter);
        // Blue-origin: centroid left of center = blue half.
        assert_eq!(wall_kind((3.0, 2.0, 6.0, 6.0), l, w), WallKind::BlueHalf);
        assert_eq!(wall_kind((11.0, 2.0, 14.0, 6.0), l, w), WallKind::RedHalf);
        // A shape crossing the center line is classified by its centroid.
        assert_eq!(wall_kind((7.0, 2.0, 10.0, 6.0), l, w), WallKind::RedHalf);
    }
}
