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

use crate::app::{App, Focus, MatrixSource, Mode, UiAction};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use riont_nt4::ClientCommand;
use riont_store::store::NtValue;

/// Redirect THIS test thread's `Config::path()` to a private scratch dir.
/// Thread-local (see `Config::set_test_path`): parallel test threads never
/// share or race on a config file, and the host's `~/.config/riont` is
/// never touched.
fn hermetic_config() {
    let thread_dir = std::env::temp_dir()
        .join("riont-test-config")
        .join(format!("{:?}", std::thread::current().id()));
    std::fs::create_dir_all(&thread_dir).expect("scratch config dir");
    crate::config::set_test_path(thread_dir.join("config.json"));
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
        Self::with_size(120, 36)
    }

    fn with_size(cols: u16, rows: u16) -> Self {
        hermetic_config();
        let app = App::new_test("127.0.0.1:5814".into());
        let term = Terminal::new(TestBackend::new(cols, rows)).expect("test backend");
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
        self.feed_owned(
            batch
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect::<Vec<_>>(),
        );
    }

    /// Same as [`Self::feed`] with owned names (built-in loops).
    fn feed_owned(&mut self, batch: Vec<(String, NtValue)>) {
        let now = std::time::Instant::now();
        self.app
            .apply_values(batch.into_iter().map(|(n, v)| (n, v, 1_000)).collect(), now);
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
fn hud_online_shows_comm_code_runtime_and_cargo_version() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    let l0 = t.render().remove(0);
    assert!(l0.contains("COMM: ONLINE"), "{l0}");
    assert!(l0.contains("127.0.0.1"), "{l0}");
    assert!(l0.contains("CODE: RUNNING"), "{l0}");
    assert!(l0.contains("RUNTIME: "), "{l0}");
    // Version comes from Cargo.toml at compile time — this test doubles as
    // the version-release check (the E2E harness asserts the same thing).
    assert!(
        l0.contains(&format!("RIONT v{}", env!("CARGO_PKG_VERSION"))),
        "{l0}"
    );
}

#[test]
fn runtime_restarts_from_zero_on_reconnect() {
    let mut t = Tui::new();
    t.connect();
    // Age the session past an hour: the HUD must show it.
    t.app.connected_since = Some(std::time::Instant::now() - std::time::Duration::from_secs(3661));
    let l0 = t.render().remove(0);
    assert!(l0.contains("RUNTIME: 01:01:01"), "{l0}");

    // Disconnect freezes the counter at the reached value...
    t.app.set_disconnected("ws closed".into());
    let l0 = t.render().remove(0);
    assert!(l0.contains("RUNTIME: 01:01:01"), "{l0}");

    // ...and reconnect restarts it from zero, not from the frozen value.
    t.connect();
    let l0 = t.render().remove(0);
    assert!(l0.contains("RUNTIME: 00:00:00"), "{l0}");
}

#[test]
fn runtime_is_blank_before_the_first_connection() {
    let mut t = Tui::new();
    let l0 = t.render().remove(0);
    assert!(l0.contains("RUNTIME: --:--:--"), "{l0}");
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

#[test]
fn code_running_while_the_server_answers_the_ping_despite_quiet_frames() {
    // Regression: NT4 pushes only CHANGED values. A running robot whose
    // telemetry happens to be static (arm parked, no motion) goes quiet
    // for seconds while its code is perfectly alive — the old heuristic
    // reported CODE: STOPPED against a green Driver Station. The 1 s
    // RTT echo is an application-level ping answered BY the robot's
    // ntcore server (which lives inside the robot program), so a fresh
    // echo proves the code is running even with zero changed values.
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.app.last_value_at =
        Some(std::time::Instant::now() - std::time::Duration::from_millis(30_000));
    t.app.note_rtt(std::time::Instant::now());
    let l0 = t.render().remove(0);
    assert!(l0.contains("CODE: RUNNING"), "{l0}");
}

#[test]
fn code_stopped_when_frames_and_ping_are_both_stale() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.app.last_value_at =
        Some(std::time::Instant::now() - std::time::Duration::from_millis(30_000));
    t.app.last_rtt_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(30_000));
    let l0 = t.render().remove(0);
    assert!(l0.contains("CODE: STOPPED"), "{l0}");
}

