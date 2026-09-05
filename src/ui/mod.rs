//! Layout + rendering for the dashboard.
//!
//! Geometry: a 35% left control column (Topic Tree 70% H + passive
//! Inspector Dock 30% H) and a 65% full-height Watchlist Canvas, under a
//! one-line Driver-Station HUD.
//!
//! Color rules (semantic neutrality):
//! - borders/chrome: dark grey, so data values carry the screen
//! - active pane border: cyan (watchlist) / amber (tree); inactive grey
//! - numbers: bright cyan; booleans: green TRUE / muted red FALSE;
//!   strings: warm amber; array brackets cyan with neutral values
//! - rates and Δ timers: muted grey — telemetry, never an alarm

pub mod tree;

use crate::app::{App, Focus, MatrixSource, Mode};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    symbols::Marker,
    widgets::canvas::{Canvas, Line as CanvasLine, Points},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// palette
// ---------------------------------------------------------------------------

const MUTED: Color = Color::Rgb(0x7C, 0x6F, 0x64); // rates, deltas, chrome text
const BORDER_GREY: Color = Color::Rgb(0x3C, 0x38, 0x36); // inactive borders
const MARK_GREY: Color = Color::Rgb(0x2E, 0x2B, 0x28); // field tape/game-line marks
/// Field-card palette: the perimeter must be visible on a near-black
/// background; obstacles are tinted by alliance half; the robot picks its
/// color from `FMSInfo/IsRedAlliance` (neutral cyan when absent).
const FIELD_PERIMETER: Color = Color::Rgb(0xB0, 0xA8, 0x9C);
const FIELD_BLUE: Color = Color::Rgb(0x45, 0x6C, 0x9E);
const FIELD_RED: Color = Color::Rgb(0x9E, 0x56, 0x45);
const ROBOT_BLUE: Color = Color::Rgb(0x5D, 0xA9, 0xFF);
const ROBOT_RED: Color = Color::Rgb(0xFF, 0x5A, 0x4D);
const AMBER: Color = Color::Rgb(0xFA, 0xBD, 0x2F); // tree focus / strings
const CYAN: Color = Color::Cyan; // numbers / watchlist focus / array brackets

fn bold(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().add_modifier(Modifier::BOLD))
}

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(MUTED))
}

fn muted(s: impl Into<String>) -> Span<'static> {
    dim(s)
}

fn rev(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().add_modifier(Modifier::REVERSED))
}

fn plain(s: impl Into<String>) -> Span<'static> {
    Span::raw(s.into())
}

fn ok(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::Green))
}

fn err(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::Red))
}

/// Terminal width (kept for headless/debug tooling).
#[allow(dead_code)]
pub fn term_width() -> u16 {
    crossterm::terminal::size().map(|(w, _)| w).unwrap_or(120)
}

// ---------------------------------------------------------------------------
// root layout
// ---------------------------------------------------------------------------

