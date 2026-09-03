//! Layout + rendering. Monochrome by default (bold/dim/reverse); color is
//! reserved for state that demands attention: green = healthy/editable,
//! yellow = retry/warning, red = down/error/stale.

pub mod sparkline;
pub mod tree;

use crate::app::{App, DiffRow, Focus, Mode, StatusKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

fn bold(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().add_modifier(Modifier::BOLD))
}

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().add_modifier(Modifier::DIM))
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

fn warn(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::Yellow))
}

pub fn draw(f: &mut Frame, app: &App) {
    let root = f.area();

    // vertical split: header / (tree|inspector) / status
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(1),    // main
            Constraint::Length(1), // status bar
        ])
        .split(root);

    draw_header(f, app, rows[0]);
    draw_main(f, app, rows[1]);
    draw_status(f, app, rows[2]);

    // overlays
    if app.mode == Mode::Search {
        draw_search(f, app);
    } else if app.mode == Mode::Edit {
        draw_edit(f, app);
    } else if app.mode == Mode::Connect {
        draw_connect(f, app);
    }
}

// ---------------------------------------------------------------------------

fn fmt_elapsed(us: u64) -> String {
    let total_secs = us / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}.{}",
        total_secs / 3600,
        (total_secs / 60) % 60,
        total_secs % 60,
        (us % 1_000_000) / 100_000
    )
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let conn = if app.connected {
        Span::styled(
            "LIVE",
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        )
    } else if app.connecting && app.retry_attempt > 0 {
        // Reconnecting after a drop: say so, and how hard we're trying.
        Span::styled(
            format!("DOWN (retry {})", app.retry_attempt),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    } else if app.connecting {
        dim("connecting...")
    } else {
        Span::styled(
            "DOWN",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    };

    let topic_count = app.store.topics.len();
    let hz = if app.connected {
        format!("{:.1} Hz", app.store.total_hz())
    } else {
        "--".into()
    };
    let rtt = app
        .rtt_ms
        .map(|r| format!("{:.1}ms", r))
        .unwrap_or_else(|| "--".into());

    let line1 = Line::from(vec![
        bold("nt-tui"),
        plain(format!("  {}  ", app.target)),
        conn,
        plain(format!("  rtt {}  {} topics  {}", rtt, topic_count, hz)),
    ]);

    // Uptime: NT4 robot timestamps are us since boot (FPGA clock), so the
    // delta between the first and latest seen server timestamp is the
    // roboRIO's uptime; process uptime needs a server-side topic later.
    let uptime = match (app.first_server_ts, app.last_server_ts) {
        (Some(first), Some(last)) if last >= first => fmt_elapsed(last - first),
        _ => "--:--:--.-".into(),
    };
    let line2 = Line::from(vec![
        dim("robot uptime "),
        plain(uptime),
        dim(format!("  {}", app.server_info)),
    ]);

    let text = vec![line1, line2];
    f.render_widget(Paragraph::new(text), area);
}

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_tree(f, app, cols[0]);
    draw_inspector(f, app, cols[1]);
}

fn draw_tree(f: &mut Frame, app: &App, area: Rect) {
    let rows = tree::build_tree(&app.store, &app.expanded);

    // Manual window around the cursor (List::scroll needs stateful widgets).
    let height = area.height as usize;
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
            let line = if row.is_topic {
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
                // Green type tag = the topic is editable (e edit works);
                // dim = read-only type. A glance says what can be changed.
                let type_tag = if td.map(|t| t.data_type.is_writable()).unwrap_or(false) {
                    ok(format!(" [{}] ", ty))
                } else {
                    dim(format!(" [{}] ", ty))
                };
                Line::from(vec![
                    plain(format!("{}{}", indent, leaf)),
                    type_tag,
                    plain(val),
                    dim(format!(" {}", hz)),
                ])
            } else {
                Line::from(vec![plain(format!("{}{}", indent, row.label))])
            };
            if idx == app.tree_cursor && app.focus == Focus::Tree {
                // Visible cursor marker: survives terminals that drop
                // reverse-video, and fits the zero-color design.
                ListItem::new(Line::from(vec![
                    bold("> "),
                    plain(format!("{}{}", indent, row.label)),
                ]))
                .style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    // Focused pane's divider renders reversed: a bright bar marking which
    // side owns the keyboard.
    let focus_style = if app.focus == Focus::Tree {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::RIGHT).border_style(focus_style));
    f.render_widget(list, area);
}

