//! Application state + vim-style input handling. Pure logic, no rendering.

use crate::config::SavedTarget;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use riont_nt4::ClientCommand;
use riont_store::store::{NtType, NtValue, Store};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Edit,
    /// Connection Picker: select a saved target or type one, connect.
    /// Zero management options (SRP: management lives in Settings).
    Connect,
    Palette,
    /// Generic single-input prompt (Add Robot Target / Save / Rename Preset).
    Prompt,
    /// List picker: remove a saved robot target.
    PickTarget,
    /// List picker: load a workspace preset.
    PickPreset,
    /// Confirmation dialog before clearing a preset slot.
    ConfirmClearPreset,
    /// Read-only configuration summary.
    SettingsView,
    /// Enlarged field view: near-fullscreen field popup for the hovered
    /// watchlist card. `f` opens (from Watchlist focus on a pose card)
    /// and `f`/Esc closes.
    FieldView,
}

/// Keyboard focus: topic tree, watchlist canvas, or top-right HUD preset bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Watchlist,
    Presets,
}

/// One entry of the watchlist. `Glob` adopts every topic under a prefix
/// (live: new keys appear automatically as the robot publishes them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixSource {
    Topic(String),
    Glob(String),
}

/// Severity of a toast message (drives the toast border/tag color).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

/// A transient, non-blocking notification rendered bottom-right.
#[derive(Debug, Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub msg: String,
    pub born: std::time::Instant,
}

const TOAST_TTL_MS: u128 = 3500;

/// Command palette entries (VS Code-style fuzzy finder, `:` / Ctrl+P).
/// Each entry names its owning subsystem so the palette doubles as a
/// feature map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SettingsOpen,
    SettingsView,
    SettingsAddTarget,
    SettingsRemoveTarget,
    WatchlistSavePreset,
    WatchlistLoadPreset,
    WatchlistClear,
    WatchlistRestorePrevious,
    FieldAllianceBlue,
    FieldAllianceRed,
    FieldTogglePoseView,
    FieldToggleTrail,
    FieldCycleMap,
    ReconnectNt,
    RestartRobotCode,
    CopyTopicPath,
}

pub const COMMANDS: [(Command, &str); 16] = [
    (Command::SettingsOpen, "Settings: Open Configuration"),
    (Command::SettingsView, "Settings: View Settings"),
    (Command::SettingsAddTarget, "Settings: Add Robot Target"),
    (
        Command::SettingsRemoveTarget,
        "Settings: Remove Robot Target",
    ),
    (
        Command::WatchlistSavePreset,
        "Watchlist: Save Active as Preset",
    ),
    (Command::WatchlistLoadPreset, "Watchlist: Load Preset"),
    (Command::WatchlistClear, "Watchlist: Clear All"),
    (
        Command::WatchlistRestorePrevious,
        "Watchlist: Restore Previous",
    ),
    (Command::FieldAllianceBlue, "Field: Set Alliance Blue"),
    (Command::FieldAllianceRed, "Field: Set Alliance Red"),
    (
        Command::FieldTogglePoseView,
        "Field: Toggle Pose View on Active Card",
    ),
    (Command::FieldToggleTrail, "Field: Toggle Trail"),
    (Command::FieldCycleMap, "Field: Cycle Map"),
    (Command::ReconnectNt, "NetworkTables: Reconnect Socket"),
    (Command::RestartRobotCode, "System: Restart Robot Code"),
    (Command::CopyTopicPath, "Copy Active Topic Path"),
];

/// The focused single-input prompts opened from the palette or preset bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    AddTarget,
    SavePreset { slot: usize },
    RenamePreset { slot: usize },
}

/// Outcome of a key press: what the caller (main loop) must do about it.
#[derive(Debug, PartialEq)]
pub enum UiAction {
    Quit,
    None,
    Client(ClientCommand),
    /// Dispatch a background SSH restart of the robot code (product-side
    /// operation; runs in the binary, not the nt4 engine crate).
    RestartRobotCode {
        host: String,
        user: String,
        cmd: String,
    },
    /// Suspend the TUI, open the file in $EDITOR, then resume.
    OpenEditor(std::path::PathBuf),
}

pub struct App {
    pub store: Store,
    pub target: String,

    // connection
    pub connected: bool,
    pub server_info: String,
    /// Local Instant the current connection was established: RUNTIME is
    /// elapsed-since-here. Set on every successful connect, so a reconnect
    /// (including after a robot-code restart drops the link) restarts the
    /// counter from zero.
    pub connected_since: Option<std::time::Instant>,
    /// RUNTIME frozen at the moment the link dropped: while disconnected
    /// the HUD keeps showing the runtime the session reached, instead of
    /// silently ticking on or blanking.
    pub runtime_at_disconnect: Option<std::time::Duration>,

    /// Persistent configuration (~/.config/riont/config.json).
    pub config: crate::config::Config,

    // navigation
    pub expanded: HashSet<String>,
    pub tree_cursor: usize,
    pub focus: Focus,
    /// Pane focused before entering `Focus::Presets` (restored on Esc/Tab/`p`).
    pub prev_focus: Focus,
    /// Highlighted slot `0..9` when `focus == Focus::Presets`.
    pub preset_cursor: usize,
    /// Currently loaded preset slot `0..9`, if any.
    pub active_preset: Option<usize>,
    /// True once the watchlist has diverged from `active_preset`.
    pub preset_dirty: bool,

    // watchlist
    pub watchlist: Vec<MatrixSource>,
    /// Watchlist stashed by the last destructive transition (preset load /
    /// Clear All): one-level undo for `u` and `Watchlist: Restore Previous`.
    pub last_watchlist: Option<Vec<MatrixSource>>,
    /// Active preset stashed alongside `last_watchlist` for `u` restore.
    pub last_active_preset: Option<usize>,
    pub watchlist_cursor: usize,
    /// Scroll offset (card rows) adjusted by the renderer to keep the cursor
    /// visible; clamped defensively here too.
    pub watchlist_scroll: usize,
    /// VERTICAL scroll into the cursor's column: index of the first
    /// rendered card within that column (renderer-owned, mirrors
    /// `watchlist_scroll` — the column scroll is horizontal, this is the
    /// per-column vertical follow). Reset on column change; re-derived
    /// every frame by scroll-into-view.
    pub watchlist_vscroll: usize,

    // modes
    pub mode: Mode,
    pub query: String,
    /// Matched topic NAMES (score-ordered). Names, never indices into a
    /// sorted_names() snapshot: topics announce/unannounce mid-search, and a
    /// stale index would pin or jump to the WRONG topic.
    pub search_matches: Vec<String>,
    pub search_cursor: usize,
    pub edit_topic: Option<String>,
    pub edit_input: String,
    pub edit_error: Option<String>,
    pub connect_input: String,
    /// Connection Picker: highlighted row in the saved-target list.
    pub connect_cursor: usize,
    /// Generic prompt state (mode == Prompt).
    pub prompt_kind: PromptKind,
    pub prompt_input: String,
    pub prompt_error: Option<String>,
    /// Slot pending confirmation in `Mode::ConfirmClearPreset`.
    pub clear_preset_slot: Option<usize>,
    /// Shared cursor for the list pickers (PickTarget / PickPreset).
    pub picker_cursor: usize,
    pub palette_query: String,
    pub palette_matches: Vec<usize>, // indices into COMMANDS
    pub palette_cursor: usize,

    /// Last rendered HUD dimensions (used to size preset name limits dynamically).
    pub hud_width: u16,
    pub hud_left_width: u16,

    // toasts
    pub toasts: Vec<Toast>,

    /// Pose-trail dots on field cards (palette `Field: Toggle Trail`).
    pub show_pose_trail: bool,

    /// Alliance color source: `FMSInfo/IsRedAlliance` (exact topic). None
    /// while the topic is absent — field cards then use a neutral color.
    pub fms_red: Option<bool>,

    /// Topic shown in the enlarged field view (Mode::FieldView).
    pub field_view: Option<String>,

    /// Active field map (built-in or external JSON). Resolved from config
    /// at startup, on `Field: Cycle Map`, and after a config-editor reload;
    /// cached so the 120 Hz render loop never touches the filesystem.
    pub field_map: crate::field::FieldMap,
}

impl App {
    pub fn new(target: String) -> Self {
        let (config, config_err) = crate::config::Config::load();
        let mut app = Self::with_config(target, config);
        if let Some(e) = config_err {
            app.toast(
                ToastKind::Warn,
                format!("config invalid, using defaults ({})", e),
            );
        }
        app
    }

