//! In-process TUI tests: keystrokes -> App state -> rendered buffer.
//!
//! This is the fast tier of the test pyramid (see README "Testing"):
//! everything the Python E2E harness verifies about *state and behavior*
//! is verified here deterministically in milliseconds, with no subprocess,
//! no network, no sleeps and no ANSI scraping:
//!
//! - keystrokes go straight to `App::handle_key`;
//! - NT updates go straight to `App::apply_values` / `set_connected`;
//! - rendering happens into a `ratatui::backend::TestBackend` buffer;
//! - publish side effects are asserted on the returned
//!   [`UiAction::Client`] command (the exact contract with the NT client
//!   task) instead of round-tripping through a socket.
//!
//! What is deliberately NOT tested here: the real socket, the real NT4
//! wire protocol, ANSI emission and terminal emulation. Those live in the
//! small contract-level E2E harness (`test/harness.py`).
//!
//! Hermeticity: `App::new_test` uses an in-memory default config, and
//! `RIONT_CONFIG` redirects the rare save-path (pin persistence, overlay
//! toggle) to a scratch file so the developer's real config.json is never
//! touched.

use crate::app::{App, MatrixSource, Mode, UiAction};
use crate::nt::store::NtValue;
use crate::nt::ClientCommand;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::sync::Once;

static HERMETIC_CONFIG: Once = Once::new();

/// Redirect `Config::path()` to a scratch dir so pin/save paths in tests
/// cannot clobber the developer's `~/.config/riont/config.json`. Uses the
/// cfg(test) OnceLock in config.rs — no process-env mutation, so parallel
/// test threads are safe.
fn hermetic_config() {
    HERMETIC_CONFIG.call_once(|| {
        let dir = std::env::temp_dir().join("riont-test-config");
        let _ = std::fs::create_dir_all(&dir);
        crate::config::set_test_path(dir.join("config.json"));
    });
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

struct Tui {
    app: App,
    term: Terminal<TestBackend>,
}

impl Tui {
    fn new() -> Self {
        hermetic_config();
        let app = App::new_test("127.0.0.1:5814".into());
        let term = Terminal::new(TestBackend::new(120, 36)).expect("test backend");
        Tui { app, term }
    }

    fn key(&mut self, code: KeyCode) -> UiAction {
        self.app.handle_key(key(code))
    }

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    /// Search-jump: `/` + query + Enter. Lands the tree cursor on the
    /// first match with its ancestors expanded (the deterministic way to
    /// position the cursor, same as the E2E harness uses).
    fn jump(&mut self, query: &str) {
        self.key(KeyCode::Char('/'));
        self.type_str(query);
        self.key(KeyCode::Enter);
    }

    fn pin_via_search(&mut self, query: &str) {
        self.key(KeyCode::Char('/'));
        self.type_str(query);
        self.key(KeyCode::Char(' '));
        self.key(KeyCode::Esc);
    }

    /// Simulate a connected server streaming values.
    fn connect(&mut self) {
        self.app.set_connected("127.0.0.1:5814".into());
    }

    /// Feed a batch of topic values through the normal update intake.
    fn feed(&mut self, batch: Vec<(&str, NtValue)>) {
        let now = std::time::Instant::now();
        self.app.apply_values(
            batch
                .into_iter()
                .map(|(n, v)| (n.to_string(), v, 1_000))
                .collect(),
            now,
        );
    }

    fn feed_battery(&mut self) {
        self.feed(vec![
            ("SmartDashboard/Battery Voltage", NtValue::Double(12.6)),
            ("SmartDashboard/kP", NtValue::Double(0.012)),
            ("Swerve/FrontLeft/Velocity", NtValue::Double(3.1)),
        ]);
    }

    /// Render and return the plain-text screen rows (trailing whitespace
    /// trimmed), like the E2E harness sees after ANSI decoding.
    fn render(&mut self) -> Vec<String> {
        let Tui { app, term, .. } = self;
        term.draw(|f| crate::ui::draw(f, app)).expect("test render");
        let buf = term.backend().buffer();
        let w = buf.area.width as usize;
        let mut lines = Vec::with_capacity(buf.area.height as usize);
        for y in 0..buf.area.height as usize {
            let mut line = String::new();
            for x in 0..w {
                let cell = &buf.content[y * w + x];
                line.push_str(cell.symbol());
            }
            lines.push(line.trim_end().to_string());
        }
        lines
    }

    fn text(&mut self) -> String {
        self.render().join("\n")
    }

    fn toast_text(&self) -> String {
        self.app
            .toasts
            .iter()
            .map(|t| format!("{:?}: {}", t.kind, t.msg))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

fn braille_chars(s: &str) -> usize {
    s.chars()
        .filter(|c| ('\u{2800}'..='\u{28ff}').contains(c))
        .count()
}

// ---------------------------------------------------------------------------
// HUD
// ---------------------------------------------------------------------------

#[test]
fn hud_online_shows_comm_code_uptime_and_cargo_version() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    let l0 = t.render().remove(0);
    assert!(l0.contains("COMM: ONLINE"), "{l0}");
    assert!(l0.contains("127.0.0.1"), "{l0}");
    assert!(l0.contains("CODE: RUNNING"), "{l0}");
    assert!(l0.contains("UPTIME: "), "{l0}");
    // Version comes from Cargo.toml at compile time — this test doubles as
    // the version-release check (the E2E harness asserts the same thing).
    assert!(
        l0.contains(&format!("RIONT v{}", env!("CARGO_PKG_VERSION"))),
        "{l0}"
    );
}

#[test]
fn hud_disconnect_shows_humanized_reason_not_raw_os_error() {
    let mut t = Tui::new();
    t.connect();
    t.app.set_disconnected(
        "connect: No connection could be made because the target machine \
         actively refused it. (os error 10061)"
            .into(),
    );
    let text = t.text();
    assert!(
        text.contains("DISCONNECTED") || text.contains("RECONNECTING"),
        "{text}"
    );
    assert!(text.contains("connection refused"), "{text}");
    // Humanized contract: raw OS error spam never reaches the HUD.
    assert!(!text.contains("os error"), "{text}");
}

#[test]
fn hud_stopped_when_robot_frames_go_quiet() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    // Age the last frame past CODE_STALE_MS (500 ms).
    t.app.last_value_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(1500));
    let l0 = t.render().remove(0);
    assert!(l0.contains("CODE: STOPPED"), "{l0}");
}

