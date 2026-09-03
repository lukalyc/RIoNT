//! Application state + vim-style input handling. Pure logic, no rendering.

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
    Connect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Inspector,
}

#[derive(Debug, Clone)]
pub enum DiffRow {
    Changed(String, String, String),
    Added(String, String),
    Removed(String, String),
}

/// How the status-bar message should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Warn,
    Error,
}

/// What the main loop should do after a keypress.
#[derive(Debug)]
pub enum UiAction {
    Quit,
    None,
    Client(ClientCommand),
}

pub struct App {
    pub store: Store,
    pub target: String,

    // connection
    pub connected: bool,
    pub connecting: bool,
    pub server_info: String,
    pub rtt_ms: Option<f64>,
    pub clock_offset_us: Option<f64>,
    pub first_server_ts: Option<u64>,
    pub last_server_ts: Option<u64>,
    pub disconnect_reason: Option<String>,

    // navigation
    pub expanded: HashSet<String>,
    pub tree_cursor: usize,
    pub inspector_scroll: usize,
    pub focus: Focus,

    // modes
    pub mode: Mode,
    pub query: String,
    pub search_matches: Vec<usize>, // indices into search_all
    pub search_cursor: usize,
    pub edit_topic: Option<String>,
    pub edit_input: String,
    pub edit_error: Option<String>,
    pub connect_input: String,

    // snapshot / diff
    pub snapshot: Option<std::collections::HashMap<String, String>>,
    pub diff: Vec<DiffRow>,
    pub diff_cursor: usize,

    // misc
    pub status: String,
    pub status_kind: StatusKind,
    /// Reconnect attempts since the last successful connection (from the
    /// client task); 0 while connected or on the first try.
    pub retry_attempt: u32,
    pub last_key_ms: u128,
}

impl App {
    pub fn new(target: String) -> Self {
        App {
            store: Store::new(),
            target,
            connected: false,
            connecting: true,
            server_info: String::new(),
            rtt_ms: None,
            clock_offset_us: None,
            first_server_ts: None,
            last_server_ts: None,
            disconnect_reason: None,
            expanded: HashSet::new(),
            tree_cursor: 0,
            inspector_scroll: 0,
            focus: Focus::Tree,
            mode: Mode::Normal,
            query: String::new(),
            search_matches: Vec::new(),
            search_cursor: 0,
            edit_topic: None,
            edit_input: String::new(),
            edit_error: None,
            connect_input: String::new(),
            snapshot: None,
            diff: Vec::new(),
            diff_cursor: 0,
            status: String::new(),
            status_kind: StatusKind::Info,
            retry_attempt: 0,
            last_key_ms: 0,
        }
    }

    // ------------------------------------------------------------------
    // Update intake (from client task)
    // ------------------------------------------------------------------

    pub fn apply_values(&mut self, batch: Vec<(String, NtValue, u64)>, now: std::time::Instant) {
        for (name, v, ts) in batch {
            if self.first_server_ts.is_none() {
                self.first_server_ts = Some(ts);
            }
            if ts > self.last_server_ts.unwrap_or(0) {
                self.last_server_ts = Some(ts);
            }
            self.store.apply_value(&name, v, ts, now);
        }
    }

    pub fn set_connected(&mut self, info: String) {
        self.connected = true;
        self.connecting = false;
        self.server_info = info;
        self.disconnect_reason = None;
        self.retry_attempt = 0;
        self.say("connected");
    }

    pub fn set_disconnected(&mut self, reason: String) {
        self.connected = false;
        self.connecting = true;
        self.disconnect_reason = Some(reason.clone());
        self.say(format!("disconnected ({})", reason));
    }

    // ------------------------------------------------------------------
    // Status messages (kind drives status-bar color)
    // ------------------------------------------------------------------

    fn say(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_kind = StatusKind::Info;
    }

    fn say_warn(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_kind = StatusKind::Warn;
    }

    fn say_err(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_kind = StatusKind::Error;
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
    // Input
    // ------------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) -> UiAction {
        self.last_key_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        // Ctrl-C always quits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return UiAction::Quit;
        }

        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Search => self.handle_search(key),
            Mode::Edit => self.handle_edit(key),
            Mode::Connect => self.handle_connect(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Char('q') => return UiAction::Quit,

            KeyCode::Char('j') | KeyCode::Down => match self.focus {
                Focus::Tree => self.tree_down(),
                Focus::Inspector => {
                    // j/k scrolls the inspector, or steps through an open diff.
                    if self.diff.is_empty() {
                        self.inspector_scroll = self.inspector_scroll.saturating_add(1);
                    } else if self.diff_cursor + 1 < self.diff.len() {
                        self.diff_cursor += 1;
                    }
                }
            },
            KeyCode::Char('k') | KeyCode::Up => match self.focus {
                Focus::Tree => self.tree_up(),
                Focus::Inspector => {
                    if self.diff.is_empty() {
                        self.inspector_scroll = self.inspector_scroll.saturating_sub(1);
                    } else {
                        self.diff_cursor = self.diff_cursor.saturating_sub(1);
                    }
                }
            },
            KeyCode::Char('g') => match self.focus {
                Focus::Tree => self.tree_cursor = 0,
                Focus::Inspector => self.inspector_scroll = 0,
            },
            KeyCode::Char('G') => match self.focus {
                Focus::Tree => {
                    let n = crate::ui::tree::build_tree(&self.store, &self.expanded).len();
                    self.tree_cursor = n.saturating_sub(1);
                }
                Focus::Inspector => {}
            },

            // h toggles: collapsed dir unfolds, expanded dir folds. l and the
            // arrow keys are aliases so the fold direction never has to be
            // memorized.
            KeyCode::Char('h') | KeyCode::Char('l') | KeyCode::Left | KeyCode::Right => {
                self.toggle_fold();
            }

            KeyCode::Tab | KeyCode::BackTab => {
                // Two panes; tab toggles between them.
                self.focus = match self.focus {
                    Focus::Tree => Focus::Inspector,
                    Focus::Inspector => Focus::Tree,
                };
            }

            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some((topic, true)) = self.cursor_path() {
                    if self.is_writable(&topic) {
                        self.mode = Mode::Edit;
                        self.edit_topic = Some(topic);
                        self.edit_input.clear();
                        self.edit_error = None;
                    } else {
                        self.say_err("topic type not editable");
                    }
                }
            }

