//! In-memory topic store: names, metadata, live values, timing/Hz.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

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
    Unknown,
}

impl NtType {
    pub fn from_str(s: &str) -> NtType {
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
    Pose2d { x: f64, y: f64, radians: f64 },
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
                format!("[{}]", v.iter().map(|b| if *b { "T" } else { "F" }).collect::<String>())
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
                v.iter().take(8).map(|i| i.to_string()).collect::<Vec<_>>().join(" ")
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
    /// Recent update instants (pruned to ~2s) for Hz estimation.
    hz_samples: VecDeque<Instant>,
    /// Pose trail for field rendering: (x_m, y_m, server_ts_us). Only
    /// populated for topics that classify as a robot pose (pose::classify).
    pose_trail: VecDeque<(f64, f64, u64)>,
}

const HZ_WINDOW: f64 = 2.0;
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
        }
    }

    fn apply(&mut self, v: NtValue, server_ts: u64, now: Instant) {
        // Struct-aware decode: a `struct:Pose2d` topic's binary payload
        // becomes a real pose value; unknown struct types stay raw — never
        // guess a schema we were not told about.
        let decoded = match &v {
            NtValue::Raw(bytes) if self.type_str.as_deref() == Some("struct:Pose2d") => {
                crate::pose::decode_pose2d(bytes, self.struct_schema.as_deref())
            }
            _ => None,
        };
        let v = decoded.unwrap_or(v);
        self.current = Some(v.clone());
        self.last_update = Some(now);
        self.last_server_ts = Some(server_ts);
        self.data_type = match NtType::from_str(v.type_name()) {
            NtType::Unknown => self.data_type,
            t => t,
        };
        self.hz_samples.push_back(now);
        let cutoff = now - std::time::Duration::from_secs_f64(HZ_WINDOW);
        while self.hz_samples.front().is_some_and(|t| *t < cutoff) {
            self.hz_samples.pop_front();
        }
        // Pose trail: only for values that classify (cheap exact-name/type
        // check); non-pose topics never allocate trail entries.
        if let Some(p) = crate::pose::classify(&self.name, self.type_str.as_deref(), &v) {
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

    /// Publish rate in Hz over the last 2s. None if never updated.
    pub fn hz(&self) -> Option<f64> {
        let n = self.hz_samples.len();
        if n == 0 {
            None
        } else {
            Some(n as f64 / HZ_WINDOW)
        }
    }

    /// Seconds since last update (or None).
    pub fn age_secs(&self) -> Option<f64> {
        self.last_update
            .map(|t| t.elapsed().as_secs_f64())
    }
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
