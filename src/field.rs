//! 2025 REEFSCAPE field geometry for the pose field card.
//!
//! Wall polylines are meters, blue-alliance origin (blue driver-station
//! wall at x = 0, +x into the field, +y to the left), matching Limelight
//! `botpose_wpiblue` and WPILib odometry frames. The perimeter is exact;
//! interior obstacles are APPROXIMATIONS pending verification against
//! PathPlanner's official field JSON — a season swap should be a
//! data-only change to the constants below (PathPlanner JSON layout:
//! `{ game, fieldLength, fieldWidth, walls: [[[x,y],...] ...] }`).
//!
//! Alliance flip is RENDERING-ONLY and USER-DRIVEN: the app never infers
//! an alliance. When the user sets `config.field.alliance = "red"` via
//! the palette, the card mirrors x -> length - x before drawing; stored
//! values are untouched.

use crate::nt::store::NtValue;

/// Wall + obstacle polylines (meters, blue origin). Perimeter is exact;
/// interior shapes are approximations (see module docs).
pub const WALLS: &[&[(f64, f64)]] = &[
    // Perimeter (exact).
    &[(0.0, 0.0), (16.54, 0.0), (16.54, 8.21), (0.0, 8.21), (0.0, 0.0)],
    // Reef: hexagonal scoring structure at field center (approximation).
    &[
        (8.27 + 1.10, 4.105),
        (8.27 + 0.55, 4.105 + 0.953),
        (8.27 - 0.55, 4.105 + 0.953),
        (8.27 - 1.10, 4.105),
        (8.27 - 0.55, 4.105 - 0.953),
        (8.27 + 0.55, 4.105 - 0.953),
        (8.27 + 1.10, 4.105),
    ],
    // Processor: wall station on the blue half (approximation).
    &[(3.0, 0.0), (3.0, 0.8), (4.2, 0.8), (4.2, 0.0)],
    // Cage posts near each driver-station wall (approximation).
    &[(1.2, 3.4), (1.2, 4.8)],
    &[(15.34, 3.4), (15.34, 4.8)],
];

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

/// Canvas bounds (x, y) that contain the WHOLE field with letterboxing,
/// preserving field aspect on the braille dot grid. On standard 1:2
/// character cells the braille grid is square in dot-space, so the drawn
/// x-span/y-span ratio must equal `cols / (2 * rows)`.
pub fn fit_bounds(
    cols: usize,
    rows: usize,
    length: f64,
    width: f64,
) -> ((f64, f64), (f64, f64)) {
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
        let p = forced_reading(&NtValue::DoubleArray(vec![
            1.0, 2.0, 0.0, 0.0, 0.0, 90.0,
        ]))
        .unwrap();
        assert!((p.radians - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        let p = forced_reading(&NtValue::Pose2d { x: 3.0, y: 4.0, radians: 0.5 }).unwrap();
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
}
