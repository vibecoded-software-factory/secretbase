//! Unhide popup (Shift+H from the inbox).
//!
//! Restores a blocked / reported conversation by name — they're gone from the
//! inbox `list`, so they can't be selected. Enter confirms
//! (`keybase chat api setstatus … unfiled`), Esc cancels.

use ratatui::{
    Frame,
    layout::Alignment,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{center_rect, editor_spans, rounded_block};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(60, 8, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(Span::styled(
            " Unhide conversation ",
            Style::default().fg(t.accent),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "restore a blocked / reported chat by name",
            Style::default().fg(t.dim),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from({
            let mut spans = vec![Span::styled("  username: ", Style::default().fg(t.dim))];
            spans.extend(editor_spans(&app.unhide_input, true, t));
            spans
        }),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: restore   |   Esc: cancel   |   (the other user's name)",
            Style::default().fg(t.dim),
        ))
        .alignment(Alignment::Center),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(rounded_block(Style::default().fg(t.accent))),
        area,
    );
}
