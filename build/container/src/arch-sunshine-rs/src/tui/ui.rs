use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use tui_term::widget::PseudoTerminal;

use crate::tui::state::{AppState, Health, LogSource, Modal};

const ACCENT: Color = Color::Rgb(255, 184, 108);
const ACCENT_DIM: Color = Color::Rgb(180, 130, 75);
const FG_MUTED: Color = Color::Rgb(170, 170, 170);
const FG_DIM: Color = Color::Rgb(110, 110, 110);
const OK: Color = Color::Rgb(120, 200, 120);
const BAD: Color = Color::Rgb(230, 110, 110);
const WARN: Color = Color::Rgb(230, 200, 90);
const BG_PANEL: Color = Color::Rgb(20, 20, 24);

pub fn draw(frame: &mut Frame, app: &AppState) -> Option<ratatui::layout::Rect> {
    let outer = frame.area();
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT_DIM))
        .title(Line::from(vec![
            Span::styled(" Arch Sunshine ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        ]))
        .title_alignment(ratatui::layout::Alignment::Left)
        .title_top(stream_title(app))
        .style(Style::default().bg(BG_PANEL));
    let inner = outer_block.inner(outer);
    frame.render_widget(outer_block, outer);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

    let mut shell_area = None;
    draw_status(frame, chunks[0], app);
    if app.modal == Modal::Shell {
        shell_area = Some(draw_shell(frame, chunks[1], app));
        draw_shell_keybar(frame, chunks[2]);
    } else {
        draw_logs(frame, chunks[1], app);
        draw_keybar(frame, chunks[2]);
    }

    if app.modal == Modal::PairPin {
        draw_pin_modal(frame, app);
    }
    shell_area
}

fn stream_title(app: &AppState) -> Line<'_> {
    let s = &app.status;
    let title = format!(
        " {}×{}@{} scale {} · logical {}×{} ",
        s.stream_width,
        s.stream_height,
        s.stream_fps,
        s.stream_scale,
        s.logical_width,
        s.logical_height,
    );
    Line::from(Span::styled(title, Style::default().fg(FG_MUTED)))
        .alignment(ratatui::layout::Alignment::Right)
}

fn health_dot(h: Health) -> Span<'static> {
    let (color, glyph) = match h {
        Health::Up => (OK, "●"),
        Health::Down => (BAD, "●"),
        Health::Unknown => (FG_DIM, "○"),
    };
    Span::styled(glyph, Style::default().fg(color))
}

fn socket_dot(present: bool) -> Span<'static> {
    if present {
        Span::styled("●", Style::default().fg(OK))
    } else {
        Span::styled("●", Style::default().fg(BAD))
    }
}

fn label(text: &str) -> Span<'_> {
    Span::styled(text, Style::default().fg(FG_MUTED).add_modifier(Modifier::BOLD))
}

fn dim(text: &str) -> Span<'_> {
    Span::styled(text, Style::default().fg(FG_DIM))
}

fn dim_owned(text: String) -> Span<'static> {
    Span::styled(text, Style::default().fg(FG_DIM))
}