// ---------------------------------------------------------------------------
// Topic tree
// ---------------------------------------------------------------------------

#[test]
fn tree_starts_collapsed_and_search_expands_ancestors() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    let text = t.text();
    assert!(text.contains("[+] SmartDashboard"), "{text}");
    assert!(!text.contains("Battery Voltage"), "{text}");

    t.jump("batt");
    let text = t.text();
    assert!(text.contains("[-] SmartDashboard"), "{text}");
    assert!(text.contains("Battery Voltage"), "{text}");
    assert_eq!(t.app.mode, Mode::Normal);
    assert_eq!(t.app.focus, crate::app::Focus::Tree);
}

#[test]
fn tree_row_shows_value_type_and_rate_inline() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("batt");
    let lines = t.render();
    let row = lines
        .iter()
        .find(|l| l.contains("Battery Voltage"))
        .expect("expanded tree row")
        .clone();
    assert!(row.contains("double"), "{row}");
    assert!(row.contains("12.6"), "{row}");
    assert!(row.contains("Hz"), "{row}");
}

#[test]
fn j_and_g_move_the_tree_cursor() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("batt");
    let before = t.app.tree_cursor;
    t.key(KeyCode::Char('j'));
    assert!(t.app.tree_cursor > before, "j must move down");
    t.key(KeyCode::Char('g'));
    assert_eq!(t.app.tree_cursor, 0, "g must jump to top");
}

#[test]
fn h_folds_the_cursor_topic_back_into_its_dir() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("batt");
    t.key(KeyCode::Char('h'));
    let text = t.text();
    assert!(text.contains("[+] SmartDashboard"), "{text}");
    assert!(!text.contains("[-] SmartDashboard"), "{text}");
}

// ---------------------------------------------------------------------------
// Pinning / watchlist
// ---------------------------------------------------------------------------

#[test]
fn space_pins_topic_card_and_marks_the_tree() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("batt");
    let action = t.key(KeyCode::Char(' '));
    assert_eq!(action, UiAction::None);
    assert!(t.app.is_pinned("SmartDashboard/Battery Voltage"));
    let text = t.text();
    assert!(text.contains("WATCHLIST (1)"), "{text}");
    assert!(text.contains("SmartDashboard/Battery Voltage"), "{text}");
    // Success toast names the pinned path.
    let toasts = t.toast_text();
    assert!(
        toasts.contains("Success") && toasts.contains("pinned"),
        "{toasts}"
    );
}