#[test]
fn hud_while_offline_names_the_target_it_is_trying() {
    // Field report: after a crash RIONT retried a stale target and the
    // operator could not tell WHICH one — "thought it was connected to
    // the simulation". The HUD must name the target whenever offline.
    let mut t = Tui::new();
    t.app.set_disconnected("connect timeout".into());
    let l0 = t.render().remove(0);
    assert!(l0.contains("127.0.0.1"), "{l0}");
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
    assert!(
        t.app.is_pinned("SmartDashboard/Battery Voltage"),
        "pin must succeed before persistence is checked"
    );
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

// Edit confirmation: local echo + engine read-back verification (the
// server does NOT echo a client's own publish back, so the display must
// update locally, and the engine verifies the write with a read-back over
// a second connection; the app only reacts to its verdict).

#[test]
fn edit_applies_local_echo() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    let action = t.key(KeyCode::Enter);
    assert!(matches!(
        action,
        UiAction::Client(ClientCommand::Publish { .. })
    ));
    // The tree and inspector read the store: it must already hold the
    // edited value, not the pre-edit one.
    assert_eq!(
        t.app.store.topics["SmartDashboard/kP"].current,
        Some(NtValue::Double(0.05))
    );
}

#[test]
fn edit_readback_confirmed_silently() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    t.key(KeyCode::Enter);
    // The engine read the topic back and the server holds the written
    // value: no extra toast.
    t.app.on_publish_verified(
        "SmartDashboard/kP",
        &NtValue::Double(0.05),
        &Some(NtValue::Double(0.05)),
    );
    let toasts = t.toast_text();
    assert!(!toasts.contains("not confirmed"), "{toasts}");
}

#[test]
fn edit_overridden_by_robot_warns() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    t.key(KeyCode::Enter);
    // Read-back returned a DIFFERENT value: the robot overrode the write.
    t.app.on_publish_verified(
        "SmartDashboard/kP",
        &NtValue::Double(0.05),
        &Some(NtValue::Double(0.42)),
    );
    let toasts = t.toast_text();
    assert!(toasts.contains("not confirmed"), "{toasts}");
    assert!(toasts.contains("0.4200"), "{toasts}");
}

#[test]
fn edit_without_readback_warns() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    t.key(KeyCode::Enter);
    // No value could be read back within the window.
    t.app
        .on_publish_verified("SmartDashboard/kP", &NtValue::Double(0.05), &None);
    let toasts = t.toast_text();
    assert!(toasts.contains("not confirmed"), "{toasts}");
}

