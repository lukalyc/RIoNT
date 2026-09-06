//! Application state + vim-style input handling. Pure logic, no rendering.

use crate::config::SavedTarget;
use crate::nt::store::{NtType, NtValue, Store};
use crate::nt::ClientCommand;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
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
    /// Generic single-input prompt (Add Robot Target / Save Preset).
    Prompt,
    /// List picker: remove a saved robot target.
    PickTarget,
    /// List picker: load a workspace preset.
    PickPreset,
    /// Read-only configuration summary.
    SettingsView,
    /// Enlarged field view: near-fullscreen field popup for the hovered
    /// watchlist card. `f` opens (from Watchlist focus on a pose card)
    /// and `f`/Esc closes.
    FieldView,
}

/// Keyboard focus. The inspector dock is strictly passive, so focus only
/// ever alternates between the topic tree and the watchlist canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Watchlist,
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
/// CODE reads RUNNING while robot frames arrived within this window.
/// Robot loops tick at 20-50 Hz, so 500 ms of silence reliably means the
/// user program stopped publishing — while absorbing batch/scheduling gaps.
pub const CODE_STALE_MS: u128 = 500;

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

/// The focused single-input prompts opened from the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    AddTarget,
    SavePreset,
}

/// Outcome of a key press: what the caller (main loop) must do about it.
#[derive(Debug, PartialEq)]
pub enum UiAction {
    Quit,
    None,
    Client(ClientCommand),
    /// Suspend the TUI, open the file in $EDITOR, then resume.
    OpenEditor(std::path::PathBuf),
}

pub struct App {
    pub store: Store,
    pub target: String,

    // connection
    pub connected: bool,
    pub connecting: bool,
    pub server_info: String,
    pub first_server_ts: Option<u64>,
    pub last_server_ts: Option<u64>,
    pub disconnect_reason: Option<String>,
    /// Local Instant of the last received value batch: drives CODE RUNNING /
    /// STOPPED (robot user loop alive = frames still streaming).
    pub last_value_at: Option<std::time::Instant>,

    /// Persistent configuration (~/.config/riont/config.json).
    pub config: crate::config::Config,

    // navigation
    pub expanded: HashSet<String>,
    pub tree_cursor: usize,
    pub focus: Focus,

    // watchlist
    pub watchlist: Vec<MatrixSource>,
    /// Watchlist stashed by the last destructive transition (preset load /
    /// Clear All): one-level undo for `u` and `Watchlist: Restore Previous`.
    pub last_watchlist: Option<Vec<MatrixSource>>,
    pub watchlist_cursor: usize,
    /// Scroll offset (card rows) adjusted by the renderer to keep the cursor
    /// visible; clamped defensively here too.
    pub watchlist_scroll: usize,

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
    /// Shared cursor for the list pickers (PickTarget / PickPreset).
    pub picker_cursor: usize,
    pub palette_query: String,
    pub palette_matches: Vec<usize>, // indices into COMMANDS
    pub palette_cursor: usize,

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

