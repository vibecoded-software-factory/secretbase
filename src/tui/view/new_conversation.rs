//! New-conversation popup (`n` from the inbox).
//!
//! Centered modal that asks for a comma-separated user list. Enter
//! confirms (`keybase chat api newconv`), Esc cancels.

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
    let area = center_rect(60, 7, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(Span::styled(
            " New conversation ",
            Style::default().fg(t.accent),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from({
            let mut spans = vec![Span::styled("  participants: ", Style::default().fg(t.dim))];
            spans.extend(editor_spans(&app.new_conv, true, t));
            spans
        }),
        Line::from(""),
        crate::tui::view::widgets::legend_line(
            &[
                ("Enter", "create"),
                ("Esc", "cancel"),
                ("", "(comma-separated usernames)"),
            ],
            area.width.saturating_sub(2) as usize,
            t,
        )
        .alignment(Alignment::Center),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(rounded_block(Style::default().fg(t.accent))),
        area,
    );
}
