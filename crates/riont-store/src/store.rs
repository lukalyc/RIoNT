//! In-memory topic store: names, metadata, live values, timing/Hz.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Type of a topic's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NtType {
    Boolean,
    Double,
    Int,
    Str,
    BooleanArray,
    DoubleArray,
    IntArray,
    StringArray,
    Json,
    Raw,
    /// Decoded WPILib `struct:Pose2d` (see pose::decode_pose2d).
    Pose2d,
    /// Decoded WPILib `struct:ChassisSpeeds` (see pose::decode_chassis_speeds).
    ChassisSpeeds,
    /// Decoded WPILib `struct:Twist2d` (see pose::decode_twist2d).
    Twist2d,
    /// Decoded WPILib `struct:SwerveModuleStates`
    /// (see pose::decode_swerve_module_states).
    SwerveModuleStates,
    Unknown,
}

impl NtType {
    pub fn parse(s: &str) -> NtType {
        match s {
            "boolean" => NtType::Boolean,
            "double" => NtType::Double,
            "int" => NtType::Int,
            "string" => NtType::Str,
            "boolean[]" => NtType::BooleanArray,
            "double[]" => NtType::DoubleArray,
            "int[]" => NtType::IntArray,
            "string[]" => NtType::StringArray,
            "json" => NtType::Json,
            "raw" | "msgpack" => NtType::Raw,
            "struct:Pose2d" | "pose2d" => NtType::Pose2d,
            "struct:ChassisSpeeds" | "chassis_speeds" => NtType::ChassisSpeeds,
            "struct:Twist2d" | "twist2d" => NtType::Twist2d,
            "struct:SwerveModuleStates" | "swerve_modules" => NtType::SwerveModuleStates,
            _ => NtType::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            NtType::Boolean => "boolean",
            NtType::Double => "double",
            NtType::Int => "int",
            NtType::Str => "string",
            NtType::BooleanArray => "boolean[]",
            NtType::DoubleArray => "double[]",
            NtType::IntArray => "int[]",
            NtType::StringArray => "string[]",
            NtType::Json => "json",
            NtType::Raw => "raw",
            NtType::Pose2d => "pose2d",
            NtType::ChassisSpeeds => "chassis_speeds",
            NtType::Twist2d => "twist2d",
            NtType::SwerveModuleStates => "swerve_modules",
            NtType::Unknown => "?",
        }
    }

    /// Human readable scalar type a user could type into the edit prompt.
    pub fn is_writable(&self) -> bool {
        matches!(
            self,
            NtType::Boolean | NtType::Double | NtType::Int | NtType::Str
        )
    }
}

/// A live NetworkTables value.
#[derive(Clone, Debug, PartialEq)]
pub enum NtValue {
    Boolean(bool),
    Double(f64),
    Int(i64),
    Str(String),
    BooleanArray(Vec<bool>),
    DoubleArray(Vec<f64>),
    IntArray(Vec<i64>),
    StringArray(Vec<String>),
    Json(String),
    Raw(Vec<u8>),
    /// Decoded WPILib Pose2d: meters + radians, blue-alliance origin.
    Pose2d {
        x: f64,
        y: f64,
        radians: f64,
    },
    /// Decoded WPILib ChassisSpeeds: m/s body-frame velocities + rad/s.
    ChassisSpeeds {
        vx: f64,
        vy: f64,
        omega: f64,
    },
    /// Decoded WPILib Twist2d: m + m + radians.
    Twist2d {
        dx: f64,
        dy: f64,
        dtheta: f64,
    },
    /// Decoded WPILib SwerveModuleStates: (angle_radians, speed_mps) per
    /// module, in WPILib module order (FL, FR, RL, RR), capped at 8.
    SwerveModuleStates(Vec<(f64, f64)>),
}