fn draw_status(frame: &mut Frame, area: Rect, app: &AppState) {
    let s = &app.status;
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        label("Desktop   "),
        health_dot(s.kwin),
        Span::raw(" kwin   "),
        health_dot(s.plasmashell),
        Span::raw(" plasmashell"),
    ]));

    lines.push(Line::from(vec![
        label("Stream    "),
        health_dot(s.sunshine),
        Span::raw(" sunshine  "),
        socket_dot(s.wayland_socket),
        Span::raw(" wayland  "),
        socket_dot(s.pipewire_socket),
        Span::raw(" pipewire  "),
        socket_dot(s.pulse_socket),
        Span::raw(" pulse "),
        dim_owned(format!("· {}", s.audio_sink)),
    ]));

    lines.push(Line::from(vec![
        label("GPU       "),
        Span::raw(s.render_nodes.clone()),
    ]));

    let encoder_parts: Vec<Span> = if s.encoder_summary.is_empty() {
        vec![dim("(probing…)")]
    } else {
        let mut spans = Vec::new();
        for (i, (codec, chosen)) in s.encoder_summary.iter().enumerate() {
            if i > 0 {
                spans.push(dim(" │ "));
            }
            spans.push(Span::styled(
                codec.to_uppercase(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            match chosen {
                Some(name) => spans.push(Span::raw(name.clone())),
                None => spans.push(Span::styled("unavailable", Style::default().fg(BAD))),
            }
        }
        spans
    };
    let mut enc_line = vec![label("Encoder   ")];
    enc_line.extend(encoder_parts);
    lines.push(Line::from(enc_line));

    let para = Paragraph::new(lines).style(Style::default().fg(Color::White));
    frame.render_widget(para, area);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(20)])
        .split(area);

    let mut items: Vec<ListItem> = Vec::new();
    for src in LogSource::ALL {
        let buf = app.logs.get(&src);
        let unread = buf.map(|b| b.unread).unwrap_or(0);
        let is_focused = src == app.focused_log;
        let marker = if is_focused { "›" } else { " " };
        let mut spans: Vec<Span> = vec![
            Span::styled(
                format!("{marker} "),
                Style::default().fg(if is_focused { ACCENT } else { FG_DIM }),
            ),
            Span::styled(
                src.label(),
                if is_focused {
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ];
        if unread > 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("●{}", unread),
                Style::default().fg(WARN),
            ));
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT_DIM))
            .border_set(border::ROUNDED)
            .title(Span::styled(" Logs ", Style::default().fg(FG_MUTED))),
    );
    frame.render_widget(list, chunks[0]);

    let viewer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_DIM))
        .border_set(border::ROUNDED)
        .title(Span::styled(
            format!(" {} ", app.focused_log.label()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = viewer_block.inner(chunks[1]);
    frame.render_widget(viewer_block, chunks[1]);

    let Some(buf) = app.focused_buffer() else { return };
    let lines_ref = buf.lines();
    let width = inner.width as usize;
    let height = inner.height as usize;
    let total = lines_ref.len();
    if width == 0 || height == 0 || total == 0 {
        return;
    }

    // Anchor end of the visible window: with follow_tail we want the very
    // last line; when scrolled back, log_scroll counts how many of the most
    // recent lines to skip.
    let end = total.saturating_sub(if app.follow_tail { 0 } else { app.log_scroll });
    let end = end.min(total);

    // Walk backwards from the anchor, summing the wrapped-row cost of each
    // log line, and stop as soon as adding another line would overflow the
    // viewport. This keeps long lines from pushing later entries out of the
    // panel and into the keybar below.
    let mut accumulated = 0usize;
    let mut start = end;
    for i in (0..end).rev() {
        let line_len = lines_ref[i].chars().count().max(1);
        let rows = line_len.div_ceil(width);
        if accumulated + rows > height {
            break;
        }
        accumulated += rows;
        start = i;
    }

    let visible: Vec<Line> = lines_ref[start..end]
        .iter()
        .map(|l| Line::from(highlight_log_line(l)))
        .collect();
    let paragraph = Paragraph::new(visible).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn highlight_log_line(line: &str) -> Vec<Span<'_>> {
    let lower = line.to_ascii_lowercase();
    let color = if lower.contains("error") || lower.contains("fatal") {
        BAD
    } else if lower.contains("warn") {
        WARN
    } else if lower.contains("info") {
        Color::Rgb(200, 200, 200)
    } else {
        FG_MUTED
    };
    vec![Span::styled(line.to_string(), Style::default().fg(color))]
}

fn draw_shell(frame: &mut Frame, area: Rect, app: &AppState) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT_DIM))
        .border_set(ratatui::symbols::border::ROUNDED)
        .title(Span::styled(
            " Shell (sunshine) ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if let Some(shell) = app.shell.as_ref() {
        if let Ok(parser) = shell.parser.lock() {
            let screen = parser.screen();
            let widget = PseudoTerminal::new(screen);
            frame.render_widget(widget, inner);
        }
    }
    inner
}

fn draw_shell_keybar(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled("[F10]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Detach shell   "),
        Span::styled("exit", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" or "),
        Span::styled("Ctrl+D", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" closes the shell"),
    ]);
    let p = Paragraph::new(line).style(Style::default().fg(FG_MUTED).bg(BG_PANEL));
    frame.render_widget(p, area);
}

fn draw_keybar(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled("[r]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Pair   "),
        Span::styled("[s]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Shell   "),
        Span::styled("[Tab]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Log   "),
        Span::styled("[↑↓]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Scroll   "),
        Span::styled("[F]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Follow   "),
        Span::styled("[q]", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" Quit"),
    ]);
    let p = Paragraph::new(line).style(Style::default().fg(FG_MUTED).bg(BG_PANEL));
    frame.render_widget(p, area);
}

fn draw_pin_modal(frame: &mut Frame, app: &AppState) {
    let area = frame.area();
    let width = 48.min(area.width.saturating_sub(4));
    let height = 7;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect { x, y, width, height };

    frame.render_widget(Clear, modal_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .border_set(border::ROUNDED)
        .title(Span::styled(
            " Moonlight Pairing ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Rgb(28, 28, 32)));
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut lines = vec![
        Line::from(Span::styled(
            "Enter the 4-digit PIN shown by Moonlight:",
            Style::default().fg(FG_MUTED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<8}", app.pin_input),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ),
        ]),
    ];
    if !app.pin_message.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            app.pin_message.clone(),
            Style::default().fg(WARN),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[Enter] submit   [Esc] cancel",
        Style::default().fg(FG_DIM),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}
