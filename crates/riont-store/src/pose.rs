//! Conservative pose-topic classification + WPILib struct decoding.
//!
//! Deliberately paranoid: only exact Limelight pose leaf names and the
//! exact `struct:Pose2d` type string classify as a robot pose. Lookalikes
//! (`targetpose`, odometry scalars, Field2d object lists, other double[6]
//! arrays) stay ordinary topics — a false positive would draw a phantom
//! robot on the field, so there is no fuzzy matching, ever.
//!
//! Struct decoding (`decode_*`): WPILib packs known struct types as
//! little-endian binary payloads on wire type DT_BINARY with a
//! `struct:<Name>` type string. Only the exact, well-known layouts below
//! are decoded — malformed or unknown payloads stay raw bytes and render
//! as `<N bytes>` rather than being guessed at.

use crate::store::NtValue;

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
    schema_declares_in_order(schema, &["x:double", "y:double", "radians:double"])
}

/// True when every named field appears in `schema` in the given order
/// (each field's first occurrence). A missing field fails; extra or
/// nested content around them is tolerated — we only need the reading
/// ORDER of the fields we decode.
fn schema_declares_in_order(schema: &str, fields: &[&str]) -> bool {
    let mut pos = 0;
    for f in fields {
        match schema[pos..].find(f) {
            Some(i) => pos += i + f.len(),
            None => return false,
        }
    }
    true
}

/// Decode a WPILib `struct:ChassisSpeeds` binary payload (24-byte
/// little-endian vx, vy, omega — m/s body-frame + rad/s) into an
/// `NtValue::ChassisSpeeds`. Same paranoia as [`decode_pose2d`]: an
/// advertised schema must declare the three fields in canonical order,
/// and NaN payloads are rejected as corrupt.
pub fn decode_chassis_speeds(bytes: &[u8], struct_schema: Option<&str>) -> Option<NtValue> {
    if let Some(schema) = struct_schema {
        if !schema_declares_in_order(schema, &["vx:double", "vy:double", "omega:double"]) {
            return None;
        }
    }
    let (vx, vy, omega) = decode_3_f64(bytes)?;
    Some(NtValue::ChassisSpeeds { vx, vy, omega })
}

/// Decode a WPILib `struct:Twist2d` binary payload (24-byte little-endian
/// dx, dy, dtheta — meters + radians) into an `NtValue::Twist2d`. Same
/// paranoia as [`decode_pose2d`].
pub fn decode_twist2d(bytes: &[u8], struct_schema: Option<&str>) -> Option<NtValue> {
    if let Some(schema) = struct_schema {
        if !schema_declares_in_order(schema, &["dx:double", "dy:double", "dtheta:double"]) {
            return None;
        }
    }
    let (dx, dy, dtheta) = decode_3_f64(bytes)?;
    Some(NtValue::Twist2d { dx, dy, dtheta })
}

/// Shared body of the 24-byte three-f64 struct decoders: exact length,
/// little-endian fields, NaN rejected as corrupt.
fn decode_3_f64(bytes: &[u8]) -> Option<(f64, f64, f64)> {
    if bytes.len() < 24 {
        return None;
    }
    let a = f64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let b = f64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let c = f64::from_le_bytes(bytes[16..24].try_into().ok()?);
    if a.is_nan() || b.is_nan() || c.is_nan() {
        return None;
    }
    Some((a, b, c))
}

/// Decode a WPILib `struct:SwerveModuleStates` binary payload: N modules
/// of 16 bytes each, little-endian `(angle: f64 radians, speed: f64 m/s)`.
/// N is the payload length / 16 — non-zero, a multiple of 16 and capped
/// at 8 (a real swerve drive has 4; anything else is corrupt, refused
/// rather than guessed at). No schema gate: the payload length alone
/// fully determines N, and WPILib's per-module schema varies by season.
/// NaN payloads are rejected as corrupt.
pub fn decode_swerve_module_states(bytes: &[u8]) -> Option<NtValue> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(16) || bytes.len() / 16 > MAX_SWERVE_MODULES
    {
        return None;
    }
    let mut states = Vec::with_capacity(bytes.len() / 16);
    for pair in bytes.as_chunks::<16>().0 {
        let angle = f64::from_le_bytes(pair[0..8].try_into().ok()?);
        let speed = f64::from_le_bytes(pair[8..16].try_into().ok()?);
        if angle.is_nan() || speed.is_nan() {
            return None;
        }
        states.push((angle, speed));
    }
    Some(NtValue::SwerveModuleStates(states))
}