#[test]
fn x_removes_the_active_card() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("batt");
    t.key(KeyCode::Char(' '));
    t.key(KeyCode::Tab); // focus watchlist
    assert_eq!(t.app.focus, crate::app::Focus::Watchlist);
    t.key(KeyCode::Char('x'));
    assert!(!t.app.is_pinned("SmartDashboard/Battery Voltage"));
    assert!(t.text().contains("WATCHLIST (0)"));
}

#[test]
fn palette_clear_empties_the_watchlist() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.pin_via_search("batt");
    t.key(KeyCode::Char(':'));
    t.type_str("clear");
    t.key(KeyCode::Enter);
    assert!(t.app.watchlist.is_empty(), "{:?}", t.app.watchlist);
}

#[test]
fn watchlist_save_persists_to_riont_config_not_the_user_file() {
    // Hermeticity regression: the pin path persists the watchlist. It must
    // land in the RIONT_CONFIG scratch file, never the real config.json.
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.pin_via_search("batt");
    let path = crate::config::Config::path();
    assert!(path.starts_with(std::env::temp_dir()), "{path:?}");
    let txt = std::fs::read_to_string(&path).expect("config written");
    assert!(txt.contains("Battery Voltage"), "{txt}");
}

// ---------------------------------------------------------------------------
// Edit + publish (the NT write contract)
// ---------------------------------------------------------------------------

#[test]
fn edit_publish_returns_client_command_with_parsed_value() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    assert_eq!(t.app.mode, Mode::Edit);
    t.type_str("0.05");
    let action = t.key(KeyCode::Enter);
    match action {
        UiAction::Client(ClientCommand::Publish { topic, value }) => {
            assert_eq!(topic, "SmartDashboard/kP");
            assert_eq!(value, NtValue::Double(0.05));
        }
        other => panic!("expected Publish command, got {other:?}"),
    }
    let toasts = t.toast_text();
    assert!(toasts.contains("published"), "{toasts}");
    assert!(toasts.contains("SmartDashboard/kP"), "{toasts}");
}

#[test]
fn edit_publish_while_offline_queues_honestly() {
    let mut t = Tui::new();
    t.feed_battery(); // no set_connected: offline
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    let action = t.key(KeyCode::Enter);
    // The command is still emitted (the client re-queues it), but the toast
    // must say QUEUED, not "published" — a pit crew acts on that word.
    assert!(matches!(
        action,
        UiAction::Client(ClientCommand::Publish { .. })
    ));
    let toasts = t.toast_text();
    assert!(toasts.contains("queued"), "{toasts}");
    assert!(!toasts.contains("published"), "{toasts}");
}

#[test]
fn edit_invalid_input_keeps_editor_open_and_publishes_nothing() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("abc");
    let action = t.key(KeyCode::Enter);
    assert_eq!(action, UiAction::None);
    assert_eq!(
        t.app.mode,
        Mode::Edit,
        "invalid input keeps the editor open"
    );
    assert!(t.app.edit_error.is_some());
}

#[test]
fn esc_cancels_the_editor_without_publishing() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    t.key(KeyCode::Esc);
    assert_eq!(t.app.mode, Mode::Normal);
    assert!(t.app.edit_topic.is_none());
}

