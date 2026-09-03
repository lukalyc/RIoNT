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
    ReconnectNt,
    RestartRobotCode,
    CopyTopicPath,
}

pub const COMMANDS: [(Command, &str); 10] = [
    (Command::SettingsOpen, "Settings: Open Configuration"),
    (Command::SettingsView, "Settings: View Settings"),
    (Command::SettingsAddTarget, "Settings: Add Robot Target"),
    (Command::SettingsRemoveTarget, "Settings: Remove Robot Target"),
    (Command::WatchlistSavePreset, "Watchlist: Save Active as Preset"),
    (Command::WatchlistLoadPreset, "Watchlist: Load Preset"),
    (Command::WatchlistClear, "Watchlist: Clear All"),
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

/// What the main loop should do after a keypress.
#[derive(Debug)]
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
    pub watchlist_cursor: usize,
    /// Scroll offset (card rows) adjusted by the renderer to keep the cursor
    /// visible; clamped defensively here too.
    pub watchlist_scroll: usize,

    // modes
    pub mode: Mode,
    pub query: String,
    pub search_matches: Vec<usize>, // indices into search_all
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

    // reconnect attempts since the last successful connection
    pub retry_attempt: u32,
}

impl App {
    pub fn new(target: String) -> Self {
        let (config, config_err) = crate::config::Config::load();
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
            retry_attempt: 0,
        };
        if let Some(e) = config_err {
            app.toast(
                ToastKind::Warn,
                format!("config invalid, using defaults ({})", e),
            );
        }
        app
    }

    // ------------------------------------------------------------------
    // Update intake (from client task)
    // ------------------------------------------------------------------

    pub fn apply_values(&mut self, batch: Vec<(String, NtValue, u64)>, now: std::time::Instant) {
        for (name, v, ts) in &batch {
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

    /// Drop toasts older than the TTL.
    pub fn prune_toasts(&mut self) {
        self.toasts
            .retain(|t| t.born.elapsed().as_millis() < TOAST_TTL_MS);
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
        rows.get(self.tree_cursor).map(|r| (r.path.clone(), r.is_topic))
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
            self.watchlist.retain(|s| s != &MatrixSource::Glob(path.clone()));
            self.toast(ToastKind::Info, format!("unpinned {}/*", path));
        } else {
            self.watchlist.push(MatrixSource::Glob(path.clone()));
            self.toast(ToastKind::Success, format!("pinned {}/* (subtree)", path));
        }
        self.clamp_watchlist_cursor();
    }

    fn unpin_topic(&mut self, topic: &str) {
        self.watchlist.retain(|s| match s {
            MatrixSource::Topic(t) => t != topic,
            MatrixSource::Glob(p) => !(topic == p || topic.starts_with(&format!("{}/", p))),
        });
        self.toast(ToastKind::Info, format!("unpinned {}", topic));
    }

    pub fn clamp_watchlist_cursor(&mut self) {
        let n = self.watchlist_cells().len();
        self.watchlist_cursor = self.watchlist_cursor.min(n.saturating_sub(1));
    }

    /// Expand the watchlist sources into the concrete topic list. Globs are
    /// re-expanded on every call, so topics the robot publishes later are
    /// adopted automatically.
    pub fn watchlist_cells(&self) -> Vec<String> {
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
            Focus::Tree => self.cursor_path().and_then(|(p, is_topic)| {
                if is_topic {
                    Some(p)
                } else {
                    None
                }
            }),
            Focus::Watchlist => self.watchlist_cells().get(self.watchlist_cursor).cloned(),
        }
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) -> UiAction {
        // Ctrl-C always quits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return UiAction::Quit;
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
            // Enter are aliases so the fold direction never has to be
            // memorized.
            KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Left | KeyCode::Right
            | KeyCode::Enter => {
                self.toggle_fold();
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
            // Only Tab/Esc leave the empty canvas.
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
                // Dismiss the active card (direct removal per spec).
                if let Some(topic) = cells.get(c).cloned() {
                    self.unpin_topic(&topic);
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
            _ => {}
        }
        self.watchlist_cursor = c;
        UiAction::None
    }

    fn begin_edit_topic(&mut self, topic: String) {
        self.mode = Mode::Edit;
        self.edit_topic = Some(topic);
        self.edit_input.clear();
        self.edit_error = None;
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
                    format!("no preset {} ({} loaded from config)", idx + 1, presets.len()),
                );
            }
        }
    }

    fn load_preset_topics(&mut self, name: &str, topics: &[String]) {
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
        self.toast(ToastKind::Success, format!("preset {}: {} card(s)", name, n));
    }

    fn handle_search(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char(' ') | KeyCode::Tab => {
                // Pin the highlighted match to the watchlist directly.
                if let Some(&idx) = self.search_matches.get(self.search_cursor) {
                    let names = self.store.sorted_names();
                    if let Some(topic) = names.get(idx).cloned() {
                        if self.is_pinned(&topic) {
                            self.unpin_topic(&topic);
                        } else {
                            self.watchlist.push(MatrixSource::Topic(topic.clone()));
                            self.toast(ToastKind::Success, format!("pinned {}", topic));
                        }
                    }
                }
                let n = self.search_matches.len();
                self.search_cursor = (self.search_cursor + 1).min(n.saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(&idx) = self.search_matches.get(self.search_cursor) {
                    let names = self.store.sorted_names();
                    if let Some(topic) = names.get(idx).cloned() {
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
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.search_cursor = (self.search_cursor + 1)
                    .min(self.search_matches.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
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
            KeyCode::Down | KeyCode::Char('j') => {
                self.palette_cursor = (self.palette_cursor + 1)
                    .min(self.palette_matches.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
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
                self.toast(ToastKind::Info, format!("reconnecting to {}...", self.target));
                UiAction::Client(ClientCommand::Reconnect)
            }
            Command::WatchlistClear => {
                let n = self.watchlist.len();
                self.watchlist.clear();
                self.watchlist_cursor = 0;
                self.watchlist_scroll = 0;
                self.toast(ToastKind::Success, format!("watchlist cleared ({} card(s))", n));
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
                    self.toast(ToastKind::Warn, "no presets configured (save one with 'Watchlist: Save Active as Preset')");
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
                let host = self.target.split(':').next().unwrap_or(&self.target).to_string();
                let user = self.config.system.ssh_user.clone();
                let restart = self.config.system.restart_cmd.clone();
                self.toast(
                    ToastKind::Info,
                    format!("restart: ssh {}@{}...", user, host),
                );
                UiAction::Client(ClientCommand::RestartRobotCode { host, user, cmd: restart })
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
                        self.toast(
                            ToastKind::Success,
                            format!("published {} = {}", topic, v.format()),
                        );
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
    /// Single purpose — connect. Digits 1-9 quick-jump, Enter confirms,
    /// Esc cancels. No add/edit/delete here (Settings owns management).
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
            // Quick jump: digit selects AND connects immediately — but only
            // while the input is empty, so typed addresses starting with a
            // digit are not hijacked.
            // Digits are ordinary input here: addresses start with digits,
            // so quick-jump-on-digit would hijack free-text typing. Selection
            // moves with j/k/arrows; Enter connects input or selection.
            KeyCode::Down | KeyCode::Char('j') => {
                self.connect_cursor = (self.connect_cursor + 1).min(targets.len().saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('k') => {
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
                                (input[..input.len() - rest.len()].trim().to_string(), rest.trim().to_string())
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
                        self.config.saved_targets.push(SavedTarget { name: name.clone(), ip: addr });
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
        if self.query.is_empty() {
            self.search_matches = (0..self.store.topics.len()).collect();
            return;
        }
        let matcher = SkimMatcherV2::default();
        let names = self.store.sorted_names();
        let mut scored: Vec<(i64, usize)> = names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| matcher.fuzzy_match(n, &self.query).map(|s| (s, i)))
            .collect();
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        self.search_matches = scored.into_iter().map(|(_, i)| i).collect();
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
                matcher.fuzzy_match(name, &self.palette_query).map(|s| (s, i))
            })
            .collect();
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
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
        Some(NtType::Boolean) | None if s.eq_ignore_ascii_case("true")
            || s.eq_ignore_ascii_case("false") =>
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