    // reconnect attempts since the last successful connection
    pub retry_attempt: u32,
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
            connecting: true,
            server_info: String::new(),
            first_server_ts: None,
            last_server_ts: None,
            disconnect_reason: None,
            last_value_at: None,
            config,
            expanded: HashSet::new(),
            tree_cursor: 0,
            focus: Focus::Tree,
            watchlist: Vec::new(),
            last_watchlist: None,
            watchlist_cursor: 0,
            watchlist_scroll: 0,
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
            picker_cursor: 0,
            palette_query: String::new(),
            palette_matches: Vec::new(),
            palette_cursor: 0,
            toasts: Vec::new(),
            show_pose_trail: true,
            fms_red: None,
            field_view: None,
            field_map: crate::field::FieldMap::builtin("2025-reefscape")
                .expect("default map is built in"),
            retry_attempt: 0,
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
            let ts = *ts;
            if self.first_server_ts.is_none() {
                self.first_server_ts = Some(ts);
            }
            if ts > self.last_server_ts.unwrap_or(0) {
                self.last_server_ts = Some(ts);
            }
            self.store.apply_value(name, v.clone(), ts, now);
        }
        if !batch.is_empty() {
            self.last_value_at = Some(now);
        }
    }

    pub fn set_connected(&mut self, info: String) {
        self.connected = true;
        self.connecting = false;
        self.server_info = info;
        self.disconnect_reason = None;
        self.retry_attempt = 0;
        self.toast(ToastKind::Success, format!("connected to {}", self.target));
    }

    pub fn set_disconnected(&mut self, reason: String) {
        self.connected = false;
        self.connecting = true;
        self.disconnect_reason = Some(reason.clone());
        self.toast(ToastKind::Error, format!("disconnected ({})", reason));
    }

    /// CODE block: Some(true) = RUNNING (frames streaming), Some(false) =
    /// STOPPED (connection alive but no fresh frames), None = offline.
    pub fn code_running(&self) -> Option<bool> {
        if !self.connected {
            return None;
        }
        let fresh = self
            .last_value_at
            .map(|t| t.elapsed().as_millis() <= CODE_STALE_MS)
            .unwrap_or(false);
        Some(fresh)
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

    /// Snapshot the watchlist into `config.last_view` (glob `prefix/*` form)
    /// and save. Called after every mutation so a quit/crash mid-session
    /// never loses the operator's view.
    fn persist_watchlist(&mut self) {
        self.config.last_view = self
            .watchlist
            .iter()
            .map(|s| match s {
                MatrixSource::Topic(t) => t.clone(),
                MatrixSource::Glob(p) => format!("{}/*", p),
            })
            .collect();
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
                self.watchlist_cursor = 0;
                self.watchlist_scroll = 0;
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
        let raw = self.watchlist_cells_raw();
        // Overlay groups collapse into ONE cell (the first member), so
        // navigation and rendering treat the composite as a single card.
        let mut overlay_seen = false;
        raw.into_iter()
            .filter(|c| {
                if self.is_field_card(c) && self.config.field.overlay_topics.iter().any(|t| t == c)
                {
                    if overlay_seen {
                        return false; // later members hide inside the composite
                    }
                    overlay_seen = true;
                }
                true
            })
            .collect()
    }

    /// Pinned topics in display order WITHOUT overlay collapsing.
    fn watchlist_cells_raw(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for src in &self.watchlist {
            match src {
                MatrixSource::Topic(t) => {
                    if seen.insert(t.clone()) {
                        out.push(t.clone());
                    }
                }
                MatrixSource::Glob(prefix) => {
                    let pfx = format!("{}/", prefix);
                    for n in self.store.sorted_names() {
                        if n.starts_with(&pfx) && seen.insert(n.clone()) {
                            out.push(n);
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
            .filter(|c| {
                self.is_field_card(c) && self.config.field.overlay_topics.iter().any(|t| t == c)
            })
            .collect()
    }

    /// Card-column packing rule shared with the renderer: 1 column for
    /// 1-4 cards, 2 for 5-12, 3 beyond — never wider than `width` allows.
    pub fn watch_cols(n: usize, width: usize) -> usize {
        let by_count = if n <= 4 {
            1
        } else if n <= 12 {
            2
        } else {
            3
        };
        let min_card_w = 14usize;
        let by_width = (width / min_card_w).max(1);
        by_count.min(by_width).max(1)
    }

    /// Terminal geometry approximation of the watchlist canvas, shared with
    /// the renderer via `ui::watch_columns` so navigation and painting agree.
    fn watch_cols_layout(&self) -> Vec<(usize, usize)> {
        let (w, h) = crossterm::terminal::size().unwrap_or((120, 36));
        // HUD(1) + status bar(1) + canvas borders(2) + a row of margin.
        let avail_h = h.saturating_sub(5);
        let avail_w = (w * 65 / 100).saturating_sub(2);
        crate::ui::watch_columns(self, avail_h, avail_w)
    }

    /// The topic the current focus points at (tree cursor or watchlist card).
    pub fn active_topic(&self) -> Option<String> {
        match self.focus {
            Focus::Tree => self
                .cursor_path()
                .and_then(|(p, is_topic)| if is_topic { Some(p) } else { None }),
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
                Mode::Search | Mode::Edit | Mode::Connect | Mode::Palette | Mode::Prompt => {
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
                // Workspace presets: 1-9 load entries of .nt-views.json.
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
        }
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
            _ => {}
        }
        self.watchlist_cursor = c;
        UiAction::None
    }

    /// Enlarged field view: `f` closes (same key that opened it); Esc is
    /// the standard overlay escape. Everything else is ignored — the
    /// popup is a passive readout.
    fn handle_field_view(&mut self, key: KeyEvent) -> UiAction {
        if matches!(key.code, KeyCode::Char('f') | KeyCode::Esc) {
            self.mode = Mode::Normal;
            self.field_view = None;
            self.focus = Focus::Watchlist;
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
    /// opted it in via `Field: Toggle Pose View on Active Card`.
    /// Lookalike topics (target poses, arbitrary double[6]) that never
    /// classified stay normal value cards.
    pub fn is_field_card(&self, topic: &str) -> bool {
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

    /// The topic's current pose reading, if the current value classifies.
    /// None between estimates (empty arrays etc.) — the field card then
    /// renders WITHOUT the robot marker instead of disappearing.
    pub fn field_reading(&self, topic: &str) -> Option<crate::pose::PoseReading> {
        let td = self.store.topics.get(topic)?;
        let v = td.current.as_ref()?;
        let forced = self
            .config
            .field
            .force_pose_topics
            .iter()
            .any(|t| t == topic);
        let auto = crate::pose::classify(topic, td.type_str.as_deref(), v);
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

    /// Active presets: config.json first, legacy .nt-views.json fallback.
    fn preset_list(&self) -> Vec<(String, Vec<String>)> {
        let cfg = self.config.preset_list();
        if cfg.is_empty() {
            legacy_presets()
        } else {
            cfg
        }
    }

    /// Load preset `idx` (0-based) into the watchlist and focus it.
    fn apply_preset(&mut self, idx: usize) {
        let presets = self.preset_list();
        match presets.get(idx) {
            Some((name, topics)) => {
                self.load_preset_topics(name, topics);
            }
            None => {
                self.toast(
                    ToastKind::Warn,
                    format!(
                        "no preset {} ({} loaded from config)",
                        idx + 1,
                        presets.len()
                    ),
                );
            }
        }
    }

    fn load_preset_topics(&mut self, name: &str, topics: &[String]) {
        // Stash the replaced view so `u` / Restore Previous can bring it
        // back — a stray preset digit must never be a one-way door.
        self.last_watchlist = Some(self.watchlist.clone());
        let replaced = self.watchlist_cells().len();
        self.watchlist = topics
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
        self.focus = Focus::Watchlist;
        let n = self.watchlist_cells().len();
        if replaced > 0 {
            self.toast(
                ToastKind::Success,
                format!(
                    "preset {}: {} card(s) (replaced {} — restore available)",
                    name, n, replaced
                ),
            );
        } else {
            self.toast(
                ToastKind::Success,
                format!("preset {}: {} card(s)", name, n),
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
                let n = self.watchlist.len();
                self.watchlist.clear();
                self.watchlist_cursor = 0;
                self.watchlist_scroll = 0;
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
                // Opt-in override for ambiguous topics: the auto-classifier
                // stays conservative; only this user action widens it.
                match self.active_topic() {
                    Some(topic) => {
                        let forced = &mut self.config.field.force_pose_topics;
                        let msg = match forced.iter().position(|t| t == &topic) {
                            Some(pos) => {
                                forced.remove(pos);
                                format!("normal card: {}", topic)
                            }
                            None => {
                                forced.push(topic.clone());
                                format!("field card: {}", topic)
                            }
                        };
                        if let Err(e) = self.config.save() {
                            self.toast(ToastKind::Error, format!("save config: {}", e));
                        }
                        self.toast(ToastKind::Success, msg);
                    }
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
                if self.watchlist.is_empty() {
                    self.toast(ToastKind::Warn, "watchlist is empty — nothing to save");
                } else {
                    self.mode = Mode::Prompt;
                    self.prompt_kind = PromptKind::SavePreset;
                    self.prompt_input.clear();
                    self.prompt_error = None;
                }
                UiAction::None
            }
            Command::WatchlistLoadPreset => {
                if self.preset_list().is_empty() {
                    self.toast(
                        ToastKind::Warn,
                        "no presets configured (save one with 'Watchlist: Save Active as Preset')",
                    );
                } else {
                    self.mode = Mode::PickPreset;
                    self.picker_cursor = 0;
                }
                UiAction::None
            }
            Command::RestartRobotCode => {
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
                UiAction::Client(ClientCommand::RestartRobotCode {
                    host,
                    user,
                    cmd: restart,
                })
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
        // Deliberate retarget, not a disconnect: clear the reason so the
        // next Connecting update owns the HUD.
        self.disconnect_reason = None;
        self.target = target.clone();
        // The new target is (almost always) a different robot: keeping the
        // old store would render the previous robot's ghost topics, frozen
        // values and wrong topic count — and first/last_server_ts from two
        // servers would make UPTIME meaningless. Start clean; the watchlist
        // survives, globs re-expand as the new robot publishes, and stale
        // explicit pins simply show "-" until re-announced (or forever for
        // topics this robot never has — acceptable, visible as a dead card).
        self.store.topics.clear();
        self.first_server_ts = None;
        self.last_server_ts = None;
        self.last_value_at = None;
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
                    PromptKind::SavePreset => {
                        let name = input;
                        if name.is_empty() {
                            self.prompt_error = Some("empty".into());
                            return UiAction::None;
                        }
                        let topics: Vec<String> = self
                            .watchlist
                            .iter()
                            .map(|s| match s {
                                MatrixSource::Topic(t) => t.clone(),
                                MatrixSource::Glob(p) => format!("{}/*", p),
                            })
                            .collect();
                        self.config.presets.insert(name.clone(), topics);
                        if let Err(e) = self.config.save() {
                            self.toast(ToastKind::Error, format!("save config: {}", e));
                        }
                        self.toast(ToastKind::Success, format!("Preset '{}' saved", name));
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
        let presets = self.preset_list();
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
                if let Some((name, topics)) = presets.get(idx) {
                    self.mode = Mode::Normal;
                    self.load_preset_topics(name, topics);
                }
            }
            KeyCode::Enter => {
                if let Some((name, topics)) = presets.get(self.picker_cursor).cloned() {
                    self.mode = Mode::Normal;
                    self.load_preset_topics(&name, &topics);
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
    let port = crate::nt::client::NT_PORT;
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
    use crate::nt::store::NtValue;

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