#[test]
fn edit_rejects_non_writable_topic_types() {
    let mut t = Tui::new();
    t.connect();
    t.feed(vec![(
        "Swerve/Wheel Faults",
        NtValue::BooleanArray(vec![false, false, false, true]),
    )]);
    t.jump("faults");
    t.key(KeyCode::Char('e'));
    assert_eq!(
        t.app.mode,
        Mode::Normal,
        "boolean[] must not open the editor"
    );
    let toasts = t.toast_text();
    assert!(
        toasts.contains("Error") && toasts.contains("not editable"),
        "{toasts}"
    );
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

#[test]
fn palette_enter_runs_the_highlighted_entry() {
    let mut t = Tui::new();
    t.connect();
    t.key(KeyCode::Char(':'));
    assert_eq!(t.app.mode, Mode::Palette);
    t.key(KeyCode::Down); // cursor onto entry 1
    t.key(KeyCode::Enter);
    // Entry 1 is Settings: View Settings (read-only overlay).
    assert_eq!(t.app.mode, Mode::SettingsView);
}

#[test]
fn quit_key_returns_quit_action() {
    let mut t = Tui::new();
    t.connect();
    assert_eq!(t.key(KeyCode::Char('q')), UiAction::Quit);
}

// ---------------------------------------------------------------------------
// Connection picker
// ---------------------------------------------------------------------------

#[test]
fn picker_typed_target_retargets() {
    let mut t = Tui::new();
    t.connect();
    t.key(KeyCode::Char('c'));
    assert_eq!(t.app.mode, Mode::Connect);
    // Digits are ordinary input here — the whole point of the picker.
    t.type_str("127.0.0.1:5999");
    let action = t.key(KeyCode::Enter);
    match action {
        UiAction::Client(ClientCommand::Retarget(target)) => {
            assert_eq!(target, "127.0.0.1:5999");
        }
        other => panic!("expected Retarget, got {other:?}"),
    }
    assert!(
        t.app.store.topics.is_empty(),
        "retarget clears the old robot's topics"
    );
}

// ---------------------------------------------------------------------------
// Field cards (pose classification is exhaustively unit-tested in pose.rs;
// here we pin the render + sticky contract end to end)
// ---------------------------------------------------------------------------

#[test]
fn botpose_renders_a_field_card_that_survives_empty_estimates() {
    let mut t = Tui::new();
    t.connect();
    t.feed(vec![(
        "SmartDashboard/botpose_wpiblue",
        NtValue::DoubleArray(vec![2.0, 4.1, 0.0, 0.0, 0.0, 90.0]),
    )]);
    t.pin_via_search("botpose");
    let text = t.text();
    assert!(text.contains("botpose_wpiblue"), "{text}");
    assert!(
        braille_chars(&text) > 20,
        "field card must draw a braille field, got {} braille chars",
        braille_chars(&text)
    );
    // Camera loses its estimate: empty array. The card must STAY a field
    // card (sticky pose) instead of flickering back to a value card.
    t.feed(vec![(
        "SmartDashboard/botpose_wpiblue",
        NtValue::DoubleArray(vec![]),
    )]);
    assert!(t.app.is_field_card("SmartDashboard/botpose_wpiblue"));
}

#[test]
fn lookalike_pose_topic_stays_a_value_card() {
    let mut t = Tui::new();
    t.connect();
    t.feed(vec![(
        "SmartDashboard/targetpose",
        NtValue::DoubleArray(vec![1.0, 2.0, 0.0, 0.0, 0.0, 45.0]),
    )]);
    t.pin_via_search("targetpose");
    let text = t.text();
    assert!(text.contains("targetpose"), "{text}");
    assert_eq!(braille_chars(&text), 0, "lookalike must not draw a field");
    assert!(!t.app.is_field_card("SmartDashboard/targetpose"));
}

#[test]
fn overlay_members_collapse_into_one_watchlist_cell() {
    let mut t = Tui::new();
    t.connect();
    t.app.watchlist = vec![
        MatrixSource::Topic("SmartDashboard/botpose_wpiblue".into()),
        MatrixSource::Topic("SmartDashboard/targetpose".into()),
    ];
    t.feed(vec![
        (
            "SmartDashboard/botpose_wpiblue",
            NtValue::DoubleArray(vec![2.0, 4.1, 0.0, 0.0, 0.0, 90.0]),
        ),
        (
            "SmartDashboard/targetpose",
            NtValue::DoubleArray(vec![1.0, 2.0, 0.0, 0.0, 0.0, 45.0]),
        ),
    ]);
    // BOTH topics opt in to the overlay group — the group renders as ONE
    // composite card (first member owns the cell), matching the `o` key
    // flow exercised end-to-end in test/harness.py (T26l).
    for topic in [
        "SmartDashboard/botpose_wpiblue",
        "SmartDashboard/targetpose",
    ] {
        t.app.config.field.overlay_topics.push(topic.into());
    }
    t.app
        .config
        .field
        .force_pose_topics
        .push("SmartDashboard/targetpose".into());
    let cells = t.app.watchlist_cells();
    assert_eq!(
        cells.len(),
        1,
        "overlay group renders as ONE composite card"
    );
    assert_eq!(cells[0], "SmartDashboard/botpose_wpiblue");
}

// ---------------------------------------------------------------------------
// Presets (config-resident; the legacy .nt-views.json path is E2E-only)
// ---------------------------------------------------------------------------

#[test]
fn preset_digit_loads_watchlist_from_config_presets() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.app
        .config
        .presets
        .insert("Test".into(), vec!["SmartDashboard/kP".into()]);
    t.key(KeyCode::Char('1'));
    assert!(t
        .app
        .watchlist
        .contains(&MatrixSource::Topic("SmartDashboard/kP".into())));
    assert!(
        t.app.focus == crate::app::Focus::Watchlist,
        "preset focuses the watchlist"
    );
}
