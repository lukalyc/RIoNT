//! Persistent configuration: `~/.config/riont/config.json`.
//!
//! Single source of truth for saved robot targets (the Connection Picker's
//! list), workspace presets, last-connected target, and the SSH settings
//! used by `System: Restart Robot Code`. Edited in-app via the Settings
//! palette commands or directly with `$EDITOR`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Test-only config redirect: THREAD-LOCAL, so every test thread gets its
// own scratch file — parallel `cargo test` threads never share (and never
// race on) a config file. Set per-thread by `tests_tui::hermetic_config`.
#[cfg(test)]
thread_local! {
    static TEST_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_path(p: std::path::PathBuf) {
    TEST_PATH.with(|slot| *slot.borrow_mut() = Some(p));
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedTarget {
    pub name: String,
    pub ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemSettings {
    pub ssh_user: String,
    pub restart_cmd: String,
}

impl Default for SystemSettings {
    fn default() -> Self {
        SystemSettings {
            ssh_user: "admin".into(),
            restart_cmd: "/usr/local/frc/bin/frcRunRobot.sh restart".into(),
        }
    }
}

/// Field geometry + user-chosen rendering options for pose visualization.
/// `alliance` flips the rendered field mirror ONLY when the user sets it
/// explicitly — the app never infers an alliance from topic names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldSettings {
    #[serde(default = "default_length_m")]
    pub length_m: f64,
    #[serde(default = "default_width_m")]
    pub width_m: f64,
    pub alliance: String,
    /// Built-in field map name (see field::BUILTIN_MAPS). Cycled by the
    /// palette `Field: Cycle Map` command.
    #[serde(default = "default_map")]
    pub map: String,
    /// External field JSON (PathPlanner format: fieldLength/fieldWidth/
    /// walls, meters, blue origin). When set and parseable it OVERRIDES
    /// the built-in map — THE path for new seasons (scripts/fetch_field.py
    /// writes this). Relative paths resolve against the launch directory.
    #[serde(default)]
    pub walls_file: Option<String>,
    /// Topics the user explicitly marked as robot pose (rendered as a
    /// field card even though the conservative auto-classifier passes on
    /// them). Managed by later user commands; persisted here only.
    #[serde(default)]
    pub force_pose_topics: Vec<String>,
    /// Auto-detected pose topics explicitly collapsed to compact numeric cards
    /// (`v` in the watchlist or `Field: Toggle Pose View on Active Card`).
    #[serde(default)]
    pub disable_pose_topics: Vec<String>,
    /// Field-card topics joined into ONE composite field card (overlay
    /// groups). Toggled per topic with `o` on a hovered field card;
    /// individual cards remain the default.
    #[serde(default)]
    pub overlay_topics: Vec<String>,
    /// Robot footprint in meters (length along the heading, width
    /// across), drawn as a rectangle with a center orientation arrow on
    /// field cards. Bumpers included if you want the true footprint.
    #[serde(default = "default_robot_length_m")]
    pub robot_length_m: f64,
    #[serde(default = "default_robot_width_m")]
    pub robot_width_m: f64,
}

fn default_length_m() -> f64 {
    16.54
}

fn default_width_m() -> f64 {
    8.21
}

fn default_map() -> String {
    "2026-rebuilt".into()
}

fn default_robot_length_m() -> f64 {
    0.9
}

fn default_robot_width_m() -> f64 {
    0.9
}

impl Default for FieldSettings {
    fn default() -> Self {
        FieldSettings {
            // 2025 REEFSCAPE: 16.54 m x 8.21 m, blue origin at the blue
            // driver-station wall, +x into the field.
            length_m: 16.54,
            width_m: 8.21,
            alliance: "blue".into(),
            map: default_map(),
            walls_file: None,
            force_pose_topics: Vec::new(),
            disable_pose_topics: Vec::new(),
            overlay_topics: Vec::new(),
            robot_length_m: default_robot_length_m(),
            robot_width_m: default_robot_width_m(),
        }
    }
}

/// One numbered workspace preset slot (`1..=9`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PresetSlot {
    /// Optional short label shown when loaded or hovered (`[1:bringup]`).
    #[serde(default)]
    pub name: String,
    /// Pinned topics (`prefix/*` for subtree globs).
    #[serde(default)]
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub last_target: Option<String>,
    /// The last live watchlist, persisted after every mutation so a
    /// restart/quit never loses the operator's view. Same format as
    /// presets: plain topic path, or `prefix/*` for a subtree (glob) pin.
    #[serde(default)]
    pub last_view: Vec<String>,
    #[serde(default)]
    pub saved_targets: Vec<SavedTarget>,
    /// Legacy name-keyed workspace presets (`name -> topic list`), retained for
    /// backward compatibility with existing config files and migrated on write.
    #[serde(default)]
    pub presets: std::collections::BTreeMap<String, Vec<String>>,
    /// Numbered workspace preset slots (`1..=9`).
    #[serde(default)]
    pub preset_slots: std::collections::BTreeMap<u8, PresetSlot>,
    #[serde(default)]
    pub system: SystemSettings,
    #[serde(default)]
    pub field: FieldSettings,
    /// Set by `load()` when the on-disk file failed to parse: the in-memory
    /// contents are then DEFAULTS, and a later `save()` must not silently
    /// overwrite the user's (fixable) file with them — see `save()`.
    #[serde(skip)]
    pub was_corrupt: bool,
}

impl Config {
    /// Default config when no file exists: the two common bench targets.
    pub fn with_defaults() -> Self {
        Config {
            last_target: None,
            last_view: Vec::new(),
            saved_targets: vec![
                SavedTarget {
                    name: "Simulation".into(),
                    ip: "127.0.0.1:5810".into(),
                },
                SavedTarget {
                    name: "USB Tether".into(),
                    ip: "172.22.11.2".into(),
                },
            ],
            presets: std::collections::BTreeMap::new(),
            preset_slots: std::collections::BTreeMap::new(),
            system: SystemSettings::default(),
            field: FieldSettings::default(),
            was_corrupt: false,
        }
    }

    /// `~/.config/riont/config.json`, or the path in `RIONT_CONFIG` when
    /// set (hermetic runs, portable installs). Under `cfg(test)` the
    /// thread-local scratch path wins over everything (see
    /// `Config::set_test_path`).
    pub fn path() -> PathBuf {
        #[cfg(test)]
        {
            let override_path = TEST_PATH.with(|slot| slot.borrow().clone());
            if let Some(p) = override_path {
                return p;
            }
        }
        if let Ok(p) = std::env::var("RIONT_CONFIG") {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        std::path::Path::new(&home)
            .join(".config")
            .join("riont")
            .join("config.json")
    }

    /// Load the config; missing file -> defaults. A malformed file also
    /// yields defaults (the caller warns) so a typo can never brick the UI —
    /// but it is flagged `was_corrupt` so `save()` protects the file.
    pub fn load() -> (Config, Option<String>) {
        match std::fs::read_to_string(Self::path()) {
            Ok(txt) => match serde_json::from_str(&txt) {
                Ok(c) => (c, None),
                Err(e) => {
                    let mut c = Config::with_defaults();
                    c.was_corrupt = true;
                    (c, Some(e.to_string()))
                }
            },
            Err(_) => (Config::with_defaults(), None),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        // Data-loss guard: if this in-memory config came from the
        // defaults-on-parse-error path, writing it would replace the
        // user's (fixable) file with defaults. Back the original up to
        // config.json.bak first; if it parses now the user already fixed
        // it in $EDITOR, and a plain overwrite of a valid file is fine.
        if self.was_corrupt {
            if let Ok(txt) = std::fs::read_to_string(&path) {
                if serde_json::from_str::<Config>(&txt).is_err() {
                    let bak = path.with_extension("json.bak");
                    std::fs::copy(&path, &bak).map_err(|e| e.to_string())?;
                }
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let txt = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, txt).map_err(|e| e.to_string())
    }

    /// Saved targets for the picker, most recently used first.
    pub fn picker_targets(&self) -> Vec<SavedTarget> {
        let mut v = self.saved_targets.clone();
        if let Some(last) = &self.last_target {
            if let Some(pos) = v.iter().position(|t| &t.ip == last) {
                let t = v.remove(pos);
                v.insert(0, t);
            }
        }
        v
    }

    /// Resolve numbered preset slots `1..=9` (`0..9` indexed), folding in any
    /// legacy name-keyed `presets` into the first free slots without moving
    /// explicit slot entries.
    pub fn resolved_slots(&self) -> [Option<PresetSlot>; 9] {
        let mut slots: [Option<PresetSlot>; 9] = Default::default();
        for (&slot, ps) in &self.preset_slots {
            if (1..=9).contains(&slot) {
                slots[(slot - 1) as usize] = Some(ps.clone());
            }
        }
        for (name, topics) in &self.presets {
            let already = slots
                .iter()
                .flatten()
                .any(|s| &s.name == name && &s.topics == topics);
            if already {
                continue;
            }
            if let Some(empty) = slots.iter_mut().find(|s| s.is_none()) {
                *empty = Some(PresetSlot {
                    name: name.clone(),
                    topics: topics.clone(),
                });
            }
        }
        slots
    }

    /// Materialize legacy name-keyed `presets` into `preset_slots` before a
    /// mutation so slot indices never shift when new presets are saved.
    pub fn migrate_legacy_presets_to_slots(&mut self) {
        if self.presets.is_empty() {
            return;
        }
        let resolved = self.resolved_slots();
        self.preset_slots.clear();
        for (idx, slot) in resolved.into_iter().enumerate() {
            if let Some(ps) = slot {
                self.preset_slots.insert((idx + 1) as u8, ps);
            }
        }
        self.presets.clear();
    }

    /// Occupied slots in `1..=9` order as `(zero_based_idx, PresetSlot)`.
    pub fn occupied_slots(&self) -> Vec<(usize, PresetSlot)> {
        self.resolved_slots()
            .into_iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.map(|ps| (idx, ps)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_without_field_geometry_uses_defaults() {
        // Older/minimal configs (and the README example) omit length_m /
        // width_m; the whole config must still parse, never fall back.
        let cfg: Config =
            serde_json::from_str(r#"{"last_target": "10.1.18.2", "field": {"alliance": "blue"}}"#)
                .expect("minimal config must parse");
        assert_eq!(cfg.field.length_m, 16.54);
        assert_eq!(cfg.field.width_m, 8.21);
        assert_eq!(cfg.field.robot_length_m, 0.9);
        assert_eq!(cfg.field.robot_width_m, 0.9);
        assert_eq!(cfg.last_target.as_deref(), Some("10.1.18.2"));
    }

    #[test]
    fn robot_footprint_is_configurable() {
        let cfg: Config = serde_json::from_str(
            r#"{"field": {"alliance": "blue", "robot_length_m": 0.78,
                        "robot_width_m": 0.62}}"#,
        )
        .expect("robot footprint keys must parse");
        assert_eq!(cfg.field.robot_length_m, 0.78);
        assert_eq!(cfg.field.robot_width_m, 0.62);
    }

    #[test]
    fn default_field_map_is_2026_and_cycle_wraps() {
        assert_eq!(default_map(), "2026-rebuilt");
        // The palette's Cycle Map steps BUILTIN_MAPS in order and wraps —
        // with 2026 as the default, one cycle must land on 2024.
        let pos = crate::field::BUILTIN_MAPS
            .iter()
            .position(|n| *n == default_map())
            .expect("default map must be a built-in");
        assert_eq!(crate::field::BUILTIN_MAPS[(pos + 1) % 3], "2024-crescendo");
    }

    #[test]
    fn preset_slots_roundtrip_named_and_unnamed() {
        let mut cfg = Config::with_defaults();
        cfg.preset_slots.insert(
            1,
            PresetSlot {
                name: "bringup".into(),
                topics: vec!["Swerve/*".into()],
            },
        );
        cfg.preset_slots.insert(
            4,
            PresetSlot {
                name: String::new(),
                topics: vec!["SmartDashboard/Battery Voltage".into()],
            },
        );
        let json = serde_json::to_string(&cfg).expect("serialize");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize");
        let slots = parsed.resolved_slots();
        assert_eq!(slots[0].as_ref().map(|s| s.name.as_str()), Some("bringup"));
        assert!(slots[1].is_none());
        assert!(slots[2].is_none());
        assert_eq!(slots[3].as_ref().map(|s| s.name.as_str()), Some(""));
        assert_eq!(
            slots[3].as_ref().map(|s| s.topics.as_slice()),
            Some(["SmartDashboard/Battery Voltage".to_string()].as_slice())
        );
    }

    #[test]
    fn legacy_presets_resolve_into_free_slots_without_clobbering_explicit() {
        let mut cfg = Config::with_defaults();
        cfg.preset_slots.insert(
            1,
            PresetSlot {
                name: "bringup".into(),
                topics: vec!["Bringup/*".into()],
            },
        );
        cfg.presets.insert("swerve".into(), vec!["Swerve/*".into()]);
        let slots = cfg.resolved_slots();
        assert_eq!(slots[0].as_ref().map(|s| s.name.as_str()), Some("bringup"));
        assert_eq!(slots[1].as_ref().map(|s| s.name.as_str()), Some("swerve"));
    }

    #[test]
    fn disable_pose_topics_defaults_empty_and_roundtrips() {
        let minimal: Config = serde_json::from_str("{}").expect("deserialize minimal");
        assert!(minimal.field.disable_pose_topics.is_empty());

        let mut cfg = Config::with_defaults();
        cfg.field.disable_pose_topics = vec!["Drive/Pose2d".into()];
        let json = serde_json::to_string(&cfg).expect("serialize");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            parsed.field.disable_pose_topics,
            vec!["Drive/Pose2d".to_string()]
        );
    }
}