            KeyCode::Char('c') => {
                // Connect to a different robot (team number or host[:port]).
                self.mode = Mode::Connect;
                self.connect_input.clear();
            }

            KeyCode::Char('/') => {
                self.mode = Mode::Search;
                self.query.clear();
                self.search_matches.clear();
                self.search_cursor = 0;
                // Seed with every topic path.
                self.recompute_matches();
            }

            KeyCode::Char('s') => {
                self.snapshot = Some(self.store.snapshot());
                self.diff.clear();
                self.say(format!("snapshot taken ({} values)", self.store.snapshot().len()));
            }

            KeyCode::Char('d') => {
                match &self.snapshot {
                    Some(snap) => {
                        let now = self.store.snapshot();
                        let mut rows: Vec<DiffRow> = Vec::new();
                        for (k, old) in snap.iter() {
                            match now.get(k) {
                                Some(new) if new != old => {
                                    rows.push(DiffRow::Changed(k.clone(), old.clone(), new.clone()))
                                }
                                Some(_) => {}
                                None => rows.push(DiffRow::Removed(k.clone(), old.clone())),
                            }
                        }
                        for (k, new) in &now {
                            if !snap.contains_key(k) {
                                rows.push(DiffRow::Added(k.clone(), new.clone()));
                            }
                        }
                        rows.sort_by(|a, b| match (a.key(), b.key()) {
                            (x, y) => x.cmp(y),
                        });
                        self.say(format!("{} change(s) since snapshot", rows.len()));
                        self.diff_cursor = 0;
                        self.diff = rows;
                    }
                    None => self.say_warn("no snapshot yet (press s)"),
                }
            }

            KeyCode::Char('R') => {
                // Drop the connection and reconnect to the same target now
                // (handy when the robot came back after a code deploy).
                self.say(format!("reconnecting to {}...", self.target));
                return UiAction::Client(ClientCommand::Reconnect);
            }

            KeyCode::Esc => {
                if !self.diff.is_empty() {
                    self.diff.clear();
                    self.status.clear();
                }
            }

            _ => {}
        }
        UiAction::None
    }

    fn handle_search(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                // Jump to the selected match.
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
                        self.say(format!("-> {}", topic));
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
                match parse_value(&self.edit_input, self.store.topics.get(&topic).map(|t| t.data_type)) {
                    Ok(v) => {
                        self.mode = Mode::Normal;
                        self.edit_topic = None;
                        self.edit_input.clear();
                        self.edit_error = None;
                        self.say(format!("published {} = {}", topic, v.format()));
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

    fn handle_connect(&mut self, key: KeyEvent) -> UiAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.connect_input.clear();
            }
            KeyCode::Backspace => {
                self.connect_input.pop();
            }
            KeyCode::Enter => {
                let input = self.connect_input.trim().to_string();
                self.mode = Mode::Normal;
                self.connect_input.clear();
                if input.is_empty() {
                    return UiAction::None;
                }
                let target = resolve_target(&input);
                // Deliberate retarget, not a disconnect: clear the reason so
                // the next Connecting update owns the status line.
                self.disconnect_reason = None;
                self.target = target.clone();
                self.say(format!("connecting to {}...", target));
                return UiAction::Client(ClientCommand::Retarget(target));
            }
            KeyCode::Char(c) => {
                self.connect_input.push(c);
            }
            _ => {}
        }
        UiAction::None
    }

    // ------------------------------------------------------------------
    // helpers
    // ------------------------------------------------------------------

    fn tree_down(&mut self) {
        let n = crate::ui::tree::build_tree(&self.store, &self.expanded).len();
        if self.tree_cursor + 1 < n {
            self.tree_cursor += 1;
        }
    }

    fn tree_up(&mut self) {
        self.tree_cursor = self.tree_cursor.saturating_sub(1);
    }

    /// h toggles the directory under the cursor: collapsed unfolds, expanded
    /// folds. On a topic row, toggles the topic's parent directory.
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
        if self.expanded.remove(&dir) {
            self.say(format!("folded {}", dir));
        } else {
            self.expanded.insert(dir.clone());
            self.say(format!("unfolded {}", dir));
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
        crate::trace(&format!(
            "recompute query={:?} names={:?}",
            self.query,
            names.first(),
        ));
        let mut scored: Vec<(i64, usize)> = names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| matcher.fuzzy_match(n, &self.query).map(|s| (s, i)))
            .collect();
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        self.search_matches = scored.into_iter().map(|(_, i)| i).collect();
        self.search_cursor = 0;
    }
}

impl DiffRow {
    fn key(&self) -> &str {
        match self {
            DiffRow::Changed(k, _, _) => k,
            DiffRow::Added(k, _) => k,
            DiffRow::Removed(k, _) => k,
        }
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
