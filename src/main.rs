mod app;
mod config;
mod field;
mod nt;
mod pose;
mod ui;

#[cfg(test)]
mod tests_tui;

use nt::{channel, command_channel, run_client, NtUpdate};

use app::{App, UiAction};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::stdout;
use std::time::{Duration, Instant};

/// CLI: `riont 9986` (team number) or `riont 172.22.11.2` (direct IP).
#[derive(clap::Parser)]
#[command(
    name = "riont",
    version,
    about = "RIONT — Robot Inspection Over Network Tables"
)]
struct Cli {
    /// Team number, IP, or IP:port. Falls back to common tether addresses.
    target: Option<String>,
}

pub(crate) fn trace(msg: &str) {
    if std::env::var("RIONT_TRACE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("riont-trace.log")
        {
            let _ = writeln!(f, "{}", msg);
        }
    }
}

fn resolve_target(arg: Option<String>) -> String {
    match arg {
        Some(t) => crate::app::resolve_target(&t),
        // No argument: fall back to the last successfully connected target
        // (config.json), then the USB tether address.
        None => {
            let (config, _) = config::Config::load();
            config.last_target.unwrap_or_else(|| "172.22.11.2".into())
        }
    }
}

fn main() -> anyhow::Result<()> {
    trace("main start");
    let cli = <Cli as clap::Parser>::parse();
    let target = resolve_target(cli.target);
    trace(&format!("target resolved: {}", target));

    // Runtime + channels: UI never blocks on the network.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async_main(&target))
}