    /// Constructor body shared by `new` (host config) and `new_test`
    /// (in-memory defaults). Keep both behavior-identical apart from the
    /// config source.
    fn with_config(target: String, config: crate::config::Config) -> Self {
        let mut app = App {
            store: Store::new(),
            target,
            connected: false,
            server_info: String::new(),
            connected_since: None,
            runtime_at_disconnect: None,
            config,
            expanded: HashSet::new(),
            tree_cursor: 0,
            focus: Focus::Tree,
            prev_focus: Focus::Tree,
            preset_cursor: 0,
            active_preset: None,
            preset_dirty: false,
            watchlist: Vec::new(),
            last_watchlist: None,
            last_active_preset: None,
            watchlist_cursor: 0,
            watchlist_scroll: 0,
            watchlist_vscroll: 0,
            mode: Mode::Normal,
            query: String::new(),
            search_matches: Vec::new(),
            search_cursor: 0,
            edit_topic: None,
            edit_input: String::new(),
            edit_error: None,
            connect_input: String::new(),
            connect_cursor: 0,
            prompt_kind: PromptKind::AddTarget,
            prompt_input: String::new(),
            prompt_error: None,
            clear_preset_slot: None,
            picker_cursor: 0,
            palette_query: String::new(),
            palette_matches: Vec::new(),
            palette_cursor: 0,
            hud_width: 120,
            hud_left_width: 72,
            toasts: Vec::new(),
            show_pose_trail: true,
            fms_red: None,
            field_view: None,
            field_map: crate::field::FieldMap::builtin("2026-rebuilt")
                .expect("default map is built in"),
        };
        // Resolve the field map from the (possibly just-loaded) config; a
        // broken walls_file falls back to the built-in map with a warning.
        let (map, field_warn) = crate::field::resolve(&app.config);
        app.field_map = map;
        if let Some(w) = field_warn {
            app.toast(ToastKind::Warn, format!("{} — using built-in map", w));
        }
        // Restore the persisted watchlist (same `prefix/*` glob format the
        // presets use) so a restart never loses the operator's view. Globs
        // re-expand live, so cards for topics the robot publishes later are
        // adopted automatically.
        app.watchlist = app
            .config
            .last_view
            .iter()
            .map(|t| match t.strip_suffix("/*") {
                Some(p) => MatrixSource::Glob(p.to_string()),
                None => MatrixSource::Topic(t.clone()),
            })
            .collect();
        if !app.config.last_view.is_empty() {
            let slots = app.preset_slots();
            for (idx, ps) in slots.iter().enumerate() {
                if let Some(ps) = ps {
                    if ps.topics == app.config.last_view {
                        app.active_preset = Some(idx);
                        break;
                    }
                }
            }
        }
        app
    }

    /// Test-only constructor: in-memory default config, never reads or
    /// writes the host's config.json until a test exercises a save path
    /// (and `RIONT_CONFIG` then redirects that to a scratch file — see
    /// `tests_tui.rs`).
    #[cfg(test)]
    pub fn new_test(target: String) -> Self {
        Self::with_config(target, crate::config::Config::with_defaults())
    }

    // ------------------------------------------------------------------
    // Update intake (from client task)
    // ------------------------------------------------------------------

    pub fn apply_values(&mut self, batch: Vec<(String, NtValue, u64)>, now: std::time::Instant) {
        for (name, v, ts) in &batch {
            // Alliance color for field cards: exact FMS topic, per spec —
            // never inferred from pose topic names. None = neutral (bench).
            if name == "FMSInfo/IsRedAlliance" {
                if let NtValue::Boolean(b) = v {
                    self.fms_red = Some(*b);
                }
            }
            self.store.apply_value(name, v.clone(), *ts, now);
        }
    }

    /// Post-publish verification result (see `NtUpdate::PublishVerified`):
    /// the engine read the topic back over a second connection. A matching
    /// value confirms the edit silently — the success toast from the edit
    /// itself is the last word. A different value (the robot overrode the
    /// write, or the server never accepted it) or no read-back at all gets
    /// an honest warning, because the user may act on a value that did not
    /// actually stick.
    pub fn on_publish_verified(
        &mut self,
        topic: &str,
        written: &NtValue,
        actual: &Option<NtValue>,
    ) {
        match actual {
            Some(v) if written.approx_eq(v) => {
                // Authoritative confirmation: reflect the read-back in the
                // store when it is not already present (the live-edit path
                // already applied a local echo; applying again would
                // double-count the edit in Hz and reset Δ, whereas the
                // offline-queued path had no local echo yet).
                let already_current = self
                    .store
                    .topics
                    .get(topic)
                    .and_then(|t| t.current.as_ref())
                    .is_some_and(|cur| cur.approx_eq(v));
                if !already_current {
                    let server_ts = self
                        .store
                        .topics
                        .get(topic)
                        .and_then(|t| t.last_server_ts)
                        .unwrap_or(0);
                    self.store
                        .apply_value(topic, v.clone(), server_ts, std::time::Instant::now());
                }
            }
            Some(v) => {
                // The read-back is authoritative: the robot holds a value
                // different from what we wrote (it overrode the edit, or
                // the write never landed). Reflect it in the store so the
                // tree/inspector stop showing a value the robot does not
                // hold, and warn.
                let server_ts = self
                    .store
                    .topics
                    .get(topic)
                    .and_then(|t| t.last_server_ts)
                    .unwrap_or(0);
                self.store
                    .apply_value(topic, v.clone(), server_ts, std::time::Instant::now());
                self.toast(
                    ToastKind::Warn,
                    format!(
                        "{} = {} — edit not confirmed, robot reads {}",
                        topic,
                        written.format(),
                        v.format()
                    ),
                )
            }
            None => self.toast(
                ToastKind::Warn,
                format!(
                    "{} = {} — edit not confirmed (no read-back)",
                    topic,
                    written.format()
                ),
            ),
        }
    }

    pub fn set_connected(&mut self, info: String) {
        self.connected = true;
        self.server_info = info;
        // RUNTIME restarts from zero on EVERY connect: a robot-code
        // restart drops the link, so a fresh session is a fresh counter.
        self.connected_since = Some(std::time::Instant::now());
        self.runtime_at_disconnect = None;
        // self.toast(ToastKind::Success, format!("connected to {}", self.target));
    }

    /// The link is down: a session died, or the client is already trying
    /// again. Deliberately silent — RIONT auto-reconnects forever, so a drop
    /// is routine operation, not an error the operator must act on; the
    /// HUD's animated `DISCONNECTED` is the whole signal.
    ///
    /// RUNTIME freezes at the value the session reached (and restarts from
    /// zero on reconnect). The freeze is IDEMPOTENT: a retry loop reports
    /// this state every attempt, and recomputing the freeze each time would
    /// walk RUNTIME forward while the link is down.
    pub fn set_disconnected(&mut self) {
        self.connected = false;
        if self.runtime_at_disconnect.is_none() {
            if let Some(since) = self.connected_since {
                self.runtime_at_disconnect = Some(since.elapsed());
            }
        }
    }

    /// Drop toasts past their severity-scaled TTL.
    pub fn prune_toasts(&mut self) {
        self.toasts
            .retain(|t| t.born.elapsed().as_millis() < Self::toast_ttl_ms(t.kind));
    }

    /// Severity-scaled toast lifetimes, anchored on TOAST_TTL_MS: transient
    /// confirmations flash for the base 3.5s, while diagnostics linger —
    /// an error's reason string is the only record of why something failed,
    /// and 3.5s is not long enough to read it (let alone notice it after
    /// glancing back at the robot).
    fn toast_ttl_ms(kind: ToastKind) -> u128 {
        match kind {
            ToastKind::Info | ToastKind::Success => TOAST_TTL_MS,
            ToastKind::Warn => 6_000,
            ToastKind::Error => 10_000,
        }
    }

