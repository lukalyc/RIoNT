//! Persistent configuration: `~/.config/riont/config.json`.
//!
//! Single source of truth for saved robot targets (the Connection Picker's
//! list), workspace presets, last-connected target, and the SSH settings
//! used by `System: Restart Robot Code`. Edited in-app via the Settings
//! palette commands or directly with `$EDITOR`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub last_target: Option<String>,
    #[serde(default)]
    pub saved_targets: Vec<SavedTarget>,
    /// Workspace presets: name -> topic list. A trailing `/*` subscribes a
    /// whole subtree.
    #[serde(default)]
    pub presets: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub system: SystemSettings,
}

impl Config {
    /// Default config when no file exists: the two common bench targets.
    pub fn with_defaults() -> Self {
        Config {
            last_target: None,
            saved_targets: vec![
                SavedTarget { name: "Simulation".into(), ip: "127.0.0.1:5810".into() },
                SavedTarget { name: "USB Tether".into(), ip: "172.22.11.2".into() },
            ],
            presets: std::collections::BTreeMap::new(),
            system: SystemSettings::default(),
        }
    }

    /// `~/.config/riont/config.json`
    pub fn path() -> PathBuf {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        std::path::Path::new(&home)
            .join(".config")
            .join("riont")
            .join("config.json")
    }

    /// Load the config; missing file -> defaults. A malformed file also
    /// yields defaults (the caller warns) so a typo can never brick the UI.
    pub fn load() -> (Config, Option<String>) {
        match std::fs::read_to_string(Self::path()) {
            Ok(txt) => match serde_json::from_str(&txt) {
                Ok(c) => (c, None),
                Err(e) => (Config::with_defaults(), Some(e.to_string())),
            },
            Err(_) => (Config::with_defaults(), None),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
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
        self.presets.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}