async fn async_main(target: &str) -> anyhow::Result<()> {
    trace("async_main entered");
    let target = target.to_string();
    let (update_tx, mut update_rx) = channel();
    let (cmd_tx, cmd_rx) = command_channel();
    let handle = tokio::spawn(run_client(
        target.clone(),
        update_tx,
        cmd_tx.clone(),
        cmd_rx,
    ));

    // Headless test mode: fixed viewport, keystroke script on stdin, ANSI
    // render on stdout. No console APIs required.
    let headless = std::env::var("RIONT_HEADLESS")
        .map(|v| v == "1")
        .unwrap_or(false);

    let (mut terminal, mut key_rx, key_tx_closed) = if headless {
        let (w, h) = std::env::var("RIONT_SIZE")
            .ok()
            .and_then(|s| {
                let mut it = s.split('x');
                Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
            })
            .unwrap_or((120u16, 36u16));
        trace("headless terminal");
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, w, h)),
            },
        )?;
        let (key_tx, key_rx) = tokio::sync::mpsc::unbounded_channel();
        // Keystroke script on stdin: one line per batch of keys, or
        // "sleep:<ms>" to pause. Lines are processed in order. Arrow keys
        // (needed since overlays navigate by arrows, not j/k) use the
        // tokens UP/DOWN/LEFT/RIGHT alongside TAB/RET/ESC/SPC.
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if let Some(ms) = line
                    .strip_prefix("sleep:")
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    std::thread::sleep(Duration::from_millis(ms));
                    continue;
                }
                if line == "resize" {
                    // not supported headless; ignore
                    continue;
                }
                for token in line.split('\u{1f}') {
                    trace(&format!("input token: {:?}", token));
                    match token {
                        "" => continue,
                        "TAB" | "RET" | "ESC" | "SPC" | "UP" | "DOWN" | "LEFT" | "RIGHT" => {
                            let code = match token {
                                "TAB" => KeyCode::Tab,
                                "RET" => KeyCode::Enter,
                                "ESC" => KeyCode::Esc,
                                "UP" => KeyCode::Up,
                                "DOWN" => KeyCode::Down,
                                "LEFT" => KeyCode::Left,
                                "RIGHT" => KeyCode::Right,
                                _ => KeyCode::Char(' '),
                            };
                            if key_tx
                                .send(KeyEvent::new(code, KeyModifiers::empty()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        word => {
                            for c in word.chars() {
                                let (code, mods) = if c.is_ascii_uppercase() {
                                    (KeyCode::Char(c), KeyModifiers::SHIFT)
                                } else {
                                    (KeyCode::Char(c), KeyModifiers::empty())
                                };
                                if key_tx.send(KeyEvent::new(code, mods)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });
        (terminal, key_rx, true)
    } else {
        trace("before raw mode");
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        trace("raw mode ok");
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend)?;
        trace("terminal ready");
        let (key_tx, key_rx) = tokio::sync::mpsc::unbounded_channel();
        // Keyboard events on a dedicated OS thread -> unbounded channel.
        std::thread::spawn(move || loop {
            match event::read() {
                Ok(Event::Key(k)) => {
                    // Only send press events (Windows sends release events too).
                    if k.kind == KeyEventKind::Press && key_tx.send(k).is_err() {
                        break;
                    }
                }
                Ok(Event::Resize(_, _)) => {
                    let _ = key_tx.send(KeyEvent::new(KeyCode::F(24), KeyModifiers::empty()));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        });
        (terminal, key_rx, false)
    };

    let mut app = App::new(target.clone());

    // ~120 fps ceiling; ratatui's diffing keeps redraws cheap. Actual work
    // happens only when something changed.
    let mut tick = tokio::time::interval(Duration::from_millis(8));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_draw = Instant::now();
    let mut last_forced_repaint = Instant::now();

    let result = loop {
        tokio::select! {
            // keyboard
            maybe_key = key_rx.recv() => {
                let key = match maybe_key {
                    Some(k) => k,
                    None => break Ok(()),
                };
                trace(&format!("key: {:?} mode-before {:?}", key, app.mode));
                if key.code == KeyCode::F(24) {
                    // resize marker: fall through, tick redraws anyway
                } else {
                    match app.handle_key(key) {
                        UiAction::Quit => break Ok(()),
                        UiAction::Client(cmd) => {
                            cmd_tx.send(cmd).ok();
                        }
                        UiAction::OpenEditor(path) => {
                            match run_editor(&path, &mut terminal, headless) {
                                Err(e) => {
                                    app.toast(app::ToastKind::Error, format!("editor: {}", e));
                                }
                                Ok(()) => {
                                    // The user just saved config.json in
                                    // $EDITOR: reload it, otherwise the
                                    // round-trip is a silent no-op until the
                                    // next launch.
                                    let (cfg, err) = config::Config::load();
                                    app.config = cfg;
                                    // A new walls_file/map may have been set
                                    // in the editor — re-resolve geometry.
                                    app.reload_field_map();
                                    match err {
                                        None => {
                                            app.toast(app::ToastKind::Success, "config reloaded");
                                        }
                                        Some(e) => {
                                            app.toast(
                                                app::ToastKind::Error,
                                                format!(
                                                    "config invalid: {} — fix before restarting",
                                                    e
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        UiAction::None => {}
                    }
                }
                trace(&format!("mode-after {:?} query {:?}", app.mode, app.query));
            }
            // network updates
            Some(update) = update_rx.recv() => {
                match update {
                    NtUpdate::Connecting { target, attempt } => {
                        app.retry_attempt = attempt;
                        app.connecting = true;
                        app.connected = false;
                        // After a disconnect, the DISCONNECTED state in the
                        // HUD stays up while the client retries.
                        let _ = target;
                    }
                    NtUpdate::Connected { server_info } => {
                        app.set_connected(server_info);
                        // Remember the target in config.json for the next
                        // run (Connection Picker MRU ordering).
                        app.config.last_target = Some(target.clone());
                        if let Err(e) = app.config.save() {
                            app.toast(app::ToastKind::Warn, format!("save config: {}", e));
                        }
                    }
                    NtUpdate::Disconnected(reason) => app.set_disconnected(reason),
                    NtUpdate::Values(batch) => {
                        let now = Instant::now();
                        app.apply_values(batch, now);
                    }
                    NtUpdate::TopicMeta { name, id, data_type, persistent, retained, type_str, struct_schema } => {
                        let t = app.store.ensure(&name);
                        if id != u64::MAX {
                            t.id = id;
                        }
                        if let Some(dt) = data_type {
                            t.data_type = dt;
                        }
                        if let Some(p) = persistent {
                            t.persistent = p;
                        }
                        if let Some(r) = retained {
                            t.retained = r;
                        }
                        if type_str.is_some() {
                            t.type_str = type_str;
                        }
                        if struct_schema.is_some() {
                            t.struct_schema = struct_schema;
                        }
                    }
                    NtUpdate::TopicRemoved(name) => {
                        app.store.topics.remove(&name);
                    }
                    NtUpdate::Toast { kind, msg } => app.toast(kind, msg),
                    // Per-topic RTT/clock measurements stay inside the client
                    // (needed for clock-synced publishes); the HUD is
                    // driver-station style and does not surface them.
                    NtUpdate::Rtt(_) | NtUpdate::ClockOffset(_) => {}
                }
            }
            // tick
            _ = tick.tick() => {
                app.prune_toasts();
                if last_draw.elapsed() >= Duration::from_millis(8) {
                    trace(&format!("draw mode={:?} topics={}", app.mode, app.store.topics.len()));
                    // Every 500 ms force a full repaint: ratatui only emits
                    // changed cells, so this keeps the screen honest if a
                    // terminal glitches, and guarantees a heartbeat.
                    if last_forced_repaint.elapsed() >= Duration::from_millis(500) {
                        terminal.current_buffer_mut().reset();
                        last_forced_repaint = Instant::now();
                    }
                    terminal.draw(|f| {
                        ui::draw(f, &mut app);
                    })?;
                    last_draw = Instant::now();
                }
            }
        }
    };

    // Teardown.
    handle.abort();
    if !key_tx_closed && !headless {
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
    }
    result
}

/// Suspend the TUI, open `path` in the user's editor ($VISUAL / $EDITOR,
/// falling back to notepad on Windows / vi elsewhere), then restore the
/// terminal. Blocking by design: the user is editing the config.
fn run_editor(
    path: &std::path::Path,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    headless: bool,
) -> anyhow::Result<()> {
    if headless {
        return Ok(()); // test mode: no interactive editor
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".into()
            } else {
                "vi".into()
            }
        });

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .map_err(|e| anyhow::anyhow!("launch {}: {}", editor, e));
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    terminal.clear()?;
    status?;
    Ok(())
}