#[test]
fn edit_fuzzy_readback_still_confirms() {
    let mut t = Tui::new();
    t.connect();
    t.feed_battery();
    t.jump("kp");
    t.key(KeyCode::Char('e'));
    t.type_str("0.05");
    t.key(KeyCode::Enter);
    // A server rounding the value through f32 must not count as failure.
    t.app.on_publish_verified(
        "SmartDashboard/kP",
        &NtValue::Double(0.05),
        &Some(NtValue::Double(0.05_f32 as f64)),
    );
    let toasts = t.toast_text();
    assert!(!toasts.contains("not confirmed"), "{toasts}");
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
// Inspector dock: undecoded struct topics (schema + hex view)
// ---------------------------------------------------------------------------

/// Simulate TopicMeta intake for an undecoded struct topic: the wire type
/// is binary (`struct:*` type_str) with an advertised structSchema.
fn feed_struct_topic(t: &mut Tui, name: &str, schema: Option<&str>, bytes: Vec<u8>) {
    t.connect();
    let topic = t.app.store.ensure(name);
    topic.type_str = Some("struct:SwerveModuleState".into());
    topic.struct_schema = schema.map(|s| s.to_string());
    t.feed_owned(vec![(name.to_string(), NtValue::Raw(bytes))]);
    t.jump("SwerveModuleState");
}

#[test]
fn inspector_undecoded_struct_shows_schema_leaves_and_hex() {
    let mut t = Tui::new();
    feed_struct_topic(
        &mut t,
        "Swerve/FrontLeft/SwerveModuleState",
        Some(
            "SwerveModuleState{angle:Rotation2d{radians:double}, \
              speedMetersPerSecond:double}",
        ),
        vec![
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
        ],
    );
    let text = t.text();
    // The dock must show parsed schema leaves (flattened, ordered) ...
    assert!(text.contains("radians"), "{text}");
    assert!(text.contains("speedMetersPerSecond"), "{text}");
    // ... the <N bytes> value line ...
    assert!(text.contains("<24 bytes>"), "{text}");
    // ... and the hex view: offset column + the raw byte pairs.
    assert!(text.contains("0000 11 22 33 44 55 66 77 88"), "{text}");
    assert!(text.contains("0008 99 aa bb cc dd ee ff 01"), "{text}");
}

#[test]
fn inspector_undecoded_struct_without_schema_still_shows_hex() {
    let mut t = Tui::new();
    feed_struct_topic(
        &mut t,
        "Swerve/FrontLeft/SwerveModuleState",
        None,
        vec![0xde, 0xad, 0xbe, 0xef],
    );
    let text = t.text();
    assert!(text.contains("0000 de ad be ef"), "{text}");
    assert!(!text.contains("Schema:"), "{text}");
}

#[test]
fn inspector_malformed_schema_degrades_to_raw_string() {
    let mut t = Tui::new();
    feed_struct_topic(
        &mut t,
        "Swerve/FrontLeft/SwerveModuleState",
        Some("SwerveModuleState{angle:Rotation2d"),
        vec![0x01],
    );
    let text = t.text();
    // Raw schema string, no leaf lines, and the hex view still present.
    // (The line is left-ellipsized to the dock width: assert on the tail.)
    assert!(text.contains("angle:Rotation2d"), "{text}");
    assert!(!text.contains("radians"), "{text}");
    assert!(text.contains("0000 01"), "{text}");
}

#[test]
fn inspector_decoded_struct_keeps_plain_value_no_hex() {
    let mut t = Tui::new();
    t.connect();
    // struct:Pose2d decodes on intake (see pose::decode_pose2d) — the
    // dock must keep the plain pose display, no schema/hex section.
    let topic = t.app.store.ensure("odometry/pose");
    topic.type_str = Some("struct:Pose2d".into());
    topic.struct_schema =
        Some("Pose2d{Translation2d{x:double, y:double}, Rotation2d{radians:double}}".into());
    let mut b = Vec::new();
    b.extend_from_slice(&1.5f64.to_le_bytes());
    b.extend_from_slice(&(-2.5f64).to_le_bytes());
    b.extend_from_slice(&0.0f64.to_le_bytes());
    t.feed_owned(vec![("odometry/pose".to_string(), NtValue::Raw(b))]);
    t.jump("odometry/pose");
    let text = t.text();
    assert!(text.contains("(1.50 m, -2.50 m"), "{text}");
    assert!(!text.contains("0000 "), "{text}");
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

// ---------------------------------------------------------------------------
// Robot glyph (field-card robot marker)
// ---------------------------------------------------------------------------

/// The size ladder, frozen: rect+arrow while the footprint spans >= 8
/// braille dots, rect+tick down to 3.5, chevron below (operator's pick —
/// see `robot_style_for`).
#[test]
fn robot_style_ladder() {
    use crate::ui::robot_style_for;
    // dots = robot_length_m * dots-per-meter.
    assert_eq!(robot_style_for(10.0), "A");
    assert_eq!(robot_style_for(8.0), "A");
    assert_eq!(robot_style_for(7.9), "B");
    assert_eq!(robot_style_for(5.2), "B"); // their small-card screenshot
    assert_eq!(robot_style_for(3.5), "B");
    assert_eq!(robot_style_for(3.4), "E");
    assert_eq!(robot_style_for(2.7), "E"); // dpm 3.0 × 0.9 m robot
}

/// Render one robot glyph alone on a blank canvas; returns the trimmed
/// text rows (the actual braille the canvas emits).
fn render_glyph(style: &str, dpm: f64) -> Vec<String> {
    use ratatui::{style::Color, symbols::Marker, widgets::canvas::Canvas};
    let (cols, rows) = (12usize, 5usize);
    let mut term = Terminal::new(TestBackend::new(cols as u16, rows as u16)).expect("backend");
    term.draw(|f| {
        let w = f.area().width as f64;
        let h = f.area().height as f64;
        let canvas = Canvas::default()
            .x_bounds([0.0, w * 2.0 / dpm])
            .y_bounds([0.0, h * 4.0 / dpm])
            .marker(Marker::Braille)
            .paint(|ctx| {
                crate::ui::draw_robot_style(
                    ctx,
                    (w * 2.0 / 2.0 / dpm, h * 4.0 / 2.0 / dpm),
                    30f64.to_radians(),
                    Color::Cyan,
                    0.9,
                    0.9,
                    style,
                );
            });
        f.render_widget(canvas, f.area());
    })
    .expect("draw");
    let buf = term.backend().buffer();
    (0..rows)
        .map(|y| {
            (0..cols)
                .map(|x| buf.content[y * cols + x].symbol().to_string())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// Snapshot of the surviving glyphs at the sizes from the design review
/// (heading 30°, 0.9 × 0.9 m robot). If one of these breaks, the glyph
/// changed — re-review it visually before updating the strings.
/// The field-card pane text (everything from the watchlist border right).
fn pane(t: &mut Tui) -> String {
    t.text()
        .lines()
        .map(|l| l.chars().skip(43).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn robot_glyph_snapshots() {
    // Print the actual glyphs once if the snapshot drifts:
    // cargo test robot_glyph_snapshots -- --nocapture
    for style in ["B", "E"] {
        let dpm = if style == "B" { 5.2 } else { 3.0 };
        println!("{style} @ {dpm}: {:?}", render_glyph(style, dpm));
    }
    // B — rect + center->front tick, at the small-card resolution.
    assert_eq!(
        render_glyph("B", 5.2),
        vec![
            "\u{2864}\u{28c0}",
            "\u{283c}\u{28d0}\u{28a1}\u{2803}",
            "\u{2801}"
        ]
    );
    // E — chevron, at the extra-small resolution.
    assert_eq!(render_glyph("E", 3.0), vec!["\u{2820}\u{28b2}\u{2803}"]);
    // A — rect + arrow (head folding back INSIDE the frame), normal card.
    assert_eq!(
        render_glyph("A", 9.0),
        vec![
            "\u{2870}\u{2831}\u{28e6}\u{28c0}",
            "\u{289c}\u{2840}\u{2810}\u{2801}\u{2871}\u{2801}",
            "\u{2808}\u{2812}\u{281c}"
        ]
    );
}

// ---------------------------------------------------------------------------
// Field-card alliance mirror + swerve vectors + struct decoding expansion
// ---------------------------------------------------------------------------

/// Braille-only content of the rendered screen (the field canvas): colors
/// and chrome ignored, so renders compare as glyph geometry.
fn braille_screen(t: &mut Tui) -> Vec<String> {
    t.render()
        .into_iter()
        .map(|l| {
            l.chars()
                .filter(|c| ('\u{2800}'..='\u{28ff}').contains(c))
                .collect::<String>()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// Feed a Limelight-style botpose (x, y, yaw_deg) at the given topic.
fn feed_botpose(t: &mut Tui, name: &str, x: f64, y: f64, yaw_deg: f64) {
    t.feed(vec![(
        name,
        NtValue::DoubleArray(vec![x, y, 0.0, 0.0, 0.0, yaw_deg]),
    )]);
}

/// Feed raw struct bytes for `name`, tagging the topic with `type_str`
/// first so the store's struct decode dispatch picks it up (in prod the
/// announce metadata carries it; here the wire path is simulated).
fn feed_raw_struct(t: &mut Tui, name: &str, type_str: &str, bytes: Vec<u8>) {
    t.app.store.ensure(name).type_str = Some(type_str.into());
    t.feed(vec![(name, NtValue::Raw(bytes))]);
}

fn swerve_payload(states: &[(f64, f64)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (a, s) in states {
        b.extend_from_slice(&a.to_le_bytes());
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// Regression: red view at stored heading θ must render the robot glyph
/// EXACTLY like blue view at −θ (the glyph's heading unit vector is
/// (sin θ, cos θ) — f64::sin_cos returns (sin, cos) — which x-mirroring
/// negates; a (cos θ, sin θ) convention would give π − θ). The robot
/// POSITION was always mirrored via fx(); the heading was not, so
/// red-view robots faced the wrong way.
#[test]
fn red_alliance_heading_mirrors_exactly_like_blue_at_minus_theta() {
    use std::f64::consts::PI;
    let (x, y, theta) = (5.0f64, 4.0f64, 0.75f64 * PI);
    let (robot_len, robot_wid) = (4.0f64, 4.0f64);

    // Robot-only dot set: pane chars that differ once the pose estimate is
    // emptied (sticky card keeps rendering; only the glyph vanishes; the
    // trail dot is mirrored identically in both scenes).
    let robot_dots = |t: &mut Tui| {
        let with = pane(t);
        t.feed(vec![(
            "SmartDashboard/botpose_wpiblue",
            NtValue::DoubleArray(vec![]),
        )]);
        let without = pane(t);
        let mut dots: Vec<(usize, usize)> = Vec::new();
        let is_braille = |l: &str| l.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c));
        for (ri, (wr, br)) in with.lines().zip(without.lines()).enumerate() {
            // Canvas rows only: the card's meta row (rate/Δ) is
            // wall-clock dependent and carries no glyph information.
            if !is_braille(wr) {
                continue;
            }
            for (ci, (wc, bc)) in wr.chars().zip(br.chars()).enumerate() {
                if wc != bc {
                    dots.push((ri, ci));
                }
            }
        }
        dots.sort();
        dots
    };

    let scene = |t: &mut Tui, px: f64, heading: f64| {
        t.connect();
        t.app.config.field.robot_length_m = robot_len;
        t.app.config.field.robot_width_m = robot_wid;
        feed_botpose(
            t,
            "SmartDashboard/botpose_wpiblue",
            px,
            y,
            heading.to_degrees(),
        );
        t.pin_via_search("botpose");
    };

    let mut red = Tui::new();
    red.app.config.field.alliance = "red".into();
    scene(&mut red, x, theta);

    let mut blue_m = Tui::new();
    scene(&mut blue_m, x, -theta); // the apply_value below overwrites x
                                   // with the MIRRORED position (len − x)
    blue_m.app.store.apply_value(
        "SmartDashboard/botpose_wpiblue",
        NtValue::DoubleArray(vec![
            field_len(&blue_m) - x,
            y,
            0.0,
            0.0,
            0.0,
            (-theta).to_degrees(),
        ]),
        1_000,
        std::time::Instant::now(),
    );

    let mut blue_ref = Tui::new();
    scene(&mut blue_ref, x, theta);

    let red_dots = robot_dots(&mut red);
    let blue_mirrored_dots = robot_dots(&mut blue_m);
    let blue_plain_dots = robot_dots(&mut blue_ref);

    assert!(!blue_mirrored_dots.is_empty(), "robot glyph must render");
    // The mirror identity.
    assert_eq!(
        red_dots, blue_mirrored_dots,
        "red @ θ must equal blue @ −θ (position mirrored)"
    );
    // And it must be discriminative: the un-mirrored blue scene differs.
    assert_ne!(
        red_dots, blue_plain_dots,
        "red @ θ must differ from blue @ θ, or the test is vacuous"
    );
}

fn field_len(t: &Tui) -> f64 {
    t.app.field_map.length_m
}

#[test]
fn struct_chassis_speeds_renders_named_fields() {
    let mut t = Tui::new();
    t.connect();
    let mut b = Vec::new();
    b.extend_from_slice(&1.25f64.to_le_bytes());
    b.extend_from_slice(&0.0f64.to_le_bytes());
    b.extend_from_slice(&0.5f64.to_le_bytes());
    feed_raw_struct(&mut t, "Odometry/ChassisSpeeds", "struct:ChassisSpeeds", b);
    t.pin_via_search("ChassisSpeeds");
    let text = t.text();
    assert!(text.contains("vx 1.25"), "{text}");
    assert!(!text.contains("<24 bytes>"), "{text}");
}

/// Swerve module vectors (ROADMAP 4): with exactly one decoded
/// `struct:SwerveModuleStates` topic in the store, the field card draws
/// each module's vector off the robot footprint corners.
#[test]
fn swerve_vectors_draw_on_the_field_card() {
    let mut t = Tui::new();
    t.connect();
    feed_botpose(&mut t, "SmartDashboard/botpose_wpiblue", 5.0, 4.0, 0.0);
    t.pin_via_search("botpose");
    let baseline = braille_screen(&mut t);
    assert!(!baseline.is_empty());

    feed_raw_struct(
        &mut t,
        "Swerve/ModuleStates",
        "struct:SwerveModuleStates",
        swerve_payload(&[(0.3, 4.0), (-0.5, 4.0), (1.2, 4.0), (0.0, 4.0)]),
    );
    let with_vectors = braille_screen(&mut t);
    assert_ne!(
        with_vectors, baseline,
        "module vectors must change the canvas"
    );
}

/// The exactly-one-topic rule: with ZERO or MULTIPLE decoded
/// SwerveModuleStates topics, no vectors are drawn (ambiguous source).
#[test]
fn swerve_vectors_require_exactly_one_module_states_topic() {
    let mut t = Tui::new();
    t.connect();
    feed_botpose(&mut t, "SmartDashboard/botpose_wpiblue", 5.0, 4.0, 0.0);
    t.pin_via_search("botpose");
    let baseline = braille_screen(&mut t);

    // TWO candidate topics: the rule refuses both.
    for name in ["Swerve/FrontStates", "Swerve/RearStates"] {
        feed_raw_struct(
            &mut t,
            name,
            "struct:SwerveModuleStates",
            swerve_payload(&[(0.3, 4.0), (-0.5, 4.0), (1.2, 4.0), (0.0, 4.0)]),
        );
    }
    assert_eq!(
        braille_screen(&mut t),
        baseline,
        "two swerve topics must draw no vectors"
    );

    // Unpin one candidate (x via the watchlist cursor — simplest: drop the
    // second topic from the store) leaves exactly one: vectors appear.
    t.app.store.topics.remove("Swerve/RearStates");
    let with_vectors = braille_screen(&mut t);
    assert_ne!(with_vectors, baseline, "one swerve topic draws vectors");
}

/// Vectors are skipped entirely when the footprint renders as the compact
/// chevron (below the robot_style_for threshold: too small to read).
#[test]
fn swerve_vectors_skip_on_compact_glyphs() {
    let mut t = Tui::new();
    t.connect();
    t.app.config.field.robot_length_m = 0.25;
    t.app.config.field.robot_width_m = 0.25;
    feed_botpose(&mut t, "SmartDashboard/botpose_wpiblue", 5.0, 4.0, 0.0);
    t.pin_via_search("botpose");
    let baseline = braille_screen(&mut t);

    feed_raw_struct(
        &mut t,
        "Swerve/ModuleStates",
        "struct:SwerveModuleStates",
        swerve_payload(&[(0.3, 4.0), (-0.5, 4.0), (1.2, 4.0), (0.0, 4.0)]),
    );
    assert_eq!(
        braille_screen(&mut t),
        baseline,
        "compact glyphs must draw no vectors"
    );
}

// ---------------------------------------------------------------------------
// Watchlist vertical scroll (many pins: the cursor must never leave view)
// ---------------------------------------------------------------------------

/// Regression: with more pinned cards than fit in three columns, the
/// overflow was appended to the last column and silently CLIPPED while
/// j/k kept moving the cursor down into the invisible region — cards the
/// operator pinned were never visible again. The watchlist now scrolls
/// vertically to keep the cursor's card on screen.
#[test]
fn watchlist_scrolls_to_keep_the_cursor_card_visible() {
    let mut t = Tui::with_size(120, 16); // ~3 cards per column
    t.connect();
    t.app.watchlist = (0..12)
        .map(|i| MatrixSource::Topic(format!("Grp/Topic{}", i)))
        .collect();
    t.app.clamp_watchlist_cursor();
    t.app.focus = Focus::Watchlist; // j/k/G must drive the watchlist
    t.app.toasts.clear(); // the connect toast would overlay the pane

    // Card titles render as "┌ Grp/TopicN " in the watchlist (the tree
    // shows the same names in a different row format, so anchor on the
    // card border, and count occurrences per line).
    let card_titles = |t: &mut Tui| {
        // Two cards share a text line (side-by-side columns), so count
        // title OCCURRENCES, not lines.
        t.text()
            .lines()
            .flat_map(|l| {
                l.match_indices("┌ Grp/Topic")
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>()
            })
            .count()
    };

    let before = card_titles(&mut t);
    // Loose pins fill columns to the brim; three columns are visible at
    // this size (3 cards each) with the tail h-scrolled.
    assert_eq!(before, 9, "3 visible columns × 3 cards");

    // Jump to the LAST pinned card: it must scroll into view.
    t.key(KeyCode::Char('G'));
    assert!(
        t.text().contains("┌ Grp/Topic11 "),
        "last card must scroll into view:\n{}",
        t.text()
    );

    // And back to the first: no stale offset.
    t.key(KeyCode::Char('g'));
    assert!(
        t.text().contains("┌ Grp/Topic0 "),
        "first card must scroll back into view:\n{}",
        t.text()
    );

    // h/l into the other column restarts its vertical follow from the top.
    t.key(KeyCode::Char('h'));
    assert!(
        t.text().contains("┌ Grp/Topic0 "),
        "entering column 1 shows its top again:\n{}",
        t.text()
    );

    // j one step at a time across the fold: the followed card is visible.
    t.app.watchlist_cursor = 7;
    t.key(KeyCode::Char('j'));
    assert!(
        t.text().contains("┌ Grp/Topic8 "),
        "card 8 must be visible after stepping onto it:\n{}",
        t.text()
    );
}

// ---------------------------------------------------------------------------
// Watchlist grouping (folder pins get their own columns with headers)
// ---------------------------------------------------------------------------

/// Folder pins group into their own columns: a `─ Name (count) ─` header
/// tops each group's column, and two folders sit side by side (left |
/// right) instead of their cards jumbling together.
#[test]
fn watchlist_folder_pins_get_group_columns_with_headers() {
    let mut t = Tui::with_size(120, 16);
    t.connect();
    t.feed(vec![
        ("LeftShooter/RPM", NtValue::Double(1.0)),
        ("LeftShooter/Current", NtValue::Double(2.0)),
        ("RightShooter/RPM", NtValue::Double(3.0)),
        ("RightShooter/Current", NtValue::Double(4.0)),
    ]);
    t.app.watchlist = vec![
        MatrixSource::Glob("LeftShooter".into()),
        MatrixSource::Glob("RightShooter".into()),
    ];
    t.app.focus = Focus::Watchlist;
    t.app.toasts.clear();

    let text = t.text();
    assert!(text.contains("LeftShooter (2)"), "{text}");
    assert!(text.contains("RightShooter (2)"), "{text}");
    // Both headers on the SAME row, Left column before Right column.
    let row = text
        .lines()
        .find(|l| l.contains("LeftShooter (2)"))
        .expect("header row");
    let lpos = row.find("LeftShooter (2)").expect("left header");
    let rpos = row
        .find("RightShooter (2)")
        .expect("right header on same row");
    assert!(rpos > lpos, "{row}");
    // The right column renders RIGHT cards (no left spillover above them).
    assert!(text.contains("┌ RightShooter/RPM"), "{text}");
}

/// A group bigger than one column continues in the next column WITH its
/// header repeated — cards after a column break keep their context.
#[test]
fn watchlist_group_overflow_repeats_the_header() {
    let mut t = Tui::with_size(120, 16);
    t.connect();
    t.feed_owned(
        (0..5)
            .map(|i| (format!("BigFolder/T{}", i), NtValue::Double(i as f64)))
            .collect(),
    );
    t.app.watchlist = vec![MatrixSource::Glob("BigFolder".into())];
    t.app.focus = Focus::Watchlist;
    t.app.toasts.clear();

    let text = t.text();
    // 5 four-row cards at this height: 2 per column under the header ->
    // three columns, each topped by the group's header.
    assert_eq!(
        text.matches("BigFolder (5)").count(),
        3,
        "header repeats on EVERY overflow column: {text}"
    );
}

/// Mixed content: a folder group keeps its own header-topped column and
/// adjacent loose pins pack header-less — no cross-contamination either
/// way. And the header count is LIVE: a topic published under the glob
/// mid-session bumps `n` on the next frame.
#[test]
fn watchlist_mixed_pins_and_live_group_count() {
    let mut t = Tui::with_size(120, 16);
    t.connect();
    t.feed(vec![
        ("LeftShooter/RPM", NtValue::Double(1.0)),
        ("LeftShooter/Current", NtValue::Double(2.0)),
        ("Misc/Value", NtValue::Double(3.0)),
    ]);
    t.app.watchlist = vec![
        MatrixSource::Glob("LeftShooter".into()),
        MatrixSource::Topic("Misc/Value".into()),
    ];
    t.app.focus = Focus::Watchlist;
    t.app.toasts.clear();

    let text = t.text();
    assert!(text.contains("LeftShooter (2)"), "{text}");
    // Loose pin shares the second column, header-less.
    assert!(text.contains("┌ Misc/Value"), "{text}");

    // Robot publishes a new topic under the folder: the count is live.
    t.feed(vec![("LeftShooter/Velocity", NtValue::Double(5.0))]);
    let text = t.text();
    assert!(text.contains("LeftShooter (3)"), "{text}");
}

// ---------------------------------------------------------------------------
// Panic evidence logging (double-clicked .exe: stderr is lost forever)
// ---------------------------------------------------------------------------

/// The panic hook appends one record per panic to `<dir>/riont-debug.log`
/// (the same file the NT4 engine appends session logs to). The helper must
/// APPEND — a second panic in the same directory must never wipe the first
/// one's evidence — and must tolerate an unwritable directory silently.
#[test]
fn panic_evidence_appends_message_location_and_backtrace_to_debug_log() {
    let dir = std::env::temp_dir().join(format!(
        "riont-panic-evidence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    crate::append_panic_evidence(
        &dir,
        "index out of bounds: in watchlist packing",
        "src/ui/mod.rs:750:5",
        "frame 1\nframe 2",
    );
    crate::append_panic_evidence(&dir, "second panic", "src/main.rs:42:1", "frame A");

    let log = std::fs::read_to_string(dir.join("riont-debug.log")).expect("debug log written");
    assert!(
        log.contains("index out of bounds: in watchlist packing"),
        "{log}"
    );
    assert!(log.contains("src/ui/mod.rs:750:5"), "{log}");
    assert!(log.contains("frame 1\nframe 2"), "{log}");
    assert_eq!(
        log.matches("=== panic at").count(),
        2,
        "records append, never truncate: {log}"
    );

    // An unwritable target must be swallowed, not panic (the hook itself
    // must never panic).
    crate::append_panic_evidence(
        std::path::Path::new("/nonexistent-riont-test-dir"),
        "boom",
        "x:1:1",
        "bt",
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Watchlist width estimate (folder arrays must not overflow their columns)
// ---------------------------------------------------------------------------

/// Regression: a folder group of long double[] cards used to be
/// height-measured at a WIDER nominal width than its real rendered
/// columns — the group overflows into more columns than the one-pass
/// guess assumes, heights came out too small, columns overfilled and the
/// painter clipped the overflow cards. Packing now iterates to the real
/// column width, so every rendered card fits its column.
#[test]
fn watchlist_folder_of_long_arrays_never_overflows_its_columns() {
    let mut t = Tui::new(); // 120x36: 31 card rows under a group header
    t.connect();
    // 7 double[] elements render 69 chars: one line at full-pane width
    // (69 <= 72), two lines at a half-pane column (69 > 33) — so the
    // measured height differs between the wrong and the right width.
    t.feed_owned(
        (0..10)
            .map(|i| {
                (
                    format!("Wrap/T{}", i),
                    NtValue::DoubleArray(vec![1000.0; 7]),
                )
            })
            .collect(),
    );
    t.app.watchlist = vec![MatrixSource::Glob("Wrap".into())];
    t.app.focus = Focus::Watchlist;
    t.app.toasts.clear();

    let text = t.text();
    for i in 0..10 {
        assert!(
            text.contains(&format!("┌ Wrap/T{}", i)),
            "card {i} missing:\n{text}"
        );
    }
    // Every visible card fully drawn: 10 card top-left borders plus the
    // three pane blocks (tree, inspector, watchlist), and exactly as many
    // bottom-right corners — a card clipped by an overfilled column
    // renders ┌ without its ┘.
    let tops = text.matches('┌').count();
    let bottoms = text.matches('┘').count();
    assert_eq!(
        tops, bottoms,
        "a card was clipped by an overfilled column:\n{text}"
    );
    assert!(
        tops >= 13,
        "expected 10 cards + 3 pane corners: {tops}\n{text}"
    );
}
