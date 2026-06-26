//! Login-hint screen — Keybase has no in-TUI password login.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::app::App;
use crate::tui::view::widgets::{center_rect, rounded_block};
use crate::tui::view::{action, logo};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let t = &app.theme;

    let layout = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);
    let body = layout[0];
    let bar = layout[1];

    logo::render(frame, app, body);

    let panel = center_rect(60, 5, body);
    let lines = vec![
        Line::from(Span::styled(
            " Not logged in to Keybase ",
            Style::default().fg(t.error),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(
            "Run  keybase login  in a terminal, then press R.",
            Style::default().fg(t.foreground),
        ))
        .alignment(Alignment::Center),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(rounded_block(Style::default().fg(t.error))),
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