    pub fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push(Toast {
            kind,
            msg: msg.into(),
            born: std::time::Instant::now(),
        });
        if self.toasts.len() > 4 {
            let excess = self.toasts.len() - 4;
            self.toasts.drain(..excess);
        }
    }

    // ------------------------------------------------------------------
    // Tree model
    // ------------------------------------------------------------------

    /// The path under the current tree cursor: Some(topic name) if the row
    /// is a topic, Some(dir path) for directories, None if out of range.
    pub fn cursor_path(&self) -> Option<(String, bool)> {
        let rows = crate::ui::tree::build_tree(&self.store, &self.expanded);
        rows.get(self.tree_cursor)
            .map(|r| (r.path.clone(), r.is_topic))
    }

    fn expand_ancestors(&mut self, topic: &str) {
        let mut path = String::new();
        for part in topic.split('/').collect::<Vec<_>>().windows(1) {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part[0]);
            self.expanded.insert(path.clone());
        }
    }

    // ------------------------------------------------------------------
    // Watchlist model
    // ------------------------------------------------------------------

    pub fn is_pinned(&self, topic: &str) -> bool {
        self.watchlist.iter().any(|s| match s {
            MatrixSource::Topic(t) => t == topic,
            MatrixSource::Glob(p) => topic == p || topic.starts_with(&format!("{}/", p)),
        })
    }

    /// Toggle the pin state of the tree cursor's item: topics pin
    /// individually, directories pin their whole subtree (glob).
    fn toggle_pin_cursor(&mut self) {
        let Some((path, is_topic)) = self.cursor_path() else {
            return;
        };
        if is_topic {
            if self.is_pinned(&path) {
                self.unpin_topic(&path);
            } else {
                self.watchlist.push(MatrixSource::Topic(path.clone()));
                self.toast(ToastKind::Success, format!("pinned {}", path));
            }
        } else if self.watchlist.contains(&MatrixSource::Glob(path.clone())) {
            self.watchlist
                .retain(|s| s != &MatrixSource::Glob(path.clone()));
            self.toast(ToastKind::Info, format!("unpinned {}/*", path));
        } else {
            self.watchlist.push(MatrixSource::Glob(path.clone()));
            self.toast(ToastKind::Success, format!("pinned {}/* (subtree)", path));
        }
        self.persist_watchlist();
        self.clamp_watchlist_cursor();
    }

    fn unpin_topic(&mut self, topic: &str) {
        // Dismissing a glob-covered card removes the WHOLE Glob source —
        // detect that and say so, instead of a per-topic toast that hides
        // the real blast radius (every card under the prefix vanishes).
        let glob_removed: Vec<String> = self
            .watchlist
            .iter()
            .filter_map(|s| match s {
                MatrixSource::Glob(p) if topic == p || topic.starts_with(&format!("{}/", p)) => {
                    Some(p.clone())
                }
                _ => None,
            })
            .collect();
        self.watchlist.retain(|s| match s {
            MatrixSource::Topic(t) => t != topic,
            MatrixSource::Glob(p) => !(topic == p || topic.starts_with(&format!("{}/", p))),
        });
        if let Some(p) = glob_removed.first() {
            let pfx = format!("{}/", p);
            let cards = self
                .store
                .sorted_names()
                .iter()
                .filter(|n| n.starts_with(&pfx))
                .count();
            self.toast(
                ToastKind::Info,
                format!("unpinned glob {}/* ({} cards removed)", p, cards),
            );
        } else {
            self.toast(ToastKind::Info, format!("unpinned {}", topic));
        }
        self.persist_watchlist();
    }

    /// Current watchlist entries serialized in config/preset format.
    pub fn current_watchlist_strings(&self) -> Vec<String> {
        self.watchlist
            .iter()
            .map(|s| match s {
                MatrixSource::Topic(t) => t.clone(),
                MatrixSource::Glob(p) => format!("{}/*", p),
            })
            .collect()
    }

    fn sync_preset_dirty(&mut self) {
        if self.watchlist.is_empty() {
            self.active_preset = None;
            self.preset_dirty = false;
            return;
        }
        if let Some(idx) = self.active_preset {
            let current = self.current_watchlist_strings();
            let saved = self.preset_slots()[idx].as_ref().map(|s| s.topics.clone());
            self.preset_dirty = saved.as_ref() != Some(&current);
        } else {
            self.preset_dirty = false;
        }
    }

    /// Snapshot the watchlist into `config.last_view` (glob `prefix/*` form)
    /// and save. Called after every mutation so a quit/crash mid-session
    /// never loses the operator's view.
    fn persist_watchlist(&mut self) {
        self.config.last_view = self.current_watchlist_strings();
        self.sync_preset_dirty();
        if let Err(e) = self.config.save() {
            self.toast(ToastKind::Warn, format!("save config: {}", e));
        }
    }

    /// Restore the watchlist stashed by the last destructive transition
    /// (preset load / Clear All). One level only — the slot is consumed.
    fn restore_previous_watchlist(&mut self) {
        match self.last_watchlist.take() {
            Some(prev) => {
                self.watchlist = prev;
                self.active_preset = self.last_active_preset.take();
                self.watchlist_cursor = 0;
                self.watchlist_scroll = 0;
                self.watchlist_vscroll = 0;
                self.clamp_watchlist_cursor();
                self.toast(ToastKind::Success, "watchlist restored");
                self.persist_watchlist();
            }
            None => self.toast(ToastKind::Warn, "nothing to restore"),
        }
    }

    pub fn clamp_watchlist_cursor(&mut self) {
        let n = self.watchlist_cells().len();
        self.watchlist_cursor = self.watchlist_cursor.min(n.saturating_sub(1));
    }

    /// Expand the watchlist sources into the concrete topic list. Globs are
    /// re-expanded on every call, so topics the robot publishes later are
    /// adopted automatically.
    pub fn watchlist_cells(&self) -> Vec<String> {
        // The overlay-collapse filter lives in `watchlist_cells_grouped`;
        // navigation and rendering key off the plain topic list.
        self.watchlist_cells_grouped()
            .into_iter()
            .map(|(c, _)| c)
            .collect()
    }

    /// Pinned topics in display order WITH their folder-pin tag (see
    /// `watchlist_cells_raw`), overlay groups collapsed. Drives the
    /// watchlist renderer's group columns.
    pub fn watchlist_cells_grouped(&self) -> Vec<(String, Option<String>)> {
        let raw = self.watchlist_cells_raw();
        let mut overlay_seen = false;
        raw.into_iter()
            .filter(|(c, _)| {
                if self.is_field_card(c) && self.config.field.overlay_topics.iter().any(|t| t == c)
                {
                    if overlay_seen {
                        return false;
                    }
                    overlay_seen = true;
                }
                true
            })
            .collect()
    }

    /// Pinned topics in display order WITHOUT overlay collapsing, each
    /// tagged with the folder pin (glob prefix) it came from — None for
    /// explicitly pinned topics. The grouping drives the watchlist's
    /// per-folder columns and headers.
    fn watchlist_cells_raw(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for src in &self.watchlist {
            match src {
                MatrixSource::Topic(t) => {
                    if seen.insert(t.clone()) {
                        out.push((t.clone(), None));
                    }
                }
                MatrixSource::Glob(prefix) => {
                    let pfx = format!("{}/", prefix);
                    for n in self.store.sorted_names() {
                        if n.starts_with(&pfx) && seen.insert(n.clone()) {
                            out.push((n, Some(prefix.clone())));
                        }
                    }
                }
            }
        }
        out
    }

    /// The topics drawn together on the field card that `topic` belongs
    /// to: just itself, unless it is a member of an overlay group with
    /// more field-card members.
    pub fn field_members(&self, topic: &str) -> Vec<String> {
        if !self.is_field_card(topic) {
            return Vec::new();
        }
        let member = self.config.field.overlay_topics.iter().any(|t| t == topic);
        if !member {
            return vec![topic.to_string()];
        }
        self.watchlist_cells_raw()
            .into_iter()
            .map(|(c, _)| c)
            .filter(|c| {
                self.is_field_card(c) && self.config.field.overlay_topics.iter().any(|t| t == c)
            })
            .collect()
    }

    /// Terminal geometry approximation of the watchlist canvas, shared with
    /// the renderer via `ui::watch_columns` so navigation and painting agree.
    fn watch_cols_layout(&self) -> Vec<(usize, usize)> {
        let (w, h) = crossterm::terminal::size().unwrap_or((120, 36));
        // HUD(1) + status bar(1) + canvas borders(2) + a row of margin.
        let avail_h = h.saturating_sub(5);
        let avail_w = (w * 65 / 100).saturating_sub(2);
        crate::ui::watch_columns(self, avail_h, avail_w)
            .into_iter()
            .map(|c| (c.start, c.count))
            .collect()
    }

    /// The topic the current focus points at (tree cursor or watchlist card).
    pub fn active_topic(&self) -> Option<String> {
        match self.focus {
            Focus::Tree | Focus::Presets => {
                self.cursor_path()
                    .and_then(|(p, is_topic)| if is_topic { Some(p) } else { None })
            }
            Focus::Watchlist => self.watchlist_cells().get(self.watchlist_cursor).cloned(),
        }
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) -> UiAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Ctrl-C carries shell "cancel" muscle memory: in text-entry
            // modes it cancels the input exactly like Esc (a stray Ctrl-C
            // mid-edit must not quit and lose unpinned work); outside them
            // it quits.
            return match self.mode {
                Mode::Search
                | Mode::Edit
                | Mode::Connect
                | Mode::Palette
                | Mode::Prompt
                | Mode::ConfirmClearPreset => {
                    self.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
                }
                // FieldView is a passive readout: Ctrl-C closes it like
                // Esc rather than quitting the app mid-inspection.
                Mode::FieldView => {
                    self.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
                }
                Mode::Normal | Mode::PickTarget | Mode::PickPreset | Mode::SettingsView => {
                    UiAction::Quit
                }
            };
        }

        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Search => self.handle_search(key),
            Mode::Edit => self.handle_edit(key),
            Mode::Connect => self.handle_connect(key),
            Mode::Palette => self.handle_palette(key),
            Mode::Prompt => self.handle_prompt(key),
            Mode::PickTarget => self.handle_pick_target(key),
            Mode::PickPreset => self.handle_pick_preset(key),
            Mode::ConfirmClearPreset => self.handle_confirm_clear_preset(key),
            Mode::SettingsView => self.handle_settings_view(key),
            Mode::FieldView => self.handle_field_view(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Char('q') => return UiAction::Quit,
            // Global commands available from either focus.
            KeyCode::Char(':') => {
                self.open_palette();
                return UiAction::None;
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_palette();
                return UiAction::None;
            }
            KeyCode::Char('p') => {
                if self.focus == Focus::Presets {
                    self.focus = self.prev_focus;
                } else {
                    self.prev_focus = self.focus;
                    self.focus = Focus::Presets;
                    if let Some(a) = self.active_preset {
                        self.preset_cursor = a.min(8);
                    }
                }
                return UiAction::None;
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.query.clear();
                self.search_matches.clear();
                self.search_cursor = 0;
                // Seed with every topic path.
                self.recompute_matches();
                return UiAction::None;
            }
            KeyCode::Char('c') => {
                // Connection Picker: select and connect only.
                self.mode = Mode::Connect;
                self.connect_input.clear();
                self.connect_cursor = 0;
                return UiAction::None;
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                // Workspace presets: 1-9 load numbered preset slots.
                self.apply_preset(c.to_digit(10).unwrap() as usize - 1);
                return UiAction::None;
            }
            KeyCode::Char('u') => {
                // One-level undo for the last destructive watchlist
                // transition (preset load / Clear All). Global, like the
                // preset digits, so it works from either pane.
                self.restore_previous_watchlist();
                return UiAction::None;
            }
            _ => {}
        }
        match self.focus {
            Focus::Tree => self.handle_tree_key(key),
            Focus::Watchlist => self.handle_watchlist_key(key),
            Focus::Presets => self.handle_presets_key(key),
        }
    }

    fn handle_presets_key(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => {
                self.focus = self.prev_focus;
            }
            KeyCode::Char('k') | KeyCode::Right | KeyCode::Char('l') => {
                self.preset_cursor = (self.preset_cursor + 1).min(8);
            }
            KeyCode::Char('j') | KeyCode::Left | KeyCode::Char('h') => {
                self.preset_cursor = self.preset_cursor.saturating_sub(1);
            }
            KeyCode::Char('g') => {
                self.preset_cursor = 0;
            }
            KeyCode::Char('G') => {
                self.preset_cursor = 8;
            }
            KeyCode::Char(' ') => {
                self.open_save_preset(self.preset_cursor);
            }
            KeyCode::Enter => {
                if self.preset_slots()[self.preset_cursor].is_some() {
                    self.apply_preset(self.preset_cursor);
                } else {
                    self.open_save_preset(self.preset_cursor);
                }
            }
            KeyCode::Char('r') => {
                self.open_rename_preset(self.preset_cursor);
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if self.preset_slots()[self.preset_cursor].is_some() {
                    self.clear_preset_slot = Some(self.preset_cursor);
                    self.mode = Mode::ConfirmClearPreset;
                } else {
                    self.toast(
                        ToastKind::Info,
                        format!("preset slot {} is empty", self.preset_cursor + 1),
                    );
                }
            }
            _ => {}
        }
        UiAction::None
    }

    fn open_palette(&mut self) {
        self.mode = Mode::Palette;
        self.palette_query.clear();
        self.palette_cursor = 0;
        self.recompute_palette();
    }

    fn handle_tree_key(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let n = crate::ui::tree::build_tree(&self.store, &self.expanded).len();
                if self.tree_cursor + 1 < n {
                    self.tree_cursor += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tree_cursor = self.tree_cursor.saturating_sub(1);
            }
            KeyCode::Char('g') => self.tree_cursor = 0,
            KeyCode::Char('G') => {
                let n = crate::ui::tree::build_tree(&self.store, &self.expanded).len();
                self.tree_cursor = n.saturating_sub(1);
            }

            // h toggles: collapsed dir unfolds, expanded dir folds. l and
            // Left/Right are aliases so the fold direction never has to be
            // memorized.
            KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Left | KeyCode::Right => {
                self.toggle_fold();
            }

            // Enter edits the cursor topic, as the README keymap documents
            // and the watchlist pane already does; on a directory it folds
            // like h/l.
            KeyCode::Enter => {
                if let Some((path, is_topic)) = self.cursor_path() {
                    if is_topic {
                        if self.is_writable(&path) {
                            self.begin_edit_topic(path);
                        } else {
                            self.toast(ToastKind::Error, "topic type not editable");
                        }
                    } else {
                        self.toggle_fold();
                    }
                }
            }

            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = Focus::Watchlist;
            }

            KeyCode::Char(' ') => {
                // Pin/unpin the cursor item to the watchlist.
                self.toggle_pin_cursor();
            }

            KeyCode::Char('W') => {
                // Alias of Space on a directory: pin the whole subtree.
                if let Some((path, is_topic)) = self.cursor_path() {
                    let src = if is_topic {
                        MatrixSource::Topic(path.clone())
                    } else {
                        MatrixSource::Glob(path.clone())
                    };
                    if !self.watchlist.contains(&src) {
                        self.watchlist.push(src);
                        self.toast(ToastKind::Success, format!("pinned {} to watchlist", path));
                    }
                    self.persist_watchlist();
                    self.clamp_watchlist_cursor();
                }
            }

            KeyCode::Char('e') => {
                if let Some((topic, true)) = self.cursor_path() {
                    if self.is_writable(&topic) {
                        self.begin_edit_topic(topic);
                    } else {
                        self.toast(ToastKind::Error, "topic type not editable");
                    }
                }
            }

            _ => {}
        }
        UiAction::None
    }

    fn handle_watchlist_key(&mut self, key: KeyEvent) -> UiAction {
        let cells = self.watchlist_cells();
        let n = cells.len();
        if n == 0 {
            // Card keys are all dead on an empty canvas, but the global
            // keys (q / / : c 1-9) still work — handle_normal intercepts
            // them before focus dispatch. Tab/Esc return focus to the tree.
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc) {
                self.focus = Focus::Tree;
            }
            return UiAction::None;
        }
        let cols = self.watch_cols_layout();
        let mut c = self.watchlist_cursor;
        // Column-major reading order: j/k step through cards down a column
        // (and into the next column at the bottom); h/l jump between columns.
        let cur_col = cols
            .iter()
            .position(|(s, cnt)| c >= *s && c < s + cnt)
            .unwrap_or(0);
        match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => self.focus = Focus::Tree,
            KeyCode::Char('j') | KeyCode::Down => c = (c + 1).min(n.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => c = c.saturating_sub(1),
            KeyCode::Char('h') | KeyCode::Left => {
                if cur_col > 0 {
                    let offset = c - cols[cur_col].0;
                    let (ps, pc) = cols[cur_col - 1];
                    c = ps + offset.min(pc - 1);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if cur_col + 1 < cols.len() {
                    let offset = c - cols[cur_col].0;
                    let (ps, pc) = cols[cur_col + 1];
                    c = ps + offset.min(pc.saturating_sub(1));
                }
            }
            KeyCode::Char('g') => c = 0,
            KeyCode::Char('G') => c = n.saturating_sub(1),
            KeyCode::Char('x') | KeyCode::Char(' ') => {
                // Dismiss the active card. On an overlay COMPOSITE that
                // means every member — destructive, so stash the previous
                // watchlist into the `u` undo slot and say so honestly.
                if let Some(topic) = cells.get(c).cloned() {
                    let members = self.field_members(&topic);
                    if members.len() > 1 {
                        self.last_watchlist = Some(self.watchlist.clone());
                        for m in &members {
                            self.unpin_topic(m);
                        }
                        self.toast(
                            ToastKind::Warn,
                            format!("removed overlay: {} topics — u restores", members.len()),
                        );
                    } else {
                        self.unpin_topic(&topic);
                    }
                }
                // NOTE: the clamp can move the cursor into another column
                // without the h/l reset below running — the renderer's
                // scroll-into-view clamp self-corrects the v offset.
                self.clamp_watchlist_cursor();
                return UiAction::None;
            }
            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(topic) = cells.get(c).cloned() {
                    if self.is_writable(&topic) {
                        self.begin_edit_topic(topic);
                    } else {
                        self.toast(ToastKind::Error, "topic type not editable");
                    }
                }
            }
            KeyCode::Char('o') => {
                // Overlay membership: merges field cards into ONE composite
                // field. Individual cards stay the default — o is opt-in
                // per topic, reversible with o, persisted in config.
                if let Some(topic) = cells.get(c).cloned() {
                    if !self.is_field_card(&topic) {
                        self.toast(ToastKind::Info, "not a field topic");
                    } else {
                        let overlaid = &mut self.config.field.overlay_topics;
                        let msg = match overlaid.iter().position(|t| t == &topic) {
                            Some(pos) => {
                                overlaid.remove(pos);
                                "removed from overlay".to_string()
                            }
                            None => {
                                overlaid.push(topic.clone());
                                "added to overlay".to_string()
                            }
                        };
                        if let Err(e) = self.config.save() {
                            self.toast(ToastKind::Error, format!("save config: {}", e));
                        }
                        self.toast(ToastKind::Success, msg);
                    }
                }
                return UiAction::None;
            }
            KeyCode::Char('f') => {
                // Enlarged field view: only meaningful over a field card.
                if let Some(topic) = cells.get(c).cloned() {
                    if self.is_field_card(&topic) {
                        self.field_view = Some(topic);
                        self.mode = Mode::FieldView;
                    } else {
                        self.toast(ToastKind::Info, "not a field topic");
                    }
                }
                return UiAction::None;
            }
            KeyCode::Char('v') => {
                // Toggle pose card between the braille field map and a
                // compact numeric card.
                if let Some(topic) = cells.get(c).cloned() {
                    if self.can_toggle_field_card(&topic) {
                        self.toggle_pose_view_on(&topic);
                    } else {
                        self.toast(ToastKind::Info, "not a pose topic");
                    }
                }
                return UiAction::None;
            }
            _ => {}
        }
        self.watchlist_cursor = c;
        let new_col = cols
            .iter()
            .position(|(s, cnt)| c >= *s && c < s + cnt)
            .unwrap_or(0);
        if new_col != cur_col {
            // Column change: the vertical follow restarts from the top of
            // the newly entered column.
            self.watchlist_vscroll = 0;
        }
        UiAction::None
    }

    /// Enlarged field view: `f` closes (same key that opened it); Esc is
    /// the standard overlay escape; `v` collapses back to a compact numeric
    /// card. Everything else is ignored — the popup is a passive readout.
    fn handle_field_view(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Char('f') | KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.field_view = None;
                self.focus = Focus::Watchlist;
            }
            KeyCode::Char('v') => {
                let topic = self.field_view.take();
                self.mode = Mode::Normal;
                self.focus = Focus::Watchlist;
                if let Some(t) = topic {
                    self.toggle_pose_view_on(&t);
                }
            }
            _ => {}
        }
        UiAction::None
    }

    fn begin_edit_topic(&mut self, topic: String) {
        self.mode = Mode::Edit;
        self.edit_topic = Some(topic);
        self.edit_input.clear();
        self.edit_error = None;
    }

    /// Is this topic rendered as a pose field card? Sticky: true once the
    /// topic has EVER classified as a pose (pose sources legitimately
    /// publish empty/no-estimate values between fixes — without stickiness
    /// the card would flicker on every cycle), or the user explicitly
    /// opted it in via `v` / `Field: Toggle Pose View on Active Card` —
    /// unless explicitly collapsed to a compact numeric card via
    /// `field.disable_pose_topics`. Lookalike topics (target poses,
    /// arbitrary double[6]) that never classified stay normal value cards.
    pub fn is_field_card(&self, topic: &str) -> bool {
        if self
            .config
            .field
            .disable_pose_topics
            .iter()
            .any(|t| t == topic)
        {
            return false;
        }
        self.store
            .topics
            .get(topic)
            .map(|t| t.is_pose_source())
            .unwrap_or(false)
            || self
                .config
                .field
                .force_pose_topics
                .iter()
                .any(|t| t == topic)
    }

    /// Can this topic be toggled between a braille field card and a compact
    /// numeric card via `v`?
    pub fn can_toggle_field_card(&self, topic: &str) -> bool {
        if self.is_field_card(topic)
            || self
                .config
                .field
                .disable_pose_topics
                .iter()
                .any(|t| t == topic)
        {
            return true;
        }
        self.store.topics.get(topic).is_some_and(|td| {
            td.is_pose_source()
                || td
                    .current
                    .as_ref()
                    .and_then(crate::field::forced_reading)
                    .is_some()
        })
    }

    /// Collapse a field card (and any overlay-group members drawn with it)
    /// into compact numeric card(s), or re-enable field rendering on a
    /// collapsed/lookalike pose topic.
    fn toggle_pose_view_on(&mut self, topic: &str) {
        if self.is_field_card(topic) {
            let mut members = self.field_members(topic);
            if members.is_empty() {
                members.push(topic.to_string());
            }
            for t in &members {
                self.config.field.force_pose_topics.retain(|x| x != t);
                let auto = self
                    .store
                    .topics
                    .get(t)
                    .is_some_and(|td| td.is_pose_source());
                if auto && !self.config.field.disable_pose_topics.iter().any(|x| x == t) {
                    self.config.field.disable_pose_topics.push(t.clone());
                }
            }
        } else {
            self.config.field.disable_pose_topics.retain(|x| x != topic);
            let auto = self
                .store
                .topics
                .get(topic)
                .is_some_and(|td| td.is_pose_source());
            if !auto
                && !self
                    .config
                    .field
                    .force_pose_topics
                    .iter()
                    .any(|x| x == topic)
            {
                self.config.field.force_pose_topics.push(topic.to_string());
            }
        }
        if let Err(e) = self.config.save() {
            self.toast(ToastKind::Error, format!("save config: {}", e));
        }
        self.clamp_watchlist_cursor();
    }

    /// The topic's current pose reading, if the current value classifies.
    /// None between estimates (empty arrays etc.) — the field card then
    /// renders WITHOUT the robot marker instead of disappearing.
    pub fn field_reading(&self, topic: &str) -> Option<riont_store::pose::PoseReading> {
        let td = self.store.topics.get(topic)?;
        let v = td.current.as_ref()?;
        let forced = self
            .config
            .field
            .force_pose_topics
            .iter()
            .any(|t| t == topic);
        let auto = riont_store::pose::classify(topic, td.type_str.as_deref(), v);
        // Forced only widens for topics the user explicitly opted in.
        if forced {
            auto.or_else(|| crate::field::forced_reading(v))
        } else {
            auto
        }
    }

    /// Re-resolve the cached field map from config (palette cycle, config
    /// editor reload). Warns when an external walls_file fails and the
    /// built-in fallback is used instead.
    pub fn reload_field_map(&mut self) {
        let (map, warn) = crate::field::resolve(&self.config);
        self.field_map = map;
        if let Some(w) = warn {
            self.toast(ToastKind::Warn, format!("{} — using built-in map", w));
        }
    }

    /// Resolved numbered preset slots `1..=9` (config.json first, legacy
    /// `.nt-views.json` fallback when config has no presets).
    pub fn preset_slots(&self) -> [Option<crate::config::PresetSlot>; 9] {
        if self.config.preset_slots.is_empty() && self.config.presets.is_empty() {
            let mut slots: [Option<crate::config::PresetSlot>; 9] = Default::default();
            for (i, (name, topics)) in legacy_presets().into_iter().take(9).enumerate() {
                slots[i] = Some(crate::config::PresetSlot { name, topics });
            }
            slots
        } else {
            self.config.resolved_slots()
        }
    }

    /// Occupied preset slots in `1..=9` order as `(zero_based_idx, PresetSlot)`.
    pub fn occupied_preset_slots(&self) -> Vec<(usize, crate::config::PresetSlot)> {
        self.preset_slots()
            .into_iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.map(|ps| (idx, ps)))
            .collect()
    }

    /// Maximum preset name character count that fits alongside `[1]..[9]` in
    /// the current terminal HUD width.
    pub fn preset_name_max_len(&self) -> usize {
        // Reserve: left HUD + 2-space gap + 35-col base strip (`[1]..[9]`) + 2 (`:` and `*`).
        let used = self.hud_left_width as usize + 2 + 35 + 2;
        (self.hud_width as usize).saturating_sub(used).clamp(6, 24)
    }

    fn open_save_preset(&mut self, slot: usize) {
        if self.watchlist.is_empty() {
            self.toast(ToastKind::Warn, "watchlist is empty — nothing to save");
            return;
        }
        let existing_name = self.preset_slots()[slot]
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        self.mode = Mode::Prompt;
        self.prompt_kind = PromptKind::SavePreset { slot };
        self.prompt_input = existing_name;
        self.prompt_error = None;
    }

    fn open_rename_preset(&mut self, slot: usize) {
        if let Some(ps) = &self.preset_slots()[slot] {
            self.mode = Mode::Prompt;
            self.prompt_kind = PromptKind::RenamePreset { slot };
            self.prompt_input = ps.name.clone();
            self.prompt_error = None;
        } else {
            self.toast(
                ToastKind::Warn,
                format!("preset slot {} is empty", slot + 1),
            );
        }
    }

    fn save_preset_slot(&mut self, slot: usize, name: String) {
        self.config.migrate_legacy_presets_to_slots();
        let topics = self.current_watchlist_strings();
        self.config.preset_slots.insert(
            (slot + 1) as u8,
            crate::config::PresetSlot {
                name: name.clone(),
                topics,
            },
        );
        if let Err(e) = self.config.save() {
            self.toast(ToastKind::Error, format!("save config: {}", e));
        }
        self.active_preset = Some(slot);
        self.preset_dirty = false;
        let msg = if name.is_empty() {
            format!("Preset {} saved", slot + 1)
        } else {
            format!("Preset {} ('{}') saved", slot + 1, name)
        };
        self.toast(ToastKind::Success, msg);
    }

    fn rename_preset_slot(&mut self, slot: usize, name: String) {
        self.config.migrate_legacy_presets_to_slots();
        if let Some(ps) = self.config.preset_slots.get_mut(&((slot + 1) as u8)) {
            ps.name = name.clone();
        }
        if let Err(e) = self.config.save() {
            self.toast(ToastKind::Error, format!("save config: {}", e));
        }
        let msg = if name.is_empty() {
            format!("Preset {} name cleared", slot + 1)
        } else {
            format!("Preset {} renamed to '{}'", slot + 1, name)
        };
        self.toast(ToastKind::Success, msg);
    }

    fn clear_preset_slot_at(&mut self, slot: usize) {
        self.config.migrate_legacy_presets_to_slots();
        self.config.preset_slots.remove(&((slot + 1) as u8));
        if let Err(e) = self.config.save() {
            self.toast(ToastKind::Error, format!("save config: {}", e));
        }
        if self.active_preset == Some(slot) {
            self.active_preset = None;
            self.preset_dirty = false;
        }
        self.toast(ToastKind::Info, format!("Cleared preset slot {}", slot + 1));
    }

    /// Load preset `idx` (0-based) into the watchlist and focus it.
    fn apply_preset(&mut self, idx: usize) {
        match self.preset_slots().get(idx).and_then(|o| o.clone()) {
            Some(ps) => {
                self.load_preset_slot(idx, &ps);
            }
            None => {
                self.toast(
                    ToastKind::Warn,
                    format!("preset slot {} is empty — press p to save", idx + 1),
                );
            }
        }
    }

    fn load_preset_slot(&mut self, idx: usize, ps: &crate::config::PresetSlot) {
        // Stash the replaced view so `u` / Restore Previous can bring it
        // back — a stray preset digit must never be a one-way door.
        self.last_watchlist = Some(self.watchlist.clone());
        self.last_active_preset = self.active_preset;
        let replaced = self.watchlist_cells().len();
        self.watchlist = ps
            .topics
            .iter()
            .map(|t| {
                if let Some(prefix) = t.strip_suffix("/*") {
                    MatrixSource::Glob(prefix.to_string())
                } else {
                    MatrixSource::Topic(t.clone())
                }
            })
            .collect();
        self.watchlist_cursor = 0;
        self.watchlist_scroll = 0;
        self.watchlist_vscroll = 0;
        self.focus = Focus::Watchlist;
        self.active_preset = Some(idx);
        self.preset_dirty = false;
        let label = if ps.name.is_empty() {
            format!("#{}", idx + 1)
        } else {
            format!("{}:{}", idx + 1, ps.name)
        };
        let n = self.watchlist_cells().len();
        if replaced > 0 {
            self.toast(
                ToastKind::Success,
                format!(
                    "preset {}: {} card(s) (replaced {} — restore available)",
                    label, n, replaced
                ),
            );
        } else {
            self.toast(
                ToastKind::Success,
                format!("preset {}: {} card(s)", label, n),
            );
        }
        self.persist_watchlist();
    }

    fn handle_search(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            // Space (Tab is a one-handed alias) pins the highlighted match
            // to the watchlist directly, then advances to the next match.
            KeyCode::Char(' ') | KeyCode::Tab => {
                if let Some(topic) = self.search_matches.get(self.search_cursor).cloned() {
                    if self.is_pinned(&topic) {
                        self.unpin_topic(&topic);
                    } else {
                        self.watchlist.push(MatrixSource::Topic(topic.clone()));
                        self.toast(ToastKind::Success, format!("pinned {}", topic));
                        self.persist_watchlist();
                    }
                }
                // Stop at the last match instead of clamping: pinning is a
                // toggle, so stepping past the end would un-pin it again.
                if self.search_cursor + 1 < self.search_matches.len() {
                    self.search_cursor += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(topic) = self.search_matches.get(self.search_cursor).cloned() {
                    self.expand_ancestors(&topic);
                    self.mode = Mode::Normal;
                    self.focus = Focus::Tree;
                    // Place the cursor on the topic row.
                    let rows = crate::ui::tree::build_tree(&self.store, &self.expanded);
                    if let Some(pos) = rows.iter().position(|r| r.is_topic && r.path == topic) {
                        self.tree_cursor = pos;
                    }
                }
            }
            // Arrows only: j/k are typed into the query, not movement —
            // topic names contain those letters ("SparkMax").
            KeyCode::Down => {
                self.search_cursor =
                    (self.search_cursor + 1).min(self.search_matches.len().saturating_sub(1));
            }
            KeyCode::Up => {
                self.search_cursor = self.search_cursor.saturating_sub(1);
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.recompute_matches();
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.recompute_matches();
            }
            _ => {}
        }
        UiAction::None
    }

    fn handle_palette(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.palette_query.clear();
            }
            // Arrows only: j/k are typed into the query, not movement.
            KeyCode::Down => {
                self.palette_cursor =
                    (self.palette_cursor + 1).min(self.palette_matches.len().saturating_sub(1));
            }
            KeyCode::Up => {
                self.palette_cursor = self.palette_cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                // Execute the HIGHLIGHTED entry (not the top match): the
                // cursor is the user's selection.
                let cmd = self
                    .palette_matches
                    .get(self.palette_cursor)
                    .or_else(|| self.palette_matches.first())
                    .and_then(|&i| COMMANDS.get(i))
                    .map(|(c, _)| *c);
                self.mode = Mode::Normal;
                self.palette_query.clear();
                if let Some(cmd) = cmd {
                    return self.exec_command(cmd);
                }
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.recompute_palette();
            }
            KeyCode::Char(c) => {
                self.palette_query.push(c);
                self.recompute_palette();
            }
            _ => {}
        }
        UiAction::None
    }

    fn exec_command(&mut self, cmd: Command) -> UiAction {
        match cmd {
            Command::ReconnectNt => {
                // Pure internal client action: drop the socket, re-handshake
                // with the same target. Never queries a topic.
                self.toast(
                    ToastKind::Info,
                    format!("reconnecting to {}...", self.target),
                );
                UiAction::Client(ClientCommand::Reconnect)
            }
            Command::WatchlistClear => {
                // Destructive but recoverable: stash for `u` / Restore Previous
                // instead of a y/n prompt (faster in the pit).
                self.last_watchlist = Some(self.watchlist.clone());
                self.last_active_preset = self.active_preset;
                let n = self.watchlist.len();
                self.watchlist.clear();
                self.watchlist_cursor = 0;
                self.watchlist_scroll = 0;
                self.watchlist_vscroll = 0;
                self.toast(
                    ToastKind::Success,
                    format!("watchlist cleared ({} card(s)) — restore available", n),
                );
                self.persist_watchlist();
                UiAction::None
            }
            Command::WatchlistRestorePrevious => {
                self.restore_previous_watchlist();
                UiAction::None
            }
            // Alliance is USER-SET ONLY: the app never infers it from topic
            // names or values. The setting affects rendering only (x mirror
            // on the field card); stored NT values are never transformed.
            Command::FieldAllianceBlue => {
                self.config.field.alliance = "blue".into();
                if let Err(e) = self.config.save() {
                    self.toast(ToastKind::Error, format!("save config: {}", e));
                }
                self.toast(ToastKind::Info, "field: blue origin (no x flip)");
                UiAction::None
            }
            Command::FieldAllianceRed => {
                self.config.field.alliance = "red".into();
                if let Err(e) = self.config.save() {
                    self.toast(ToastKind::Error, format!("save config: {}", e));
                }
                self.toast(ToastKind::Info, "field: red origin (field cards mirror x)");
                UiAction::None
            }
            Command::FieldTogglePoseView => {
                // Opt-in/opt-out override for field card rendering on the active
                // topic (collapses auto-detected pose cards or widens lookalikes).
                match self.active_topic() {
                    Some(topic) => self.toggle_pose_view_on(&topic),
                    None => self.toast(ToastKind::Warn, "no topic under cursor"),
                }
                UiAction::None
            }
            Command::FieldToggleTrail => {
                self.show_pose_trail = !self.show_pose_trail;
                self.toast(
                    ToastKind::Info,
                    format!(
                        "pose trail {}",
                        if self.show_pose_trail { "on" } else { "off" }
                    ),
                );
                UiAction::None
            }
            Command::FieldCycleMap => {
                // Cycle the built-in maps (2024 -> 2025 -> 2026 -> ...).
                // Cycling selects built-ins, so an external walls_file is
                // cleared — the file overrides built-ins while it is set.
                let pos = crate::field::BUILTIN_MAPS
                    .iter()
                    .position(|n| *n == self.field_map.name)
                    .map(|p| (p + 1) % crate::field::BUILTIN_MAPS.len())
                    .unwrap_or(0);
                let next = crate::field::BUILTIN_MAPS[pos];
                self.config.field.map = next.to_string();
                self.config.field.walls_file = None;
                if let Err(e) = self.config.save() {
                    self.toast(ToastKind::Error, format!("save config: {}", e));
                }
                self.reload_field_map();
                let m = &self.field_map;
                self.toast(
                    ToastKind::Success,
                    format!(
                        "field map: {} — {} ({:.2} x {:.2} m)",
                        m.name, m.game, m.length_m, m.width_m
                    ),
                );
                UiAction::None
            }
            Command::CopyTopicPath => {
                match self.active_topic() {
                    Some(topic) => match copy_to_clipboard(&topic) {
                        Ok(()) => self.toast(ToastKind::Success, format!("copied {}", topic)),
                        Err(e) => self.toast(ToastKind::Error, format!("clipboard: {}", e)),
                    },
                    None => self.toast(ToastKind::Warn, "no topic under cursor"),
                }
                UiAction::None
            }
            Command::SettingsOpen => UiAction::OpenEditor(crate::config::Config::path()),
            Command::SettingsView => {
                self.mode = Mode::SettingsView;
                UiAction::None
            }
            Command::SettingsAddTarget => {
                self.mode = Mode::Prompt;
                self.prompt_kind = PromptKind::AddTarget;
                self.prompt_input.clear();
                self.prompt_error = None;
                UiAction::None
            }
            Command::SettingsRemoveTarget => {
                if self.config.saved_targets.is_empty() {
                    self.toast(ToastKind::Warn, "no saved targets to remove");
                } else {
                    self.mode = Mode::PickTarget;
                    self.picker_cursor = 0;
                }
                UiAction::None
            }
            Command::WatchlistSavePreset => {
                let target_slot = self
                    .preset_slots()
                    .iter()
                    .position(|s| s.is_none())
                    .unwrap_or_else(|| self.active_preset.unwrap_or(0));
                self.open_save_preset(target_slot);
                UiAction::None
            }
            Command::WatchlistLoadPreset => {
                if self.occupied_preset_slots().is_empty() {
                    self.toast(
                        ToastKind::Warn,
                        "no presets configured (press p to save one to slots 1-9)",
                    );
                } else {
                    self.mode = Mode::PickPreset;
                    self.picker_cursor = 0;
                }
                UiAction::None
            }
            Command::RestartRobotCode => {
                // Check if the robot is connected before attempting to restart its code.
                if !self.connected {
                    self.toast(ToastKind::Warn, "robot is not connected");
                    return UiAction::None;
                }

                // Primary method: background SSH to the robot (no NT topic
                // involved). The client task spawns it asynchronously and
                // reports the outcome as a toast.
                let host = self
                    .target
                    .split(':')
                    .next()
                    .unwrap_or(&self.target)
                    .to_string();
                let user = self.config.system.ssh_user.clone();
                let restart = self.config.system.restart_cmd.clone();
                self.toast(
                    ToastKind::Info,
                    format!("restart: ssh {}@{}...", user, host),
                );
                UiAction::RestartRobotCode {
                    host,
                    user,
                    cmd: restart,
                }
            }
        }
    }

    fn handle_edit(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.edit_topic = None;
                self.edit_input.clear();
                self.edit_error = None;
            }
            KeyCode::Backspace => {
                self.edit_input.pop();
            }
            KeyCode::Enter => {
                let Some(topic) = self.edit_topic.clone() else {
                    self.mode = Mode::Normal;
                    return UiAction::None;
                };
                match parse_value(
                    &self.edit_input,
                    self.store.topics.get(&topic).map(|t| t.data_type),
                ) {
                    Ok(v) => {
                        self.mode = Mode::Normal;
                        self.edit_topic = None;
                        self.edit_input.clear();
                        self.edit_error = None;
                        // Honesty: the client re-queues offline publishes for
                        // the next successful session, so claiming
                        // "published" while disconnected would be a lie the
                        // pit crew acts on. Warn that it is queued instead.
                        if self.connected {
                            // Local echo: the NT4 server never sends a
                            // client's own publish back to it (verified
                            // against ntcore), so without this the tree and
                            // inspector would keep showing the pre-edit
                            // value forever even though the write landed on
                            // the robot. Apply optimistically; the engine
                            // then verifies the write with a read-back
                            // (NtUpdate::PublishVerified).
                            let server_ts = self
                                .store
                                .topics
                                .get(&topic)
                                .and_then(|t| t.last_server_ts)
                                .unwrap_or(0);
                            self.store.apply_value(
                                &topic,
                                v.clone(),
                                server_ts,
                                std::time::Instant::now(),
                            );
                            self.toast(
                                ToastKind::Success,
                                format!("published {} = {}", topic, v.format()),
                            );
                        } else {
                            self.toast(
                                ToastKind::Warn,
                                format!(
                                    "queued {} = {} — offline, will send on reconnect",
                                    topic,
                                    v.format()
                                ),
                            );
                        }
                        return UiAction::Client(ClientCommand::Publish { topic, value: v });
                    }
                    Err(e) => {
                        self.edit_error = Some(e);
                    }
                }
            }
            KeyCode::Char(c) => {
                self.edit_input.push(c);
            }
            _ => {}
        }
        UiAction::None
    }

    /// Connection Picker: select a saved target or type a fresh one.
    /// Single purpose — connect. Enter confirms, Esc cancels. No
    /// add/edit/delete here (Settings owns management). Digits are
    /// ORDINARY input: addresses and team numbers start with digits, so
    /// any bare-digit quick-jump would hijack the first keystroke of
    /// exactly the text it must never touch. Selection moves with
    /// arrows; Enter connects.
    fn handle_connect(&mut self, key: KeyEvent) -> UiAction {
        let targets = self.config.picker_targets();
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.connect_input.clear();
            }
            KeyCode::Backspace => {
                self.connect_input.pop();
            }
            // Arrows only: j/k are typed into the address, not movement
            // (hostnames contain those letters).
            KeyCode::Down => {
                self.connect_cursor =
                    (self.connect_cursor + 1).min(targets.len().saturating_sub(1));
            }
            KeyCode::Up => {
                self.connect_cursor = self.connect_cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                // Typed input wins; otherwise connect to the highlighted
                // (most recently used first) saved target.
                let input = self.connect_input.trim().to_string();
                self.mode = Mode::Normal;
                self.connect_input.clear();
                if !input.is_empty() {
                    return self.connect_to(resolve_target(&input));
                }
                if let Some(t) = targets.get(self.connect_cursor) {
                    return self.connect_to(t.ip.clone());
                }
            }
            KeyCode::Char(c) => {
                self.connect_input.push(c);
            }
            _ => {}
        }
        UiAction::None
    }

    fn connect_to(&mut self, target: String) -> UiAction {
        self.target = target.clone();
        // The new target is (almost always) a different robot: keeping the
        // old store would render the previous robot's ghost topics, frozen
        // values and wrong topic count. Start clean; the watchlist
        // survives, globs re-expand as the new robot publishes, and stale
        // explicit pins simply show "-" until re-announced (or forever for
        // topics this robot never has — acceptable, visible as a dead card).
        self.store.topics.clear();
        self.connected_since = None;
        self.runtime_at_disconnect = None;
        self.tree_cursor = 0;
        self.clamp_watchlist_cursor();
        self.toast(ToastKind::Info, format!("connecting to {}...", target));
        UiAction::Client(ClientCommand::Retarget(target))
    }

    fn handle_prompt(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.prompt_input.clear();
                self.prompt_error = None;
            }
            KeyCode::Backspace => {
                self.prompt_input.pop();
                self.prompt_error = None;
            }
            KeyCode::Enter => {
                let input = self.prompt_input.trim().to_string();
                match self.prompt_kind {
                    PromptKind::AddTarget => {
                        if input.is_empty() {
                            self.prompt_error = Some("empty".into());
                            return UiAction::None;
                        }
                        // "Practice 10.99.86.2" -> name + address; a bare
                        // team number or IP names itself.
                        let (name, addr) = match input.split_once(' ') {
                            Some((nm, rest)) if !rest.trim().is_empty() => {
                                let _ = nm;
                                (
                                    input[..input.len() - rest.len()].trim().to_string(),
                                    rest.trim().to_string(),
                                )
                            }
                            _ => {
                                let addr = resolve_target(&input);
                                let name = if input.parse::<u32>().is_ok() {
                                    format!("Team {}", input)
                                } else {
                                    format!("Host {}", addr)
                                };
                                (name, addr)
                            }
                        };
                        let addr = resolve_target(&addr);
                        // Replace an existing entry for the same address.
                        self.config.saved_targets.retain(|t| t.ip != addr);
                        self.config.saved_targets.push(SavedTarget {
                            name: name.clone(),
                            ip: addr,
                        });
                        if let Err(e) = self.config.save() {
                            self.toast(ToastKind::Error, format!("save config: {}", e));
                        }
                        self.toast(ToastKind::Success, format!("Added {}", name));
                        self.mode = Mode::Normal;
                        self.prompt_input.clear();
                        self.prompt_error = None;
                    }
                    PromptKind::SavePreset { slot } => {
                        let name = input;
                        let max_len = self.preset_name_max_len();
                        let len = name.chars().count();
                        if len > max_len {
                            self.prompt_error = Some(format!("too long ({}/{})", len, max_len));
                            return UiAction::None;
                        }
                        self.save_preset_slot(slot, name);
                        self.mode = Mode::Normal;
                        self.prompt_input.clear();
                        self.prompt_error = None;
                    }
                    PromptKind::RenamePreset { slot } => {
                        let name = input;
                        let max_len = self.preset_name_max_len();
                        let len = name.chars().count();
                        if len > max_len {
                            self.prompt_error = Some(format!("too long ({}/{})", len, max_len));
                            return UiAction::None;
                        }
                        self.rename_preset_slot(slot, name);
                        self.mode = Mode::Normal;
                        self.prompt_input.clear();
                        self.prompt_error = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                self.prompt_input.push(c);
                self.prompt_error = None;
            }
            _ => {}
        }
        UiAction::None
    }

    fn handle_confirm_clear_preset(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(slot) = self.clear_preset_slot.take() {
                    self.clear_preset_slot_at(slot);
                }
                self.mode = Mode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.clear_preset_slot = None;
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        UiAction::None
    }

    fn handle_pick_target(&mut self, key: KeyEvent) -> UiAction {
        let targets = self.config.saved_targets.clone();
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => {
                self.picker_cursor = (self.picker_cursor + 1).min(targets.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.picker_cursor = self.picker_cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(t) = targets.get(self.picker_cursor).cloned() {
                    self.config.saved_targets.retain(|x| x.ip != t.ip);
                    if let Err(e) = self.config.save() {
                        self.toast(ToastKind::Error, format!("save config: {}", e));
                    }
                    self.toast(ToastKind::Success, format!("Removed {}", t.name));
                }
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        UiAction::None
    }

    fn handle_pick_preset(&mut self, key: KeyEvent) -> UiAction {
        let presets = self.occupied_preset_slots();
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Down | KeyCode::Char('j') => {
                self.picker_cursor = (self.picker_cursor + 1).min(presets.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.picker_cursor = self.picker_cursor.saturating_sub(1);
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = c.to_digit(10).unwrap() as usize - 1;
                self.mode = Mode::Normal;
                self.apply_preset(idx);
            }
            KeyCode::Enter => {
                if let Some((idx, ps)) = presets.get(self.picker_cursor).cloned() {
                    self.mode = Mode::Normal;
                    self.load_preset_slot(idx, &ps);
                }
            }
            _ => {}
        }
        UiAction::None
    }

    fn handle_settings_view(&mut self, key: KeyEvent) -> UiAction {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter) {
            self.mode = Mode::Normal;
        }
        UiAction::None
    }

    // ------------------------------------------------------------------
    // helpers
    // ------------------------------------------------------------------

    /// h toggles the directory under the cursor: collapsed unfolds, expanded
    /// folds. On a topic row, toggles the topic's parent directory. Silent:
    /// folding is visually self-evident, so no toast fires.
    fn toggle_fold(&mut self) {
        let Some((path, is_topic)) = self.cursor_path() else {
            return;
        };
        let dir = if is_topic {
            match path.rsplit_once('/') {
                Some((parent, _)) => parent.to_string(),
                None => return, // top-level topic: nothing to fold
            }
        } else {
            path
        };
        if !self.expanded.remove(&dir) {
            self.expanded.insert(dir);
        }
    }

    fn is_writable(&self, topic: &str) -> bool {
        self.store
            .topics
            .get(topic)
            .map(|t| t.data_type.is_writable())
            .unwrap_or(false)
    }

    fn recompute_matches(&mut self) {
        // Matches are stored as NAMES and resolved fresh at act-time —
        // see the field comment.
        if self.query.is_empty() {
            self.search_matches = self.store.sorted_names();
            return;
        }
        let matcher = SkimMatcherV2::default();
        let names = self.store.sorted_names();
        let mut scored: Vec<(i64, String)> = names
            .into_iter()
            .filter_map(|n| matcher.fuzzy_match(&n, &self.query).map(|s| (s, n)))
            .collect();
        scored.sort_unstable_by_key(|(score, _)| std::cmp::Reverse(*score));
        self.search_matches = scored.into_iter().map(|(_, n)| n).collect();
        self.search_cursor = 0;
    }

    fn recompute_palette(&mut self) {
        if self.palette_query.is_empty() {
            self.palette_matches = (0..COMMANDS.len()).collect();
            return;
        }
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, usize)> = COMMANDS
            .iter()
            .enumerate()
            .filter_map(|(i, (_, name))| {
                matcher
                    .fuzzy_match(name, &self.palette_query)
                    .map(|s| (s, i))
            })
            .collect();
        scored.sort_unstable_by_key(|(score, _)| std::cmp::Reverse(*score));
        self.palette_matches = scored.into_iter().map(|(_, i)| i).collect();
        self.palette_cursor = 0;
    }
}