pub fn draw(f: &mut Frame, app: &mut App) {
    let root = f.area();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // HUD
            Constraint::Min(1),    // main
            Constraint::Length(1), // status bar
        ])
        .split(root);

    draw_hud(f, app, rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(rows[1]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(cols[0]);

    draw_tree(f, app, left[0]);
    draw_inspector(f, app, left[1]);
    draw_watchlist(f, app, cols[1]);
    draw_status(f, app, rows[2]);

    // overlays
    match app.mode {
        Mode::Search => draw_search(f, app),
        Mode::Edit => draw_edit(f, app),
        Mode::Connect => draw_connect(f, app),
        Mode::Palette => draw_palette(f, app),
        Mode::Prompt => draw_prompt(f, app),
        Mode::PickTarget => draw_pick_target(f, app),
        Mode::PickPreset => draw_pick_preset(f, app),
        Mode::SettingsView => draw_settings_view(f, app),
        Mode::Normal => {}
    }
    draw_toasts(f, app);
}

// ---------------------------------------------------------------------------
// HUD: driver-station diagnostics
// ---------------------------------------------------------------------------

fn draw_hud(f: &mut Frame, app: &App, area: Rect) {
    let ip = app.target.split(':').next().unwrap_or(&app.target).to_string();
    // Flash the reconnecting indicator at ~2 Hz (amber = retry loop).
    let blink_on = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_millis() < 500)
        .unwrap_or(true);
    let amber_style = Style::default().fg(AMBER).add_modifier(Modifier::BOLD);

    // The client already knows WHY the link dropped — surface the reason
    // and the retry count instead of blinking amber forever with no
    // diagnosis. State strings are left-ellipsized to the comm budget so
    // the diagnostic tail (the actionable part) survives narrow terminals.
    let reason = app.disconnect_reason.clone().unwrap_or_default();
    let comm_budget = (area.width as usize).saturating_sub(75).clamp(20, 48);
    let comm = if app.connected {
        Span::styled(
            format!("ONLINE ({})", ip),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )
    } else if app.retry_attempt > 0 {
        // Retry loop: attempt count plus the last failure reason. The state
        // keyword is rendered WHOLE — ellipsizing the full label could drop
        // "RECONNECTING" itself on long Windows error strings — and only
        // the reason is left-ellipsized into the remaining budget (the
        // actionable tail of long reasons survives).
        let kw = format!("RECONNECTING (attempt {})", app.retry_attempt);
        let label = if reason.is_empty() {
            kw
        } else {
            let rem = comm_budget.saturating_sub(kw.chars().count() + 1);
            format!("{} {}", kw, ellipsize_left(&reason, rem))
        };
        if blink_on {
            Span::styled(label, amber_style)
        } else {
            dim(label)
        }
    } else if !reason.is_empty() {
        // Just dropped, retry counter not yet ticking: the red state the
        // README promises, finally reachable — and carrying the cause.
        // Same keyword-first rule as above: "DISCONNECTED" never ellipsized.
        let rem = comm_budget.saturating_sub("DISCONNECTED".chars().count() + 3);
        Span::styled(
            format!("DISCONNECTED — {}", ellipsize_left(&reason, rem)),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    } else if app.connecting {
        // First connect: no reason yet, no attempt — plain amber.
        Span::styled("RECONNECTING...", amber_style)
    } else {
        Span::styled("DISCONNECTED", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    };

    let code = match app.code_running() {
        Some(true) => Span::styled(
            "RUNNING",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Some(false) => Span::styled(
            "STOPPED",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        None => dim("--"),
    };

    // Uptime from the robot's server clock: NT4 timestamps on a roboRIO are
    // FPGA microseconds since boot, so the latest value timestamp IS the
    // uptime once synced (epoch-based off-robot servers fall back to the
    // first→last delta). Rendered HH:MM:SS, no local-app wall clock.
    const EPOCH_US_CUTOFF: u64 = 500_000_000_000_000; // ~15.8 years of µs
    let uptime = match app.last_server_ts {
        Some(ts) if ts < EPOCH_US_CUTOFF => fmt_hms(ts / 1_000_000),
        Some(ts) => match app.first_server_ts {
            Some(first) if ts >= first => fmt_hms((ts - first) / 1_000_000),
            _ => "--:--:--".into(),
        },
        None => "--:--:--".into(),
    };

    let line = Line::from(vec![
        // Single source of truth: Cargo.toml's `version`, baked in at
        // compile time. Never hardcode a version string here.
        bold(format!("RIONT v{}", env!("CARGO_PKG_VERSION"))),
        dim("  [COMM: "),
        comm,
        dim("]"),
        dim("  [CODE: "),
        code,
        dim("]"),
        dim("  [UPTIME: "),
        plain(uptime),
        dim("]"),
        dim(format!("  {} topics", app.store.topics.len())),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn fmt_hms(total_secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", total_secs / 3600, (total_secs / 60) % 60, total_secs % 60)
}

/// Humanized Δ since last change: `20ms`, `14.2s`, `3.4m`, `1.2h`.
fn fmt_delta(secs: f64) -> String {
    if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 90.0 {
        format!("{:.1}s", secs)
    } else if secs < 5400.0 {
        format!("{:.1}m", secs / 60.0)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}

// ---------------------------------------------------------------------------
// topic tree (35% W, 70% H)
// ---------------------------------------------------------------------------

fn draw_tree(f: &mut Frame, app: &App, area: Rect) {
    let border = if app.focus == Focus::Tree {
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER_GREY)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(
            " TOPIC TREE ",
            if app.focus == Focus::Tree {
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(MUTED)
            },
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = tree::build_tree(&app.store, &app.expanded);
    let pinned = app_pinned(app);
    let globbed = app_glob_pinned(app);

    let height = inner.height as usize;
    let start = if rows.len() > height {
        app.tree_cursor
            .saturating_sub(height.saturating_sub(1) / 2)
            .min(rows.len() - height)
    } else {
        0
    };
    let end = (start + height).min(rows.len());

    let items: Vec<ListItem> = rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let idx = start + i;
            let indent = "  ".repeat(row.depth);
            let selected = idx == app.tree_cursor && app.focus == Focus::Tree;
            // Full row content (leaf, star, type tag, value, Hz) is built
            // once; the selected row reuses the SAME spans so nothing gets
            // clipped out of the highlight — the REVERSED modifier is
            // applied per-span so tag/value/rate all invert with the row.
            let mut spans: Vec<Span> = if row.is_topic {
                let td = app.store.topics.get(&row.path);
                let val = td
                    .and_then(|t| t.current.as_ref())
                    .map(|v| v.format())
                    .unwrap_or_else(|| "-".into());
                let ty = td.map(|t| t.data_type.as_str()).unwrap_or("?");
                let hz = td
                    .and_then(|t| t.hz())
                    .map(|h| format!("{:.0}Hz", h))
                    .unwrap_or_default();
                let leaf = row.path.rsplit('/').next().unwrap_or(&row.path);
                // Green type tag = editable (e works); dim = read-only type.
                let type_tag = if td.map(|t| t.data_type.is_writable()).unwrap_or(false) {
                    ok(format!(" [{}] ", ty))
                } else {
                    dim(format!(" [{}] ", ty))
                };
                // Amber bold star = direct pin; dim star = covered by a
                // subtree (glob) pin — so the tree shows inherited pins too
                // and Space/x on them is never a surprise.
                let star = if pinned.contains(&row.path) {
                    Span::styled("* ", Style::default().fg(AMBER).add_modifier(Modifier::BOLD))
                } else if globbed.contains(&row.path) {
                    Span::styled("* ", Style::default().fg(MUTED))
                } else {
                    plain("")
                };
                vec![
                    plain(indent.clone()),
                    star,
                    plain(leaf),
                    type_tag,
                    plain(val),
                    dim(format!(" {}", hz)),
                ]
            } else {
                vec![plain(indent.clone()), plain(&row.label)]
            };
            if selected {
                // Cursor marker takes the place of the first indent spaces;
                // REVERSED on every span inverts the entire row uniformly.
                spans[0] = bold("> ");
                for s in spans.iter_mut() {
                    s.style = s.style.add_modifier(Modifier::REVERSED);
                }
                ListItem::new(Line::from(spans))
                    .style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                ListItem::new(Line::from(spans))
            }
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn app_pinned(app: &App) -> std::collections::HashSet<String> {
    app.watchlist
        .iter()
        .filter_map(|s| match s {
            MatrixSource::Topic(t) => Some(t.clone()),
            MatrixSource::Glob(_) => None,
        })
        .collect()
}

/// Topics covered by a subtree (glob) pin — rendered with a dim star so
/// they are visually distinct from direct pins. Computed once per frame
/// alongside `app_pinned`.
fn app_glob_pinned(app: &App) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for s in &app.watchlist {
        if let MatrixSource::Glob(p) = s {
            let pfx = format!("{}/", p);
            for n in app.store.sorted_names() {
                if n.starts_with(&pfx) {
                    out.insert(n);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// inspector dock (35% W, 30% H) — strictly passive
// ---------------------------------------------------------------------------

fn draw_inspector(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_GREY))
        .title(Span::styled(" INSPECTOR ", Style::default().fg(MUTED)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let w = inner.width as usize;
    // Mirror the EFFECTIVE active topic: with focus on the watchlist the
    // tree cursor is stale, yet every active command (e/edit, copy path)
    // acts on the card — so the dock shows the card's topic there. The
    // dock stays strictly passive; only WHAT it mirrors changes.
    let (path, is_topic) = if app.focus == Focus::Watchlist {
        // Canvas rows are always topics; an empty canvas shows the
        // no-selection state.
        match app.active_topic() {
            Some(t) => (t, true),
            None => (String::new(), false),
        }
    } else {
        app.cursor_path()
            .unwrap_or_else(|| (String::new(), false))
    };

    let mut lines: Vec<Line> = Vec::new();
    if path.is_empty() {
        lines.push(Line::from(dim("(no topic selected)")));
    } else if !is_topic {
        lines.push(Line::from(vec![dim("Path:  "), plain(&path)]));
        lines.push(Line::from(dim("(directory)")));
    } else if let Some(t) = app.store.topics.get(&path) {
        lines.push(Line::from(vec![
            dim("Path:  "),
            plain(ellipsize_left(&path, w.saturating_sub(7))),
        ]));

        let mut flags: Vec<String> = Vec::new();
        if t.persistent {
            flags.push("persistent".into());
        }
        if t.retained {
            flags.push("retained".into());
        }
        if flags.is_empty() {
            flags.push("read".into());
        }
        lines.push(Line::from(vec![
            dim("Type:  "),
            plain(t.data_type.as_str()),
            dim(format!("      Flags: {}", flags.join(","))),
        ]));

        let hz = t
            .hz()
            .map(|h| format!("{:.1} Hz", h))
            .unwrap_or_else(|| "--".into());
        let age = t.age_secs().map(fmt_delta).unwrap_or_else(|| "--".into());
        lines.push(Line::from(vec![
            dim("Rate:  "),
            muted(hz),
            dim(format!("       \u{394} {}", age)),
        ]));

        let val = t
            .current
            .as_ref()
            .map(|v| v.format())
            .unwrap_or_else(|| "-".into());
        lines.push(Line::from(vec![dim("Value: "), plain(ellipsize_left(&val, w.saturating_sub(8)))]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Left-ellipsize: keep the tail (most specific path segments) visible.
fn ellipsize_left(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width || width < 2 {
        return s.to_string();
    }
    format!("…{}", s.chars().skip(n - width + 1).collect::<String>())
}

// ---------------------------------------------------------------------------
// watchlist canvas (65% W, 100% H)
// ---------------------------------------------------------------------------

/// Shorten a topic path to its last two segments, e.g.
/// `SmartDashboard/Arm/Angle` -> `Arm/Angle`, ellipsized on the left to fit.
fn short_name(path: &str, width: usize) -> String {
    let segs: Vec<&str> = path.split('/').collect();
    let mut s = if segs.len() >= 2 {
        format!("{}/{}", segs[segs.len() - 2], segs[segs.len() - 1])
    } else {
        path.to_string()
    };
    let n = s.chars().count();
    if n > width && width > 1 {
        s = format!("…{}", s.chars().skip(n - width + 1).collect::<String>());
    }
    s
}

/// Type-aware value spans for a card's primary row. Strings get strict
/// single-line ellipsis truncation; arrays wrap onto a second sub-row.
fn value_lines(v: &crate::nt::store::NtValue, width: usize) -> Vec<Line<'static>> {
    use crate::nt::store::NtValue as V;
    let w = width.saturating_sub(2).max(2);
    match v {
        V::Boolean(b) => {
            let s = if *b { "[ TRUE ]" } else { "[ FALSE ]" };
            let style = if *b {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(0xB8, 0x4A, 0x3A))
            };
            vec![Line::from(Span::styled(s, style))]
        }
        V::Double(_) | V::Int(_) => vec![Line::from(Span::styled(
            ellipsize(&v.format(), w),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ))],
        V::Str(_) => {
            let s = format!("\"{}\"", v.format());
            vec![Line::from(Span::styled(
                ellipsize(&s, w),
                Style::default().fg(AMBER),
            ))]
        }
        V::Json(_) => vec![Line::from(Span::styled(
            ellipsize(&v.format(), w),
            Style::default().fg(AMBER),
        ))],
        // Decoded struct:Pose2d: string-like single-line card, warm amber
        // like strings (format already carries the units/degree sign).
        V::Pose2d { .. } => vec![Line::from(Span::styled(
            ellipsize(&v.format(), w),
            Style::default().fg(AMBER),
        ))],
        arr @ (V::BooleanArray(_) | V::DoubleArray(_) | V::IntArray(_) | V::StringArray(_)) => {
            let elems = array_elements(arr);
            // Cyan bracket grouping, neutral values; wrap dense arrays across
            // two fixed sub-rows with an ellipsis cap.
            let mut line1 = String::from("[");
            let mut rest: Vec<String> = Vec::new();
            for (i, e) in elems.iter().enumerate() {
                let piece = if i == 0 {
                    e.clone()
                } else {
                    format!(", {}", e)
                };
                if line1.chars().count() + piece.chars().count() + 1 <= w {
                    line1.push_str(&piece);
                } else {
                    rest.push(e.clone());
                }
            }
            if rest.is_empty() {
                line1.push(']');
                vec![Line::from(array_line(&line1))]
            } else {
                // Second sub-row: remaining elements, ellipsized.
                let mut line2 = rest.join(", ");
                let budget = w.saturating_sub(1);
                if line2.chars().count() > budget {
                    line2 = ellipsize(&line2, budget);
                }
                line1.push(',');
                vec![
                    Line::from(array_line(&line1)),
                    Line::from(array_line(&line2)),
                ]
            }
        }
        V::Raw(_) => vec![Line::from(Span::styled(
            ellipsize(&v.format(), w),
            Style::default().fg(MUTED),
        ))],
    }
}

fn array_line(s: &str) -> Vec<Span<'static>> {
    // "[" and "]" (and commas) cyan; the body neutral.
    let mut spans = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut buf = String::new();
    let mut in_value = false;
    for c in chars {
        match c {
            '[' | ']' => {
                if !buf.is_empty() {
                    spans.push(plain(std::mem::take(&mut buf)));
                }
                spans.push(Span::styled(c.to_string(), Style::default().fg(CYAN)));
                in_value = false;
            }
            ',' => {
                buf.push(',');
                spans.push(plain(std::mem::take(&mut buf)));
                spans.push(plain(" "));
                in_value = false;
            }
            ' ' if !in_value => {}
            _ => {
                buf.push(c);
                in_value = true;
            }
        }
    }
    if !buf.is_empty() {
        spans.push(plain(buf));
    }
    spans
}

fn ellipsize(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        s.to_string()
    } else if width > 1 {
        format!("{}…", s.chars().take(width - 1).collect::<String>())
    } else {
        "…".into()
    }
}

fn array_elements(v: &crate::nt::store::NtValue) -> Vec<String> {
    use crate::nt::store::NtValue as V;
    match v {
        V::BooleanArray(a) => a.iter().map(|b| b.to_string()).collect(),
        V::DoubleArray(a) => a.iter().map(|f| format!("{:.3}", f)).collect(),
        V::IntArray(a) => a.iter().map(|i| i.to_string()).collect(),
        V::StringArray(a) => a.iter().map(|s| s.clone()).collect(),
        _ => Vec::new(),
    }
}

/// Inner canvas rows reserved for a field card (borders + meta add 3).
const FIELD_CARD_ROWS: usize = 14;

/// Is this topic rendered as a pose field card? True ONLY when the
/// conservative auto-classifier accepts the current value, or the user
/// explicitly opted the topic in via `Field: Toggle Pose View on Active
/// Card` — lookalike topics (target poses, arbitrary double[6]) stay
/// normal value cards.
fn card_pose(app: &App, topic: &str) -> Option<crate::pose::PoseReading> {
    let td = app.store.topics.get(topic)?;
    let v = td.current.as_ref()?;
    let forced = app
        .config
        .field
        .force_pose_topics
        .iter()
        .any(|t| t == topic);
    let reading = {
        let auto = crate::pose::classify(topic, td.type_str.as_deref(), v);
        // Forced only widens for topics the user explicitly opted in.
        if forced {
            auto.or_else(|| crate::field::forced_reading(v))
        } else {
            auto
        }
    };
    reading
}

/// Total terminal lines a card occupies (borders + value line(s) + meta).
fn card_total_height(app: &App, topic: &str, width: usize) -> u16 {
    if card_pose(app, topic).is_some() {
        // 2 borders + canvas rows + meta row.
        return (FIELD_CARD_ROWS + 3) as u16;
    }
    let vlines = match app.store.topics.get(topic).and_then(|t| t.current.as_ref()) {
        Some(v) => {
            use crate::nt::store::NtValue as V;
            match v {
                V::BooleanArray(_) | V::DoubleArray(_) | V::IntArray(_) | V::StringArray(_) => {
                    value_lines(v, width.saturating_sub(2)).len()
                }
                _ => 1,
            }
        }
        None => 1,
    };
    (3 + vlines) as u16 // 2 borders + value line(s) + meta row
}

/// Height-first column packing, shared with the app layer for navigation:
/// column 1 is filled to 100% of the available height (using each card's
/// real rendered height) before column 2 is instantiated, up to `max_cols`
/// columns. Returns (start_index, count) per column. Recomputed on resize —
/// the caller passes the current inner size.
pub fn watch_columns(app: &App, avail_h: u16, avail_w: u16) -> Vec<(usize, usize)> {
    let cells = app.watchlist_cells();
    let n = cells.len();
    if n == 0 || avail_h == 0 || avail_w == 0 {
        return Vec::new();
    }
    let max_cols = App::watch_cols(n, avail_w as usize);
    let nominal_w = (avail_w as usize / max_cols).max(10);
    let mut cols: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        let mut count = 0usize;
        let mut used = 0u16;
        while i + count < n {
            let h = card_total_height(app, &cells[i + count], nominal_w);
            if used + h > avail_h && count > 0 {
                break;
            }
            used += h;
            count += 1;
        }
        if count == 0 {
            count = 1; // single card taller than the pane: keep progress
        }
        cols.push((i, count));
        i += count;
        if cols.len() >= max_cols {
            if i < n {
                // Width cap reached: navigation-wise the tail lives in the
                // last column (the renderer scrolls columns into view).
                let last = cols.last_mut().unwrap();
                last.1 += n - i;
            }
            break;
        }
    }
    cols
}

fn draw_watchlist(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Watchlist;
    let border = if focused {
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER_GREY)
    };
    let cells = app.watchlist_cells();
    let title_style = if focused {
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(
            format!(" WATCHLIST ({}) ", cells.len()),
            title_style,
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if cells.is_empty() {
        let lines = vec![
            Line::from(vec![
                bold("watchlist empty"),
                dim("  — highlight a topic in the tree and press "),
                plain("Space"),
                dim(" to pin it"),
            ]),
            Line::from(dim("directories pin their whole subtree; presets load with 1-9")),
        ];
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // Height-first packing: fill column 1 top-to-bottom before column 2.
    let cols_layout = watch_columns(app, inner.height, inner.width);
    let max_vis = 3.min(cols_layout.len()).max(1);

    // Keep the cursor's column visible (renderer-owned column scroll).
    let cur_col = cols_layout
        .iter()
        .position(|(s, c)| app.watchlist_cursor >= *s && app.watchlist_cursor < s + c)
        .unwrap_or(0);
    if cur_col < app.watchlist_scroll {
        app.watchlist_scroll = cur_col;
    } else if cur_col >= app.watchlist_scroll + max_vis {
        app.watchlist_scroll = cur_col + 1 - max_vis;
    }
    app.watchlist_scroll = app
        .watchlist_scroll
        .min(cols_layout.len().saturating_sub(max_vis));

    let visible: Vec<(usize, usize)> = cols_layout[app.watchlist_scroll..]
        .iter()
        .take(max_vis)
        .copied()
        .collect();
    let ncols = visible.len().max(1);
    let col_w = ((inner.width as usize) / ncols).max(10);

    for (ci, (start, count)) in visible.iter().enumerate() {
        let x = inner.x + (ci * col_w) as u16;
        let w = if ci == ncols - 1 {
            inner.width.saturating_sub((ci * col_w) as u16)
        } else {
            (col_w as u16).saturating_sub(1)
        };
        let mut y = inner.y;
        for j in 0..*count {
            let idx = start + j;
            let Some(topic) = cells.get(idx) else { break };
            let h = card_total_height(app, topic, w as usize);
            if y + h > inner.y + inner.height {
                break; // column full (over-wide tail): clipped until scrolled
            }
            let rect = Rect { x, y, width: w, height: h };
            render_card(f, app, &cells, idx, rect, focused);
            y += h;
        }
    }
}

fn render_card(f: &mut Frame, app: &App, cells: &[String], idx: usize, rect: Rect, focused: bool) {
    let topic = &cells[idx];
    let sel = idx == app.watchlist_cursor;
    let accent = if focused { CYAN } else { AMBER };

    let border_style = if sel {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER_GREY)
    };
    let title_style = if sel {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let title = short_name(topic, rect.width.saturating_sub(2) as usize);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(format!(" {} ", title), title_style));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let td = app.store.topics.get(topic);
    // Pose field cards: walls + robot marker + trail on a braille canvas.
    if let Some(reading) = card_pose(app, topic) {
        render_field_card(f, app, topic, &reading, rect, focused, sel);
        return;
    }
    let mut lines: Vec<Line> = match td.and_then(|t| t.current.as_ref()) {
        Some(v) => value_lines(v, inner.width as usize),
        None => vec![Line::from(dim("-"))],
    };
    // Meta row: type, rate, Δ — all muted (telemetry neutrality).
    let ty = td.map(|t| t.data_type.as_str()).unwrap_or("?");
    let hz = td
        .and_then(|t| t.hz())
        .map(|h| format!("{:.1} Hz", h))
        .unwrap_or_else(|| "--".into());
    let age = td.and_then(|t| t.age_secs()).map(fmt_delta).unwrap_or_else(|| "--".into());
    lines.push(Line::from(vec![
        muted(ty),
        muted("  "),
        muted(hz),
        muted(format!("  \u{394} {}", age)),
    ]));

    f.render_widget(Paragraph::new(lines), inner);
}

/// Pose field card: braille canvas — walls (muted), trail (muted), robot
/// triangle (bright cyan, on top). Coordinates are meters in the
/// blue-origin field frame; `fx` applies the USER-SET alliance mirror so
/// a red-origin user sees the field from their own side. Stored values
/// and the trail buffer are never transformed.
fn render_field_card(
    f: &mut Frame,
    app: &App,
    topic: &str,
    reading: &crate::pose::PoseReading,
    rect: Rect,
    focused: bool,
    sel: bool,
) {
    let accent = if focused { CYAN } else { AMBER };
    let border_style = if sel {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(BORDER_GREY)
    };
    let title_style = if sel {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    let title = short_name(topic, rect.width.saturating_sub(2) as usize);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(format!(" {} ", title), title_style));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Bottom row = same muted meta as value cards; canvas fills the rest.
    let (canvas_area, meta_area) = {
        let meta_h = 1u16.min(inner.height);
        let canvas_h = inner.height.saturating_sub(meta_h);
        (
            Rect { height: canvas_h, ..inner },
            Rect { y: inner.y + canvas_h, height: meta_h, ..inner },
        )
    };

    let length = app.field_map.length_m;
    let red = app.config.field.alliance == "red";
    let fx = |x: f64| if red { length - x } else { x };

    let trail: Vec<(f64, f64)> = if app.show_pose_trail {
        app.store
            .pose_trail(topic)
            .map(|t| t.iter().map(|(x, y, _)| (fx(*x), *y)).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let robot = (fx(reading.x), reading.y);
    // Alliance color: FMSInfo/IsRedAlliance when present, neutral cyan
    // otherwise (bench testing — no FMS topic on the practice field).
    let robot_color = match app.fms_red {
        Some(true) => ROBOT_RED,
        Some(false) => ROBOT_BLUE,
        None => CYAN,
    };
    let (hdx, hdy) = reading.radians.sin_cos();
    // Triangle marker: ~0.6 m tip, ~0.5 m base — legible at card scale.
    let tip = (robot.0 + 0.6 * hdx, robot.1 + 0.6 * hdy);
    let base_l = (robot.0 - 0.25 * hdx - 0.3 * hdy, robot.1 - 0.25 * hdy + 0.3 * hdx);
    let base_r = (robot.0 - 0.25 * hdx + 0.3 * hdy, robot.1 - 0.25 * hdy - 0.3 * hdx);

    let ((bx0, bx1), (by0, by1)) = crate::field::fit_bounds(
        canvas_area.width as usize,
        canvas_area.height as usize,
        length,
        app.field_map.width_m,
    );
    let canvas = Canvas::default()
        .x_bounds([bx0, bx1])
        .y_bounds([by0, by1])
        .marker(Marker::Braille)
        .paint(|ctx| {
            // Game-line marks sit behind the walls, dimmer still.
            for mark in &app.field_map.marks {
                for seg in mark.windows(2) {
                    ctx.draw(&CanvasLine {
                        x1: fx(seg[0].0),
                        y1: seg[0].1,
                        x2: fx(seg[1].0),
                        y2: seg[1].1,
                        color: MARK_GREY,
                    });
                }
            }
            // Walls: perimeter bright, obstacles tinted by alliance half.
            for wall in &app.field_map.walls {
                let color = {
                    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for (x, y) in wall {
                        b.0 = b.0.min(*x);
                        b.1 = b.1.min(*y);
                        b.2 = b.2.max(*x);
                        b.3 = b.3.max(*y);
                    }
                    match crate::field::wall_kind(b, length, app.field_map.width_m) {
                        crate::field::WallKind::Perimeter => FIELD_PERIMETER,
                        crate::field::WallKind::BlueHalf => FIELD_BLUE,
                        crate::field::WallKind::RedHalf => FIELD_RED,
                    }
                };
                for seg in wall.windows(2) {
                    ctx.draw(&CanvasLine {
                        x1: fx(seg[0].0),
                        y1: seg[0].1,
                        x2: fx(seg[1].0),
                        y2: seg[1].1,
                        color,
                    });
                }
            }
            // Alliance side labels — anchored per half, so the USER-SET x
            // mirror moves them with their side (never auto-inferred).
            let label_y = app.field_map.width_m / 2.0;
            ctx.print(fx(0.6), label_y, Span::styled("BLUE", Style::default().fg(ROBOT_BLUE)));
            ctx.print(
                fx(length - 3.4),
                label_y,
                Span::styled("RED", Style::default().fg(ROBOT_RED)),
            );
            ctx.layer(); // robot layer paints over the walls
            if !trail.is_empty() {
                ctx.draw(&Points { coords: &trail, color: MUTED });
            }
            for (a, b) in [(tip, base_l), (base_l, base_r), (base_r, tip)] {
                ctx.draw(&CanvasLine { x1: a.0, y1: a.1, x2: b.0, y2: b.1, color: robot_color });
            }
            ctx.draw(&Points { coords: &[robot], color: robot_color });
        });
    f.render_widget(canvas, canvas_area);

    // Meta row: type, rate, Δ — all muted (telemetry neutrality).
    let td = app.store.topics.get(topic);
    let ty = td.map(|t| t.data_type.as_str()).unwrap_or("?");
    let hz = td
        .and_then(|t| t.hz())
        .map(|h| format!("{:.1} Hz", h))
        .unwrap_or_else(|| "--".into());
    let age = td
        .and_then(|t| t.age_secs())
        .map(fmt_delta)
        .unwrap_or_else(|| "--".into());
    let meta = Line::from(vec![
        muted(ty),
        muted("  "),
        muted(hz),
        muted(format!("  \u{394} {}", age)),
    ]);
    f.render_widget(Paragraph::new(meta), meta_area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    // Hints list only keys that work in the current state — the empty
    // canvas drops its dead card keys (h/j/k/l, x, e, spc) instead of
    // advertising them.
    let hints = match app.focus {
        Focus::Tree => {
            "j/k move  h fold  spc pin  / find  e edit  tab watchlist  g/G ends  1-9 presets  : commands  c connect  q quit"
        }
        Focus::Watchlist if app.watchlist.is_empty() => {
            "tab tree  1-9 presets  / find  : commands  c connect  q quit"
        }
        Focus::Watchlist => {
            "h/j/k/l move  x remove  e edit  spc unpin  tab tree  g/G ends  1-9 presets  : commands  q quit"
        }
    };
    f.render_widget(Paragraph::new(Line::from(vec![dim(hints)])), area);
}

// ---------------------------------------------------------------------------
// overlays
// ---------------------------------------------------------------------------

fn draw_search(f: &mut Frame, app: &App) {
    let area = centered_rect(f.area(), 60, 14);
    f.render_widget(Clear, area);

    // Glob-aware pinned count: is_pinned honors subtree pins, so the old
    // Topic-only count under-counted — the title said "0 pinned" while
    // the rows showed amber stars.
    let pinned_n = app
        .store
        .sorted_names()
        .iter()
        .filter(|n| app.is_pinned(n))
        .count();
    let title = if pinned_n == 0 {
        " search — space/tab pin · enter jump · esc close ".to_string()
    } else {
        format!(" search — {} pinned · space/tab pin · enter jump · esc close ", pinned_n)
    };
    let mut lines: Vec<Line> = vec![Line::from(vec![plain("/"), bold(app.query.clone())])];
    // Never a silent blank box (the palette shows "no matching commands";
    // search owes the same courtesy).
    if app.search_matches.is_empty() {
        lines.push(Line::from(dim("no matches")));
    }
    let visible = area.height as usize - 2;
    let start = app
        .search_cursor
        .saturating_sub(visible.saturating_sub(2))
        .min(app.search_matches.len().saturating_sub(1));
    // search_matches holds NAMES (not indices): a topic announcing between
    // keystrokes cannot shift the list under the cursor.
    for (i, name) in app
        .search_matches
        .iter()
        .skip(start)
        .take(visible.saturating_sub(1))
        .enumerate()
    {
        let row = start + i;
        let mark = if app.is_pinned(name) {
            Span::styled("* ", Style::default().fg(AMBER).add_modifier(Modifier::BOLD))
        } else {
            plain("  ")
        };
        if row == app.search_cursor {
            lines.push(Line::from(vec![rev(format!(" {} ", name))]));
        } else {
            lines.push(Line::from(vec![plain(" "), mark, plain(name.clone())]));
        }
    }
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

/// Inline edit prompt pinned to the bottom of the screen:
/// `Set limelight-front/tv: [ 1.0000_ ]` over a dim `type · current:` row.
fn draw_edit(f: &mut Frame, app: &App) {
    let root = f.area();
    let w = root.width.min(90);
    // 5 rows: 2 borders + input line + type/current line + error line. The
    // parse error gets its OWN row so a long topic path can never push it
    // off-screen (the error is the point of the screen); showing the type
    // and current value lets the operator compose the new value without
    // memorizing either.
    let rect = Rect {
        x: 0,
        y: root.height.saturating_sub(5),
        width: w,
        height: 5,
    };
    f.render_widget(Clear, rect);

    let topic = app.edit_topic.clone().unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        bold("Set "),
        plain(topic.clone()),
        plain(":  [ "),
        bold(app.edit_input.clone()),
        plain("_"),
        plain(" ]"),
        dim("   enter=publish  esc=cancel"),
    ])];
    if let Some(t) = app.store.topics.get(&topic) {
        let cur = t
            .current
            .as_ref()
            .map(|v| v.format())
            .unwrap_or_else(|| "-".into());
        lines.push(Line::from(dim(format!(
            "{} · current: {}",
            t.data_type.as_str(),
            cur
        ))));
    }
    if let Some(e) = &app.edit_error {
        lines.push(Line::from(err(e.clone())));
    }
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " set value ",
        Style::default().fg(AMBER),
    ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    f.render_widget(Paragraph::new(lines), inner);
}

/// Connection Picker: select and connect only. Saved targets (most recently
/// used first) plus a free-text input; selection is arrow-based because
/// digits are ordinary input (addresses start with digits). Zero
/// management options — that belongs to the Settings commands.
fn draw_connect(f: &mut Frame, app: &App) {
    let targets = app.config.picker_targets();
    let h = (targets.len() as u16 + 6)
        .min(f.area().height.saturating_sub(4))
        .max(6);
    let area = centered_rect(f.area(), 60, h);
    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = vec![Line::from(vec![
        dim("connect to: "),
        bold(app.connect_input.clone()),
        plain("_"),
    ])];
    for (i, t) in targets.iter().enumerate() {
        // No [n] index prefixes: digits are ordinary input (addresses
        // start with digits), so [n] would advertise a shortcut that
        // must not exist.
        let marker = if i == app.connect_cursor && app.connect_input.is_empty() {
            bold("> ")
        } else {
            plain("  ")
        };
        lines.push(Line::from(vec![
            marker,
            plain(format!("{:<14} ", t.name)),
            dim(&t.ip),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(dim(
        "[Enter] Connect   [Arrows] Select   [Esc]",
    )));

    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " [CONNECT TARGET] ",
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

/// Focused single-input prompt (Add Robot Target / Save Preset).
fn draw_prompt(f: &mut Frame, app: &App) {
    let area = centered_rect(f.area(), 60, 5);
    f.render_widget(Clear, area);

    let (title, label) = match app.prompt_kind {
        crate::app::PromptKind::AddTarget => (
            " [ADD ROBOT TARGET] ",
            "IP/Team (e.g. \"Practice 10.99.86.2\" or \"118\"): ",
        ),
        crate::app::PromptKind::SavePreset => (" [SAVE PRESET] ", "Preset name: "),
    };
    let mut spans = vec![
        dim(label),
        plain("[ "),
        bold(app.prompt_input.clone()),
        plain("_"),
        plain(" ]"),
    ];
    if let Some(e) = &app.prompt_error {
        spans.push(err(format!("  {}", e)));
    }
    let lines = vec![
        Line::from(spans),
        Line::from(dim("enter=save  esc=cancel")),
    ];
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        title,
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_pick_target(f: &mut Frame, app: &App) {
    let targets = app.config.saved_targets.clone();
    let h = (targets.len() as u16 + 4)
        .min(f.area().height.saturating_sub(4))
        .max(5);
    let area = centered_rect(f.area(), 60, h);
    f.render_widget(Clear, area);

    let mut lines = vec![Line::from(dim("select a target to remove"))];
    for (i, t) in targets.iter().enumerate() {
        if i == app.picker_cursor {
            lines.push(Line::from(vec![
                bold("> "),
                plain(&t.name),
                dim(format!("  {}", t.ip)),
            ]));
        } else {
            lines.push(Line::from(vec![
                plain("  "),
                dim(&t.name),
                dim(format!("  {}", t.ip)),
            ]));
        }
    }
    lines.push(Line::from(dim("[Enter] Remove   [j/k] Select   [Esc]")));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" [REMOVE ROBOT TARGET] ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_pick_preset(f: &mut Frame, app: &App) {
    let presets = app.config.preset_list();
    let h = (presets.len() as u16 + 4)
        .min(f.area().height.saturating_sub(4))
        .max(5);
    let area = centered_rect(f.area(), 60, h);
    f.render_widget(Clear, area);

    let mut lines = Vec::new();
    for (i, (name, topics)) in presets.iter().enumerate() {
        let marker = if i == app.picker_cursor {
            bold("> ")
        } else {
            plain("  ")
        };
        lines.push(Line::from(vec![
            marker,
            plain(format!("[{}] ", i + 1)),
            plain(format!("{:<14} ", name)),
            dim(format!("{} topic(s)", topics.len())),
        ]));
    }
    lines.push(Line::from(dim("[Enter] Load   [1-9] Quick Load   [Esc]")));
    let block = Block::default().borders(Borders::ALL).title(" [LOAD PRESET] ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

/// Read-only configuration summary (Settings: View Settings).
fn draw_settings_view(f: &mut Frame, app: &App) {
    let area = centered_rect(f.area(), 70, 16);
    f.render_widget(Clear, area);

    let cfg = &app.config;
    let mut lines = vec![
        Line::from(vec![
            dim("config: "),
            plain(crate::config::Config::path().display().to_string()),
        ]),
        Line::from(""),
        Line::from(vec![
            bold(format!("targets ({})", cfg.saved_targets.len())),
            dim(match &cfg.last_target {
                Some(t) => format!("  last: {}", t),
                None => String::new(),
            }),
        ]),
    ];
    for t in &cfg.saved_targets {
        lines.push(Line::from(vec![
            plain("  "),
            plain(&t.name),
            dim(format!("  {}", t.ip)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        bold(format!("presets ({})", cfg.presets.len())),
        dim(": "),
        dim(cfg.presets.keys().cloned().collect::<Vec<_>>().join(", ")),
    ]));
    lines.push(Line::from(vec![
        bold("system"),
        dim(format!(
            "  ssh_user={}  restart_cmd={}",
            cfg.system.ssh_user, cfg.system.restart_cmd
        )),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(dim(
        "edit with 'Settings: Open Configuration' in the palette  [Esc] close",
    )));

    let block = Block::default().borders(Borders::ALL).title(" SETTINGS ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_palette(f: &mut Frame, app: &App) {
    let matches = &app.palette_matches;
    let h = (matches.len() as u16 + 3).min(12).max(4);
    let area = centered_rect(f.area(), 55, h);
    f.render_widget(Clear, area);

    let mut lines: Vec<Line> = vec![Line::from(vec![
        plain("> "),
        bold(app.palette_query.clone()),
    ])];
    let visible = area.height as usize - 3;
    let start = app
        .palette_cursor
        .saturating_sub(visible.saturating_sub(1))
        .min(matches.len().saturating_sub(1));
    for (i, &ci) in matches.iter().skip(start).take(visible).enumerate() {
        let name = crate::app::COMMANDS[ci].1;
        let row = start + i;
        if row == app.palette_cursor {
            lines.push(Line::from(vec![rev(format!(" > {} ", name))]));
        } else {
            lines.push(Line::from(vec![plain("   "), plain(name)]));
        }
    }
    if matches.is_empty() {
        lines.push(Line::from(dim("   no matching commands")));
    }
    let block = Block::default().borders(Borders::ALL).title(" commands ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------------
// toasts (bottom-right, non-blocking)
// ---------------------------------------------------------------------------

fn toast_kind_tag(kind: crate::app::ToastKind) -> &'static str {
    match kind {
        crate::app::ToastKind::Info => "INFO",
        crate::app::ToastKind::Success => "SUCCESS",
        crate::app::ToastKind::Warn => "WARN",
        crate::app::ToastKind::Error => "ERROR",
    }
}

fn toast_color(kind: crate::app::ToastKind) -> Color {
    match kind {
        crate::app::ToastKind::Info => MUTED,
        crate::app::ToastKind::Success => Color::Green,
        crate::app::ToastKind::Warn => Color::Yellow,
        crate::app::ToastKind::Error => Color::Red,
    }
}

fn draw_toasts(f: &mut Frame, app: &App) {
    let root = f.area();
    // The inline edit prompt is bottom-anchored and rendered BEFORE the
    // toasts; during Edit shift the stack up so a toast's Clear can never
    // erase the prompt (or its parse error) — they share the same rows.
    let shift = if app.mode == Mode::Edit { 5u16 } else { 0u16 };
    let mut bottom = root.height.saturating_sub(1 + shift); // just above the status bar
    for toast in app.toasts.iter().rev().take(3) {
        let text = format!("[{}] {}", toast_kind_tag(toast.kind), toast.msg);
        // Clamp to the terminal width (never skip rendering on narrow
        // terms) and ellipsize to the inner width so long failure reasons
        // terminate visibly instead of hard-clipping mid-word.
        let w = ((text.chars().count() as u16) + 4).clamp(12, 60).min(root.width);
        let h = 3u16;
        if bottom < h {
            break;
        }
        let text = ellipsize(&text, (w as usize).saturating_sub(2));
        let rect = Rect {
            x: root.width.saturating_sub(w + 1),
            y: bottom - h,
            width: w,
            height: h,
        };
        f.render_widget(Clear, rect);
        let color = toast_color(toast.kind);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(color));
        f.render_widget(block, rect);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: w - 2,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(text, Style::default().fg(color)))),
            inner,
        );
        bottom = bottom.saturating_sub(h);
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