/// Upper bound on decoded swerve modules (see
/// [`decode_swerve_module_states`]).
const MAX_SWERVE_MODULES: usize = 8;

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

    // ---- struct decoding expansion (ROADMAP 6) -----------------------

    fn pack3(a: f64, b: f64, c: f64) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&a.to_le_bytes());
        v.extend_from_slice(&b.to_le_bytes());
        v.extend_from_slice(&c.to_le_bytes());
        v
    }

    #[test]
    fn chassis_speeds_decodes() {
        let b = pack3(1.5, -0.5, 0.25);
        let v = decode_chassis_speeds(&b, None).unwrap();
        assert_eq!(
            v,
            NtValue::ChassisSpeeds {
                vx: 1.5,
                vy: -0.5,
                omega: 0.25
            }
        );
    }

    #[test]
    fn chassis_speeds_accepts_canonical_schema_and_refuses_unknown() {
        let b = pack3(0.0, 0.0, 0.0);
        assert!(decode_chassis_speeds(
            &b,
            Some("ChassisSpeeds{vx:double, vy:double, omega:double}")
        )
        .is_some());
        assert!(
            decode_chassis_speeds(&b, Some("Pose2d{Translation2d{x:double, y:double}}")).is_none()
        );
    }

    #[test]
    fn chassis_speeds_refuses_short_and_nan() {
        assert!(decode_chassis_speeds(&[0u8; 23], None).is_none());
        let b = pack3(f64::NAN, 0.0, 0.0);
        assert!(decode_chassis_speeds(&b, None).is_none());
    }

    #[test]
    fn twist2d_decodes() {
        let b = pack3(0.4, -1.25, 3.5);
        let v = decode_twist2d(&b, None).unwrap();
        assert_eq!(
            v,
            NtValue::Twist2d {
                dx: 0.4,
                dy: -1.25,
                dtheta: 3.5
            }
        );
        // Short payload / NaN / wrong schema degrade to raw bytes.
        assert!(decode_twist2d(&[0u8; 16], None).is_none());
        assert!(decode_twist2d(&pack3(0.0, f64::NAN, 0.0), None).is_none());
        assert!(decode_twist2d(
            &pack3(0.0, 0.0, 0.0),
            Some("Twist2d{dx:double, other:double}")
        )
        .is_none());
    }

    #[test]
    fn swerve_module_states_decode() {
        // Two modules: (angle, speed) pairs.
        let mut b = Vec::new();
        for (a, s) in [(0.25f64, 2.5f64), (-1.0, 4.0)] {
            b.extend_from_slice(&a.to_le_bytes());
            b.extend_from_slice(&s.to_le_bytes());
        }
        let v = decode_swerve_module_states(&b).unwrap();
        assert_eq!(
            v,
            NtValue::SwerveModuleStates(vec![(0.25, 2.5), (-1.0, 4.0)])
        );
    }

    #[test]
    fn swerve_module_states_refuse_malformed_payloads() {
        // Not a multiple of 16.
        assert!(decode_swerve_module_states(&[0u8; 24]).is_none());
        // Zero modules (empty).
        assert!(decode_swerve_module_states(&[]).is_none());
        // Over the 8-module cap: a real swerve drive has 4.
        assert!(decode_swerve_module_states(&[0u8; 9 * 16]).is_none());
        // NaN anywhere in the payload refuses the WHOLE value.
        let mut b = vec![0u8; 4 * 16];
        b[40..48].copy_from_slice(&f64::NAN.to_le_bytes());
        assert!(decode_swerve_module_states(&b).is_none());
        // Exactly 8 modules (128 bytes) is the accepted maximum.
        assert!(decode_swerve_module_states(&[0u8; 8 * 16]).is_some());
    }
}