/// Parse user input into an NtValue, guided by the topic's current type.
fn parse_value(s: &str, hint: Option<NtType>) -> Result<NtValue, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty value".into());
    }
    match hint {
        Some(NtType::Boolean) | None
            if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") =>
        {
            Ok(NtValue::Boolean(s.eq_ignore_ascii_case("true")))
        }
        Some(NtType::Int) => s
            .parse::<i64>()
            .map(NtValue::Int)
            .map_err(|_| format!("not an int: {}", s)),
        Some(NtType::Double) => s
            .parse::<f64>()
            .map(NtValue::Double)
            .map_err(|_| format!("not a number: {}", s)),
        Some(NtType::Str) => Ok(NtValue::Str(s.to_string())),
        // Explicit boolean failure BEFORE the catch-all: a bool topic IS
        // editable, so "only boolean/int/double/string topics are editable"
        // reads as a lie when it is the INPUT (not the topic) that is invalid.
        Some(NtType::Boolean) => Err("enter true or false".into()),
        None => {
            // Unknown type: try int, then double, then string.
            if let Ok(i) = s.parse::<i64>() {
                Ok(NtValue::Int(i))
            } else if let Ok(f) = s.parse::<f64>() {
                Ok(NtValue::Double(f))
            } else {
                Ok(NtValue::Str(s.to_string()))
            }
        }
        Some(_) => Err("only boolean/int/double/string topics are editable".into()),
    }
}

