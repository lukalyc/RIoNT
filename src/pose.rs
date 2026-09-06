//! Conservative pose-topic classification + WPILib `struct:Pose2d` decoding.
//!
//! Deliberately paranoid: only exact Limelight pose leaf names and the
//! exact `struct:Pose2d` type string classify as a robot pose. Lookalikes
//! (`targetpose`, odometry scalars, Field2d object lists, other double[6]
//! arrays) stay ordinary topics — a false positive would draw a phantom
//! robot on the field, so there is no fuzzy matching, ever.

use crate::nt::store::NtValue;

/// Where a pose reading came from (drives later rendering decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseSource {
    /// Limelight `botpose*` double[6] (x, y, z, roll, pitch, yaw_deg),
    /// meters/degrees, blue-alliance origin.
    Limelight,
    /// WPILib `struct:Pose2d`: 24-byte packed little-endian x, y, radians.
    Pose2dStruct,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseReading {
    pub x: f64,
    pub y: f64,
    pub radians: f64,
    pub source: PoseSource,
}

/// Exact Limelight pose leaf names (any camera/namespace prefix allowed —
/// suffix match on the leaf only).
const LIMELIGHT_LEAVES: [&str; 5] = [
    "botpose",
    "botpose_wpiblue",
    "botpose_wpired",
    "botpose_orb_wpiblue",
    "botpose_orb_wpired",
];

/// Classify a topic's latest value as a robot pose. Returns `None` for
/// everything that is not an exact, well-known pose topic.
pub fn classify(topic_name: &str, type_str: Option<&str>, value: &NtValue) -> Option<PoseReading> {
    let leaf = topic_name.rsplit('/').next().unwrap_or(topic_name);
    if LIMELIGHT_LEAVES.contains(&leaf) {
        // Limelight: [x, y, z, roll, pitch, yaw_deg]. An exact name with a
        // wrong-shaped value is NOT a reading — refuse rather than guess.
        if let NtValue::DoubleArray(v) = value {
            if v.len() >= 6 {
                return Some(PoseReading {
                    x: v[0],
                    y: v[1],
                    radians: v[5].to_radians(),
                    source: PoseSource::Limelight,
                });
            }
        }
        return None;
    }
    if type_str == Some("struct:Pose2d") {
        if let NtValue::Pose2d { x, y, radians } = value {
            return Some(PoseReading {
                x: *x,
                y: *y,
                radians: *radians,
                source: PoseSource::Pose2dStruct,
            });
        }
    }
    None
}

/// Decode a WPILib `struct:Pose2d` binary payload (24-byte little-endian
/// x, y, radians) into an `NtValue::Pose2d`. When a `structSchema` property
/// was advertised it must declare the three fields in the canonical order
/// we know how to read — otherwise the payload is left as raw bytes rather
/// than guessed at. NaN payloads are rejected as corrupt.
pub fn decode_pose2d(bytes: &[u8], struct_schema: Option<&str>) -> Option<NtValue> {
    if let Some(schema) = struct_schema {
        if !schema_matches_canonical(schema) {
            return None;
        }
    }
    if bytes.len() < 24 {
        return None;
    }
    let x = f64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let y = f64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let radians = f64::from_le_bytes(bytes[16..24].try_into().ok()?);
    if x.is_nan() || y.is_nan() || radians.is_nan() {
        return None;
    }
    Some(NtValue::Pose2d { x, y, radians })
}

/// WPILib's own Pose2d schema reads
/// `Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}`.
/// Accept only schemas declaring exactly those fields in that order.
fn schema_matches_canonical(schema: &str) -> bool {
    let Some(x) = schema.find("x:double") else {
        return false;
    };
    let Some(y) = schema.find("y:double") else {
        return false;
    };
    let Some(r) = schema.find("radians:double") else {
        return false;
    };
    x < y && y < r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn da(v: &[f64]) -> NtValue {
        NtValue::DoubleArray(v.to_vec())
    }

    #[test]
    fn limelight_botpose_classifies() {
        let p = classify(
            "limelight-front/botpose_wpiblue",
            Some("double[]"),
            &da(&[2.0, 3.0, 0.0, 0.0, 0.0, 90.0]),
        )
        .unwrap();
        assert_eq!(p.x, 2.0);
        assert_eq!(p.y, 3.0);
        assert!((p.radians - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert_eq!(p.source, PoseSource::Limelight);
    }

    #[test]
    fn limelight_orb_classifies() {
        let p = classify(
            "limelight-rear/botpose_orb_wpired",
            Some("double[]"),
            &da(&[1.0, 2.0, 0.0, 0.0, 0.0, 0.0]),
        )
        .unwrap();
        assert_eq!(p.source, PoseSource::Limelight);
    }

    #[test]
    fn lookalikes_do_not_classify() {
        // Megatag2 target pose: exact "pose-ish" name, wrong leaf.
        assert!(classify(
            "limelight-front/targetpose",
            Some("double[]"),
            &da(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
        )
        .is_none());
        // Exact name but wrong shape (double[3] Field2d-style).
        assert!(classify("botpose", Some("double[]"), &da(&[1.0, 2.0, 3.0])).is_none());
        // Odometry scalar: right values, unrelated name.
        assert!(classify("Swerve/odometry_x", Some("double"), &NtValue::Double(1.0)).is_none());
        // Generic "pose" leaf is NOT a known Limelight name.
        assert!(classify("pose", Some("double[]"), &da(&[1.0, 2.0, 3.0])).is_none());
        // Wrong type string entirely.
        assert!(classify(
            "odometry/pose",
            Some("struct:Pose3d"),
            &NtValue::Pose2d {
                x: 1.0,
                y: 2.0,
                radians: 0.0
            }
        )
        .is_none());
    }

    #[test]
    fn struct_pose2d_classifies() {
        let mut b = Vec::new();
        b.extend_from_slice(&1.5f64.to_le_bytes());
        b.extend_from_slice(&(-2.5f64).to_le_bytes());
        b.extend_from_slice(&0.25f64.to_le_bytes());
        let v = decode_pose2d(&b, None).unwrap();
        let p = classify("odometry/pose", Some("struct:Pose2d"), &v).unwrap();
        assert_eq!(p.x, 1.5);
        assert_eq!(p.y, -2.5);
        assert_eq!(p.radians, 0.25);
        assert_eq!(p.source, PoseSource::Pose2dStruct);
    }

    #[test]
    fn struct_decode_accepts_canonical_schema() {
        let b = vec![0u8; 24];
        let schema = "Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}";
        assert!(decode_pose2d(&b, Some(schema)).is_some());
    }

    #[test]
    fn struct_decode_refuses_unknown_schema() {
        let b = vec![0u8; 24];
        assert!(decode_pose2d(
            &b,
            Some(
                "SwerveModuleState{angle:Rotation2d{radians:double}, speedMetersPerSecond:double}"
            )
        )
        .is_none());
    }

    #[test]
    fn struct_decode_refuses_short_payload() {
        assert!(decode_pose2d(&[0u8; 23], None).is_none());
    }

    #[test]
    fn struct_decode_refuses_nan_payload() {
        let mut b = Vec::new();
        b.extend_from_slice(&f64::NAN.to_le_bytes());
        b.extend_from_slice(&0.0f64.to_le_bytes());
        b.extend_from_slice(&0.0f64.to_le_bytes());
        assert!(decode_pose2d(&b, None).is_none());
    }
}
