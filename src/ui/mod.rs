//! Layout + rendering. Monochrome by default (bold/dim/reverse); color is
//! reserved for state that demands attention: green = healthy/editable,
//! yellow = retry/warning, red = down/error/stale.

pub mod plot;
pub mod sparkline;
pub mod tree;

use crate::app::{App, DiffRow, Focus, Mode, StatusKind, View};
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
    match app.view {
        View::Tree => draw_main(f, app, rows[1]),
        View::Matrix => draw_matrix(f, app, rows[1]),
        View::Zoom => draw_zoom(f, app, rows[1]),
    }
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

fn pad_cell(spans: &mut Vec<Span>, text: &str, width: usize) {
    let n = text.chars().count();
    let mut t = text.to_string();
    if n > width {
        t = text.chars().take(width).collect();
    }
    spans.push(plain(t));
    if n < width {
        spans.push(plain(" ".repeat(width - n)));
    }
}

/// Auto-packed telemetry matrix: every cell shows name, bold value, Hz and
/// an inline sparkline. No canvas, no dragging — cells pack themselves.
fn draw_matrix(f: &mut Frame, app: &App, area: Rect) {
    let cells = app.matrix_cells();
    if cells.is_empty() {
        let mut lines = vec![Line::from(vec![
            bold("matrix empty"),
            dim("  — stage topics with / + space, press W on a directory, or load a preset with 1-9"),
        ])];
        for (i, p) in app.presets.iter().enumerate() {
            lines.push(Line::from(vec![
                dim(format!("  [{}] ", i + 1)),
                plain(&p.name),
                dim(format!("  ({} item(s))", p.topics.len())),
            ]));
        }
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let cols = app.grid_cols.max(1);
    let card_w = ((area.width as usize) / cols).max(12);
    let card_h = 3usize; // name / value+hz / sparkline
    let grid_rows = ((area.height as usize) / card_h).max(1);

    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    'rows: for r in 0..grid_rows {
        let mut name_l: Vec<Span> = Vec::new();
        let mut val_l: Vec<Span> = Vec::new();
        let mut spark_l: Vec<Span> = Vec::new();
        for c in 0..cols {
            let idx = r * cols + c;
            if idx >= cells.len() {
                name_l.push(plain(" ".repeat(card_w)));
                val_l.push(plain(" ".repeat(card_w)));
                spark_l.push(plain(" ".repeat(card_w)));
                continue;
            }
            let topic = &cells[idx];
            let td = app.store.topics.get(topic);
            let val = td
                .and_then(|t| t.current.as_ref())
                .map(|v| v.format())
                .unwrap_or_else(|| "-".into());
            let hz = td
                .and_then(|t| t.hz())
                .map(|h| format!("{:.0}Hz", h))
                .unwrap_or_else(|| "-".into());
            let hist = td.map(|t| &t.history);
            let sel = idx == app.matrix_cursor;

            let name = short_name(topic, card_w.saturating_sub(2));
            let name = format!(" {} ", name);
            if sel {
                pad_cell(&mut name_l, &name, card_w);
                // re-color the just-padded spans reversed
                let start = name_l.len() - 1; // last two spans belong to this cell
                for s in &mut name_l[start..] {
                    s.style = Style::default().add_modifier(Modifier::REVERSED);
                }
            } else {
                name_l.push(dim(name));
            }

            let hz_w = hz.len().min(card_w.saturating_sub(6));
            let val_w = card_w.saturating_sub(hz_w + 2);
            let mut v = val;
            if v.chars().count() > val_w {
                v = v.chars().take(val_w).collect();
            }
            let vpad = card_w.saturating_sub(v.chars().count() + hz_w + 1);
            if sel {
                val_l.push(rev(v));
                val_l.push(plain(" ".repeat(vpad)));
                val_l.push(rev(hz));
            } else {
                val_l.push(bold(v));
                val_l.push(plain(" ".repeat(vpad)));
                val_l.push(dim(hz));
            }
            val_l.push(plain(" ")); // gutter between cards

            let spark = hist
                .map(|h| sparkline::render(h, card_w.saturating_sub(2)))
                .unwrap_or_default();
            let spark = format!(" {}", spark);
            if sel {
                pad_cell(&mut spark_l, &spark, card_w);
                let start = spark_l.len() - 1;
                for s in &mut spark_l[start..] {
                    s.style = Style::default().add_modifier(Modifier::REVERSED);
                }
            } else {
                spark_l.push(dim(spark));
            }
        }
        lines.push(Line::from(name_l));
        lines.push(Line::from(val_l));
        lines.push(Line::from(spark_l));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Zoom: one topic exploded into a full-width multi-row ASCII plot.
fn draw_zoom(f: &mut Frame, app: &App, area: Rect) {
    let Some(topic) = &app.zoom_topic else {
        f.render_widget(Paragraph::new(dim("no topic")), area);
        return;
    };
    let td = app.store.topics.get(topic);
    let val = td
        .and_then(|t| t.current.as_ref())
        .map(|v| v.format())
        .unwrap_or_else(|| "-".into());
    let hz = td
        .and_then(|t| t.hz())
        .map(|h| format!("{:.2} Hz", h))
        .unwrap_or_else(|| "-".into());

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![bold(topic)]));
    lines.push(Line::from(vec![
        Span::styled(
            val.clone(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        dim(format!("   {}   last update ", hz)),
        plain(
            td.and_then(|t| t.age_secs())
                .map(|a| format!("{:.2}s", a))
                .unwrap_or_else(|| "-".into()),
        ),
    ]));
    let used = lines.len() + 2;
    let plot_h = (area.height as usize).saturating_sub(used).max(1);
    let w = area.width as usize;
    if let Some(t) = td {
        for row in plot::render(&t.history, w, plot_h) {
            lines.push(Line::from(vec![ok(row)]));
        }
    }
    // min/max footer
    if let Some(t) = td {
        let (min, max) = t
            .history
            .iter()
            .cloned()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                (a.min(v), b.max(v))
            });
        if min.is_finite() {
            lines.push(Line::from(dim(format!(
                "min {:.4}  max {:.4}  ({} samples)",
                min,
                max,
                t.history.len()
            ))));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
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
    let hints = match app.view {
        View::Zoom => "z close  v grid  q quit",
        View::Matrix => "h/j/k/l move  z zoom  e edit  v tree  1-9 presets  q quit",
        View::Tree => match app.focus {
            Focus::Tree => "j/k move  h fold  e edit  / find  tab  s snap  d diff  W all  v grid  R recon  c conn  q quit",
            Focus::Inspector => "j/k scroll  tab pane  q quit",
        },
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
    let title = if app.staged.is_empty() {
        "search".to_string()
    } else {
        format!("search ({} staged — enter adds to matrix)", app.staged.len())
    };
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
        let mark = if app.staged.iter().any(|t| t == name) {
            ok("[x] ")
        } else {
            dim("[ ] ")
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