fn draw_inspector(f: &mut Frame, app: &App, area: Rect) {
    let (path, is_topic) = app
        .cursor_path()
        .unwrap_or_else(|| (String::new(), false));

    let mut lines: Vec<Line> = Vec::new();

    if path.is_empty() {
        lines.push(Line::from(dim("(no topic selected)")));
    } else {
        lines.push(Line::from(vec![bold(&path)]));

        if let Some(t) = app.store.topics.get(&path) {
            if is_topic {
                lines.push(Line::from(vec![
                    dim("type     "),
                    plain(t.data_type.as_str()),
                ]));
                lines.push(Line::from(vec![
                    dim("edit     "),
                    if t.data_type.is_writable() {
                        ok("yes")
                    } else {
                        dim("no (read-only type)")
                    },
                ]));
                lines.push(Line::from(vec![
                    dim("flags    "),
                    plain(if t.persistent { "persistent " } else { "" }),
                    plain(if t.retained { "retained" } else { "" }),
                ]));

                let val = t
                    .current
                    .as_ref()
                    .map(|v| v.format())
                    .unwrap_or_else(|| "-".into());
                lines.push(Line::from(vec![dim("value    "), plain(val)]));

                let hz = t.hz().map(|h| format!("{:.2} Hz", h)).unwrap_or("-".into());
                let age = t
                    .age_secs()
                    .map(|a| format!("{:.2}s", a))
                    .unwrap_or("-".into());
                lines.push(Line::from(vec![
                    dim("rate     "),
                    plain(hz),
                    dim("  last update "),
                    plain(age),
                ]));

                if let Some(hist) = app
                    .store
                    .topics
                    .get(&path)
                    .map(|t| &t.history)
                {
                    let w = area.width.saturating_sub(2) as usize;
                    lines.push(Line::from(vec![dim("trend    "), plain(sparkline::render(hist, w))]));
                }
            } else {
                lines.push(Line::from(dim("(directory)")));
            }
        }
    }

    // diff panel below the inspector info
    if !app.diff.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(bold(format!("diff ({} rows)", app.diff.len()))));
        let visible = area.height.saturating_sub(lines.len() as u16 + 1) as usize;
        let start = app
            .diff_cursor
            .saturating_sub(visible.saturating_sub(1))
            .min(app.diff.len().saturating_sub(1));
        for (i, row) in app.diff.iter().skip(start).take(visible.max(1)).enumerate() {
            let idx = start + i;
            let (marker, marker_span) = match row {
                DiffRow::Changed(..) => ("~", warn("~")),
                DiffRow::Added(..) => ("+", ok("+")),
                DiffRow::Removed(..) => ("-", err("-")),
            };
            let text = match row {
                DiffRow::Changed(k, old, new) => format!(" {}  {} -> {}", k, old, new),
                DiffRow::Added(k, new) => format!(" {}  {}", k, new),
                DiffRow::Removed(k, old) => format!(" {}  {}", k, old),
            };
            if idx == app.diff_cursor {
                lines.push(Line::from(vec![rev(format!("> {}{}", marker, text))]));
            } else {
                lines.push(Line::from(vec![marker_span, plain(text)]));
            }
        }
    }

    let block = Block::default().borders(Borders::LEFT).border_style(
        if app.focus == Focus::Inspector {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        },
    );
    let inner_area = block.inner(area);
    f.render_widget(block, area);
    let para = Paragraph::new(lines).scroll((app.inspector_scroll as u16, 0));
    f.render_widget(para, inner_area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.focus {
        Focus::Tree => "j/k move  h fold  e edit  / find  tab pane  s snap  d diff  R recon  c connect  q quit",
        Focus::Inspector => "j/k scroll  tab pane  q quit",
    };
    // Status first: a fresh message (published ... / disconnected ...) must
    // never be truncated away by the hint list; hints may clip instead.
    let mut spans = Vec::new();
    if !app.status.is_empty() {
        spans.push(match app.status_kind {
            StatusKind::Info => dim(app.status.clone()),
            StatusKind::Warn => warn(app.status.clone()),
            StatusKind::Error => err(app.status.clone()),
        });
        spans.push(dim("  |  "));
    }
    spans.push(dim(hints));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// overlays
// ---------------------------------------------------------------------------

fn draw_search(f: &mut Frame, app: &App) {
    let area = centered_rect(f.area(), 60, 12);
    f.render_widget(ratatui::widgets::Clear, area);

    let names = app.store.sorted_names();
    let mut lines: Vec<Line> = vec![Line::from(vec![plain("/"), bold(app.query.clone())])];
    let visible = area.height as usize - 2;
    let start = app
        .search_cursor
        .saturating_sub(visible.saturating_sub(2))
        .min(app.search_matches.len().saturating_sub(1));
    for (i, &idx) in app
        .search_matches
        .iter()
        .skip(start)
        .take(visible.saturating_sub(1))
        .enumerate()
    {
        let name = &names[idx];
        let row = start + i;
        if row == app.search_cursor {
            lines.push(Line::from(vec![rev(format!(" {}", name))]));
        } else {
            lines.push(Line::from(plain(format!(" {}", name))));
        }
    }
    let block = Block::default().borders(Borders::ALL).title("search");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_edit(f: &mut Frame, app: &App) {
    let area = centered_rect(f.area(), 50, 5);
    f.render_widget(ratatui::widgets::Clear, area);

    let topic = app.edit_topic.clone().unwrap_or_default();
    let lines = vec![
        Line::from(vec![bold("set "), plain(topic)]),
        Line::from(vec![plain("> "), bold(app.edit_input.clone()), plain("_")]),
        Line::from(vec![
            dim("enter=publish  esc=cancel"),
            match app.edit_error {
                Some(ref e) => err(format!("  {}", e)),
                None => plain(""),
            },
        ]),
    ];
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_connect(f: &mut Frame, app: &App) {
    let area = centered_rect(f.area(), 60, 5);
    f.render_widget(ratatui::widgets::Clear, area);

    let lines = vec![
        Line::from(vec![
            bold("connect to: "),
            plain(app.connect_input.clone()),
            plain("_"),
        ]),
        Line::from(dim(
            "team number or host[:port]  enter=connect  esc=cancel",
        )),
    ];
    let block = Block::default().borders(Borders::ALL).title("connect");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
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
