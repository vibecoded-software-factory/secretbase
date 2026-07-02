//! Login-hint screen — Keybase has no in-TUI password login.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{center_rect_abs, rounded_block, wrapped_line_count};
use crate::tui::view::{action, logo};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let t = &app.theme;

    let layout = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);
    let body = layout[0];
    let bar = layout[1];

    logo::render(frame, app, body);

    // Content-sized, responsive box: the message wraps instead of clipping, and
    // the box grows/shrinks with it. A fixed `width_pct` clipped the hint on a
    // narrow terminal — here we size to the text and only shrink when we must.
    let title = "Not logged in to Keybase";
    let msg = "Run  keybase login  in a terminal, then press R.";
    let natural = title.chars().count().max(msg.chars().count()) as u16;
    // Leave a margin so the box never sits edge-to-edge; clamp to the terminal.
    let inner_w = natural.min(body.width.saturating_sub(6)).max(12);
    let msg_lines = wrapped_line_count(msg, inner_w);
    // width = content + 2 border + 2 side padding; height = borders (2) +
    // title (1) + blank (1) + wrapped message lines.
    let panel = center_rect_abs(inner_w + 4, msg_lines + 4, body);

    let lines = vec![
        Line::from(Span::styled(title, Style::default().fg(t.error))).alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(msg, Style::default().fg(t.foreground)))
            .alignment(Alignment::Center),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(rounded_block(Style::default().fg(t.error))),
        panel,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " R: retry   ·   Q/Esc: quit   ·   F1: help",
            Style::default().fg(t.dim),
        ))),
        bar,
    );

    action::render(frame, app, body);
}