/// Legacy preset file `.nt-views.json` (array form) — still honored when
/// the config has no presets so old setups keep working after the config
/// migration.
fn legacy_presets() -> Vec<(String, Vec<String>)> {
    let Ok(txt) = std::fs::read_to_string(".nt-views.json") else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else {
        return Vec::new();
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|it| {
                    let name = it.get("name")?.as_str()?.to_string();
                    let topics = it
                        .get("topics")?
                        .as_array()?
                        .iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect();
                    Some((name, topics))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// System clipboard via arboard (Windows/macOS/X11/Wayland).
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_string()))
        .map_err(|e| e.to_string())
}

/// Turn user input (team number, host, or host:port) into a `host:port`
/// target, mirroring the CLI argument handling in main.
pub fn resolve_target(input: &str) -> String {
    let input = input.trim();
    let port = riont_nt4::NT_PORT;
    // Team number -> 10.TE.AM.2 (valid for team < 10000).
    if let Ok(team) = input.parse::<u32>() {
        if team < 100 {
            return format!("10.{}.{}.2:{}", team / 100, team % 100, port);
        }
        if team < 10000 {
            return format!("10.{}.{:02}.2:{}", team / 100, team % 100, port);
        }
    }
    if input.contains(':') {
        input.to_string()
    } else {
        format!("{}:{}", input, port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riont_store::store::NtValue;

    #[test]
    fn overlay_group_collapses_cells_and_expands_members() {
        let mut app = App::new("127.0.0.1:5810".into());
        // Isolate from the developer's real ~/.config/riont/config.json.
        app.config = crate::config::Config::with_defaults();
        // Two pose topics: botpose (auto-classified) + targetpose (forced).
        app.apply_values(
            vec![
                (
                    "SmartDashboard/botpose_wpiblue".into(),
                    NtValue::DoubleArray(vec![1.0, 2.0, 0.0, 0.0, 0.0, 90.0]),
                    1_000,
                ),
                (
                    "SmartDashboard/targetpose".into(),
                    NtValue::DoubleArray(vec![3.0, 4.0, 0.0, 0.0, 0.0, 45.0]),
                    2_000,
                ),
            ],
            std::time::Instant::now(),
        );
        app.config
            .field
            .force_pose_topics
            .push("SmartDashboard/targetpose".into());
        app.watchlist = vec![
            MatrixSource::Topic("SmartDashboard/botpose_wpiblue".into()),
            MatrixSource::Topic("SmartDashboard/targetpose".into()),
        ];
        // Not overlaid yet: two cells, two single-member cards.
        assert_eq!(app.watchlist_cells().len(), 2);
        // Overlay both.
        app.config.field.overlay_topics = vec![
            "SmartDashboard/botpose_wpiblue".into(),
            "SmartDashboard/targetpose".into(),
        ];
        let cells = app.watchlist_cells();
        assert_eq!(cells.len(), 1, "cells: {:?}", cells);
        let members = app.field_members("SmartDashboard/botpose_wpiblue");
        assert_eq!(members.len(), 2, "members: {:?}", members);
    }
}