impl NtValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            NtValue::Boolean(_) => "boolean",
            NtValue::Double(_) => "double",
            NtValue::Int(_) => "int",
            NtValue::Str(_) => "string",
            NtValue::BooleanArray(_) => "boolean[]",
            NtValue::DoubleArray(_) => "double[]",
            NtValue::IntArray(_) => "int[]",
            NtValue::StringArray(_) => "string[]",
            NtValue::Json(_) => "json",
            NtValue::Raw(_) => "raw",
            NtValue::Pose2d { .. } => "pose2d",
            NtValue::ChassisSpeeds { .. } => "chassis_speeds",
            NtValue::Twist2d { .. } => "twist2d",
            NtValue::SwerveModuleStates(_) => "swerve_modules",
        }
    }

    /// Numeric-tolerant equality for publish verification (read-back):
    /// doubles compare with a small relative epsilon (a server may round a
    /// written value through f32 on its way back); everything else compares
    /// exactly.
    pub fn approx_eq(&self, other: &NtValue) -> bool {
        const REL_EPS: f64 = 1e-6;
        match (self, other) {
            (NtValue::Double(a), NtValue::Double(b)) => {
                (a - b).abs() <= REL_EPS * a.abs().max(b.abs()).max(1.0)
            }
            (NtValue::DoubleArray(a), NtValue::DoubleArray(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| (x - y).abs() <= REL_EPS * x.abs().max(y.abs()).max(1.0))
            }
            (a, b) => a == b,
        }
    }

    /// Compact single-line rendering for the tree/inspector.
    pub fn format(&self) -> String {
        match self {
            NtValue::Boolean(b) => b.to_string(),
            NtValue::Double(f) => {
                if f.abs() >= 1_000_000.0 || (*f != 0.0 && f.abs() < 0.001) {
                    format!("{:.3e}", f)
                } else {
                    format!("{:.4}", f)
                }
            }
            NtValue::Int(i) => i.to_string(),
            NtValue::Str(s) => s.clone(),
            NtValue::BooleanArray(v) => {
                format!(
                    "[{}]",
                    v.iter()
                        .map(|b| if *b { "T" } else { "F" })
                        .collect::<String>()
                )
            }
            NtValue::DoubleArray(v) => format!(
                "[{}]",
                v.iter()
                    .take(8)
                    .map(|f| format!("{:.3}", f))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            NtValue::IntArray(v) => format!(
                "[{}]",
                v.iter()
                    .take(8)
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            NtValue::StringArray(v) => format!("[{}]", v.join(", ")),
            NtValue::Json(s) => s.clone(),
            NtValue::Raw(b) => format!("<{} bytes>", b.len()),
            NtValue::Pose2d { x, y, radians } => format!(
                "({:.2} m, {:.2} m, {:.1}\u{00b0})",
                x,
                y,
                radians.to_degrees()
            ),
            NtValue::ChassisSpeeds { vx, vy, omega } => format!(
                "vx {:.2} m/s  vy {:.2} m/s  \u{03c9} {:.2} rad/s",
                vx, vy, omega
            ),
            NtValue::Twist2d { dx, dy, dtheta } => format!(
                "dx {:.2} m  dy {:.2} m  d\u{03b8} {:.2} rad",
                dx, dy, dtheta
            ),
            NtValue::SwerveModuleStates(states) => format!(
                "[{}]",
                states
                    .iter()
                    .map(|(a, s)| format!("{:.2}rad {:.2}m/s", a, s))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// One topic + its live stream metadata.
#[derive(Debug)]
pub struct TopicData {
    pub name: String,
    /// Server-assigned topic id (from SetTopic), 0 until known.
    pub id: u64,
    pub data_type: NtType,
    pub type_str: Option<String>,
    /// Advertised `structSchema` announce property (needed to decode
    /// struct-typed raw payloads; see pose::decode_pose2d).
    pub struct_schema: Option<String>,
    pub persistent: bool,
    pub retained: bool,
    pub current: Option<NtValue>,
    pub last_update: Option<Instant>,
    pub last_server_ts: Option<u64>,
    /// Recent updates `(local_receipt_instant, server_ts_us)` for Hz estimation.
    hz_samples: VecDeque<(Instant, u64)>,
    /// Pose trail for field rendering: (x_m, y_m, server_ts_us). Only
    /// populated for topics that classify as a robot pose (pose::classify).
    pose_trail: VecDeque<(f64, f64, u64)>,
    /// Sticky pose marker: set the first time this topic classifies as a
    /// robot pose and never cleared while the topic exists. Pose sources
    /// legitimately publish NON-pose values between estimates (Limelight
    /// sends an empty array with no target) — without stickiness the field
    /// card would flicker on/off every update cycle.
    pose_sticky: bool,
}

/// Active sliding window for fast/normal streams (>= 0.5 Hz).
const HZ_WINDOW: Duration = Duration::from_secs(2);
/// Maximum retained interval for slow topics (0.1–0.5 Hz).
const HZ_SLOW_WINDOW: Duration = Duration::from_secs(10);
/// Minimum silence grace before decaying (absorbs 50 ms NT4 flush batching + jitter).
const HZ_GRACE_MIN: f64 = 0.25;
/// Minimum span required to divide by an interval (guards against micro-burst spikes).
const HZ_MIN_SPAN: f64 = 0.001;
/// Max allowed delta between server-timestamp span and local span before falling back
/// to local time (guards against stale initial-value timestamps, clock jumps, or local echoes).
const HZ_SERVER_TOLERANCE: f64 = 0.15;
/// Pose trail caps: 300 points or 10 s of server time, whichever first.
const POSE_TRAIL_CAP: usize = 300;
const POSE_TRAIL_WINDOW_US: u64 = 10_000_000;

impl TopicData {
    fn new(name: String) -> Self {
        TopicData {
            name,
            id: 0,
            data_type: NtType::Unknown,
            type_str: None,
            struct_schema: None,
            persistent: false,
            retained: false,
            current: None,
            last_update: None,
            last_server_ts: None,
            hz_samples: VecDeque::new(),
            pose_trail: VecDeque::new(),
            pose_sticky: false,
        }
    }

    fn apply(&mut self, v: NtValue, server_ts: u64, now: Instant) {
        // Struct-aware decode: known WPILib struct topics' binary
        // payloads become real typed values; unknown struct types stay
        // raw — never guess a schema we were not told about.
        let decoded = match (&v, self.type_str.as_deref()) {
            (NtValue::Raw(bytes), Some("struct:Pose2d")) => {
                crate::pose::decode_pose2d(bytes, self.struct_schema.as_deref())
            }
            (NtValue::Raw(bytes), Some("struct:ChassisSpeeds")) => {
                crate::pose::decode_chassis_speeds(bytes, self.struct_schema.as_deref())
            }
            (NtValue::Raw(bytes), Some("struct:Twist2d")) => {
                crate::pose::decode_twist2d(bytes, self.struct_schema.as_deref())
            }
            (NtValue::Raw(bytes), Some("struct:SwerveModuleStates")) => {
                crate::pose::decode_swerve_module_states(bytes)
            }
            _ => None,
        };
        let v = decoded.unwrap_or(v);
        self.current = Some(v.clone());
        self.last_update = Some(now);
        self.last_server_ts = Some(server_ts);
        self.data_type = match NtType::parse(v.type_name()) {
            NtType::Unknown => self.data_type,
            t => t,
        };
        self.hz_samples.push_back((now, server_ts));
        let fast_cutoff = now.checked_sub(HZ_WINDOW);
        let slow_cutoff = now.checked_sub(HZ_SLOW_WINDOW);
        while self.hz_samples.len() > 2
            && fast_cutoff.is_some_and(|fc| self.hz_samples.front().is_some_and(|(t, _)| *t < fc))
        {
            self.hz_samples.pop_front();
        }
        if let Some(sc) = slow_cutoff {
            while self.hz_samples.len() > 1 && self.hz_samples.front().is_some_and(|(t, _)| *t < sc)
            {
                self.hz_samples.pop_front();
            }
        }
        // Pose trail: only for values that classify (cheap exact-name/type
        // check); non-pose topics never allocate trail entries. The FIRST
        // classified value marks the topic as a pose source for good — see
        // pose_sticky.
        if let Some(p) = crate::pose::classify(&self.name, self.type_str.as_deref(), &v) {
            self.pose_sticky = true;
            self.pose_trail.push_back((p.x, p.y, server_ts));
            while self.pose_trail.len() > POSE_TRAIL_CAP {
                self.pose_trail.pop_front();
            }
            while self
                .pose_trail
                .front()
                .is_some_and(|(_, _, ts)| *ts + POSE_TRAIL_WINDOW_US <= server_ts)
            {
                self.pose_trail.pop_front();
            }
        }
    }

    /// Pose trail for field rendering (may be empty).
    pub fn pose_trail(&self) -> &VecDeque<(f64, f64, u64)> {
        &self.pose_trail
    }

    /// True once this topic has EVER classified as a robot pose. See
    /// pose_sticky: empty/no-estimate updates must not un-field a card.
    pub fn is_pose_source(&self) -> bool {
        self.pose_sticky
    }

    /// Continuous interval-based publish rate in Hz (uses a 2 s active
    /// window for normal/fast streams and retains the last pair up to 10 s
    /// for slow topics down to 0.1 Hz). Returns `None` if never updated, and
    /// `Some(0.0)` for single-sample topics or once updates stop.
    pub fn hz(&self) -> Option<f64> {
        self.hz_at(Instant::now())
    }

    /// Publish rate in Hz ending at `now`.
    /// Returns `None` if never updated, `Some(0.0)` when only a single sample
    /// exists or updates have stopped, and a continuous interval-based rate
    /// (`(n - 1) / span`) while updating — using server timestamps when they
    /// agree with local arrival times (eliminating 50 ms batch quantization).
    pub fn hz_at(&self, now: Instant) -> Option<f64> {
        self.last_update?;
        let fast_cutoff = now.checked_sub(HZ_WINDOW);

        let mut fast_count: usize = 0;
        let mut fast_first: Option<(Instant, u64)> = None;
        let mut fast_last: Option<(Instant, u64)> = None;
        let mut fast_server_monotonic = true;
        let mut prev_overall: Option<(Instant, u64)> = None;
        let mut last_overall: Option<(Instant, u64)> = None;

        for &(t, s) in &self.hz_samples {
            if t > now {
                break;
            }
            prev_overall = last_overall;
            last_overall = Some((t, s));
            if fast_cutoff.is_none_or(|fc| t >= fc) {
                if let Some((_, prev_s)) = fast_last {
                    if s < prev_s {
                        fast_server_monotonic = false;
                    }
                } else {
                    fast_first = Some((t, s));
                }
                fast_last = Some((t, s));
                fast_count += 1;
            }
        }

        if fast_count >= 2 {
            let (t0, s0) = fast_first?;
            let (t1, s1) = fast_last?;
            let local_span = t1.duration_since(t0).as_secs_f64();
            let span = best_span(local_span, s0, s1, fast_server_monotonic);
            if span < HZ_MIN_SPAN {
                return Some(fast_count as f64 / HZ_WINDOW.as_secs_f64());
            }
            let intervals = (fast_count - 1) as f64;
            let period = span / intervals;
            let age = now.duration_since(t1).as_secs_f64();
            let grace = (period * 1.5).max(HZ_GRACE_MIN);
            let denom = if age <= grace {
                span
            } else {
                span + (age - grace)
            };
            return Some(intervals / denom);
        }

        // Fewer than 2 samples in the 2 s fast window: use the last pair for
        // genuinely slow topics (0.1–1.0 Hz), decaying and timing out once overdue.
        let (Some((t0, s0)), Some((t1, s1))) = (prev_overall, last_overall) else {
            return Some(0.0);
        };
        let local_period = t1.duration_since(t0).as_secs_f64();
        let period = best_span(local_period, s0, s1, s1 >= s0);
        if period < HZ_MIN_SPAN || period > HZ_SLOW_WINDOW.as_secs_f64() {
            return Some(0.0);
        }
        let age = now.duration_since(t1).as_secs_f64();
        let timeout = (period * 2.0)
            .max(HZ_WINDOW.as_secs_f64())
            .min(HZ_SLOW_WINDOW.as_secs_f64());
        if age > timeout {
            return Some(0.0);
        }
        let grace = (period * 1.5).max(HZ_GRACE_MIN);
        let denom = if age <= grace {
            period
        } else {
            period + (age - grace)
        };
        Some(1.0 / denom)
    }

    /// Seconds since last update (or None).
    pub fn age_secs(&self) -> Option<f64> {
        self.last_update.map(|t| t.elapsed().as_secs_f64())
    }
}

fn best_span(local_span: f64, s0: u64, s1: u64, server_monotonic: bool) -> f64 {
    if server_monotonic && s1 > s0 {
        let server_span = (s1 - s0) as f64 / 1_000_000.0;
        if (server_span - local_span).abs() <= HZ_SERVER_TOLERANCE {
            return server_span;
        }
    }
    local_span
}

/// Full topic store owned by the UI side; mutated from client updates.
#[derive(Debug, Default)]
pub struct Store {
    pub topics: HashMap<String, TopicData>,
}

impl Store {
    pub fn new() -> Self {
        Store::default()
    }

    pub fn ensure(&mut self, name: &str) -> &mut TopicData {
        self.topics
            .entry(name.to_string())
            .or_insert_with(|| TopicData::new(name.to_string()))
    }

    pub fn apply_value(&mut self, name: &str, v: NtValue, server_ts: u64, now: Instant) {
        self.ensure(name).apply(v, server_ts, now);
    }

    /// Pose trail samples for `name` (None if the topic was never seen or
    /// never classified as a pose).
    pub fn pose_trail(&self, name: &str) -> Option<&VecDeque<(f64, f64, u64)>> {
        self.topics.get(name).map(|t| t.pose_trail())
    }

    pub fn sorted_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.topics.keys().cloned().collect();
        names.sort_unstable();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_sticky_survives_empty_updates() {
        let now = std::time::Instant::now();
        let mut td = TopicData::new("SmartDashboard/botpose_wpiblue".into());
        td.apply(
            NtValue::DoubleArray(vec![1.0, 2.0, 0.0, 0.0, 0.0, 90.0]),
            1_000,
            now,
        );
        assert!(td.is_pose_source());
        // Camera loses its estimate: Limelight publishes an empty array.
        // The topic must REMAIN a pose source (no card flicker).
        td.apply(NtValue::DoubleArray(vec![]), 2_000, now);
        assert!(td.is_pose_source());
        // A topic that never classified is not sticky.
        let mut other = TopicData::new("SmartDashboard/Battery Voltage".into());
        other.apply(NtValue::Double(12.6), 3_000, now);
        assert!(!other.is_pose_source());
        // A lookalike name with a valid shape IS sticky (conservative
        // classifier still refuses it — this is the exact-name path).
        let mut lookalike = TopicData::new("SmartDashboard/targetpose".into());
        lookalike.apply(
            NtValue::DoubleArray(vec![1.0, 2.0, 0.0, 0.0, 0.0, 45.0]),
            4_000,
            now,
        );
        assert!(!lookalike.is_pose_source());
    }

    #[test]
    fn struct_decoding_expansion_dispatches_on_type_str() {
        let now = std::time::Instant::now();

        // ChassisSpeeds: raw payload decodes into the named-field value.
        let mut td = TopicData::new("Odometry/ChassisSpeeds".into());
        td.type_str = Some("struct:ChassisSpeeds".into());
        let mut b = Vec::new();
        b.extend_from_slice(&1.25f64.to_le_bytes());
        b.extend_from_slice(&0.0f64.to_le_bytes());
        b.extend_from_slice(&0.5f64.to_le_bytes());
        td.apply(NtValue::Raw(b), 1_000, now);
        assert_eq!(
            td.current,
            Some(NtValue::ChassisSpeeds {
                vx: 1.25,
                vy: 0.0,
                omega: 0.5
            })
        );
        assert_eq!(td.data_type, NtType::ChassisSpeeds);

        // Malformed payload degrades to raw bytes (never a guess).
        let mut bad = TopicData::new("Odometry/Bad".into());
        bad.type_str = Some("struct:ChassisSpeeds".into());
        bad.apply(NtValue::Raw(vec![0u8; 23]), 2_000, now);
        assert_eq!(bad.current, Some(NtValue::Raw(vec![0u8; 23])));
        assert_eq!(bad.data_type, NtType::Raw);

        // SwerveModuleStates decode for the field-card vectors.
        let mut td = TopicData::new("Swerve/ModuleStates".into());
        td.type_str = Some("struct:SwerveModuleStates".into());
        let mut b = Vec::new();
        for (a, s) in [(0.1f64, 2.0f64), (0.2, 2.0), (0.3, 2.0), (0.4, 2.0)] {
            b.extend_from_slice(&a.to_le_bytes());
            b.extend_from_slice(&s.to_le_bytes());
        }
        td.apply(NtValue::Raw(b), 3_000, now);
        assert_eq!(
            td.current,
            Some(NtValue::SwerveModuleStates(vec![
                (0.1, 2.0),
                (0.2, 2.0),
                (0.3, 2.0),
                (0.4, 2.0)
            ]))
        );
    }

    #[test]
    fn unknown_struct_type_stays_raw() {
        let now = std::time::Instant::now();
        let mut td = TopicData::new("Struct/Unknown".into());
        td.type_str = Some("struct:SomethingElse".into());
        td.apply(NtValue::Raw(vec![1u8, 2, 3]), 1_000, now);
        assert_eq!(td.current, Some(NtValue::Raw(vec![1u8, 2, 3])));
    }

    #[test]
    fn hz_is_none_before_first_update() {
        let base = Instant::now();
        let td = TopicData::new("SmartDashboard/Unseen".into());
        assert_eq!(td.hz_at(base), None);
    }

    #[test]
    fn hz_single_update_is_zero_and_decays_after_burst() {
        let base = Instant::now();
        let mut td = TopicData::new("SmartDashboard/Static".into());
        td.apply(NtValue::Boolean(false), 1_000, base);

        assert_eq!(td.hz_at(base), Some(0.0));

        // Second update 700 ms later gives ~1.4286 Hz (not snapped to 0.5 Hz steps).
        td.apply(
            NtValue::Boolean(true),
            701_000,
            base + Duration::from_millis(700),
        );
        let rate = td
            .hz_at(base + Duration::from_millis(700))
            .expect("rate present");
        assert!(
            (rate - 1.0 / 0.7).abs() < 1e-6,
            "expected ~1.4286 Hz, got {rate}"
        );

        // Stays steady within grace (1.05s), decays past grace, hits 0.0 past 2.0s.
        let steady = td
            .hz_at(base + Duration::from_millis(1400))
            .expect("steady rate");
        assert!((steady - 1.0 / 0.7).abs() < 1e-6, "got {steady}");

        let decaying = td
            .hz_at(base + Duration::from_millis(2200))
            .expect("decaying rate");
        assert!(
            decaying > 0.0 && decaying < 1.0,
            "expected decay below 1.0 Hz, got {decaying}"
        );

        assert_eq!(td.hz_at(base + Duration::from_millis(2800)), Some(0.0));
    }

    #[test]
    fn hz_resolves_non_half_hz_rates() {
        let base = Instant::now();
        let mut td = TopicData::new("SmartDashboard/Custom".into());
        for i in 0..6u64 {
            let t = base + Duration::from_millis(i * 300);
            td.apply(NtValue::Double(i as f64), (i + 1) * 300_000, t);
        }
        let rate = td
            .hz_at(base + Duration::from_millis(1500))
            .expect("rate present");
        assert!(
            (rate - 1.0 / 0.3).abs() < 1e-6,
            "expected ~3.3333 Hz, got {rate}"
        );
    }

    #[test]
    fn hz_tracks_stream_rate_and_decays_when_stream_stops() {
        let base = Instant::now();
        let mut td = TopicData::new("SmartDashboard/Stream".into());
        for i in 0..40u64 {
            let t = base + Duration::from_millis(i * 50);
            td.apply(NtValue::Double(i as f64), (i + 1) * 50_000, t);
        }

        // 40 samples over 1.95s -> 20.0 Hz.
        let active = td
            .hz_at(base + Duration::from_millis(1950))
            .expect("rate present");
        assert!(
            (active - 20.0).abs() < 1e-6,
            "expected 20.0 Hz, got {active}"
        );

        // 1s after stream stops: decays smoothly.
        let mid = td
            .hz_at(base + Duration::from_millis(2950))
            .expect("rate present");
        assert!(mid > 5.0 && mid < 15.0, "expected ~10 Hz, got {mid}");

        // >2s after the last sample: fully decayed to 0.0 Hz.
        assert_eq!(td.hz_at(base + Duration::from_millis(4050)), Some(0.0));
    }

    #[test]
    fn hz_tracks_slow_topics_below_half_hz_and_expires() {
        let base = Instant::now();
        let mut td = TopicData::new("SmartDashboard/Slow".into());
        td.apply(NtValue::Double(1.0), 1_000_000, base);
        td.apply(
            NtValue::Double(2.0),
            4_000_000,
            base + Duration::from_secs(3),
        );

        let r1 = td.hz_at(base + Duration::from_secs(3)).expect("rate");
        assert!((r1 - 1.0 / 3.0).abs() < 1e-6, "got {r1}");

        // 2.5s after last sample: outside 2s fast window, still within 4.5s grace.
        let r2 = td.hz_at(base + Duration::from_millis(5500)).expect("rate");
        assert!((r2 - 1.0 / 3.0).abs() < 1e-6, "got {r2}");

        // Past 2 * period (6.0s after last sample -> 9.1s): expires to 0.0 Hz.
        assert_eq!(td.hz_at(base + Duration::from_millis(9100)), Some(0.0));
    }

    #[test]
    fn hz_uses_server_timestamps_for_batched_fast_streams() {
        let base = Instant::now();
        let mut td = TopicData::new("SmartDashboard/Fast50".into());
        // 50 Hz source arriving in 50 ms UI flush batches (2 or 3 items sharing `batch_t`).
        for i in 0..100u64 {
            let batch_idx = (i * 20) / 50;
            let batch_t = base + Duration::from_millis(batch_idx * 50);
            td.apply(NtValue::Double(i as f64), 10_000_000 + i * 20_000, batch_t);
        }
        let rate = td
            .hz_at(base + Duration::from_millis(1950))
            .expect("rate present");
        assert!(
            (rate - 50.0).abs() < 0.1,
            "expected ~50.0 Hz from server timestamps, got {rate}"
        );
    }

    #[test]
    fn hz_ignores_stale_initial_server_timestamp() {
        let base = Instant::now();
        let mut td = TopicData::new("SmartDashboard/Jump".into());
        td.apply(NtValue::Double(1.0), 10_000_000, base);
        td.apply(
            NtValue::Double(2.0),
            300_800_000,
            base + Duration::from_millis(800),
        );
        let rate = td
            .hz_at(base + Duration::from_millis(800))
            .expect("rate present");
        assert!(
            (rate - 1.25).abs() < 1e-6,
            "expected 1.25 Hz from local span, got {rate}"
        );
    }
}
