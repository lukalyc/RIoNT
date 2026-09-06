//! Persistent configuration: `~/.config/riont/config.json`.
//!
//! Single source of truth for saved robot targets (the Connection Picker's
//! list), workspace presets, last-connected target, and the SSH settings
//! used by `System: Restart Robot Code`. Edited in-app via the Settings
//! palette commands or directly with `$EDITOR`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Test-only config redirect: THREAD-LOCAL, so every test thread gets its
/// own scratch file — parallel `cargo test` threads never share (and never
/// race on) a config file. Set per-thread by `tests_tui::hermetic_config`.
#[cfg(test)]
thread_local! {
    static TEST_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        std::cell::RefCell::new(None);
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
    pub length_m: f64,
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
    /// Field-card topics joined into ONE composite field card (overlay
    /// groups). Toggled per topic with `o` on a hovered field card;
    /// individual cards remain the default.
    #[serde(default)]
    pub overlay_topics: Vec<String>,
}

fn default_map() -> String {
    "2025-reefscape".into()
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
            overlay_topics: Vec::new(),
        }
    }
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
    /// Workspace presets: name -> topic list. A trailing `/*` subscribes a
    /// whole subtree.
    #[serde(default)]
    pub presets: std::collections::BTreeMap<String, Vec<String>>,
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

    /// Workspace presets in file order (BTreeMap keeps them stable).
    pub fn preset_list(&self) -> Vec<(String, Vec<String>)> {
        self.presets
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}
