//! Small input popups: reaction input + attachment download path.
//!
//! The destructive confirmations (logout, delete message) render through
//! the shared `widgets::draw_confirm_popup` from `view::mod`.

use ratatui::{
    Frame,
    layout::Alignment,
    style::Style,
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{center_rect, editor_spans, rounded_block};

pub fn download_attachment(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(70, 7, frame.area());
    frame.render_widget(Clear, area);

    let header = app
        .download_msg_id
        .map(|id| format!("Download attachment from msg #{id}"))
        .unwrap_or_else(|| "Download attachment".to_string());

    let lines = vec![
        Line::from(Span::styled(header, Style::default().fg(t.accent)))
            .alignment(Alignment::Center),
        Line::from(""),
        Line::from(editor_spans(&app.download, true, t)),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: save   |   Esc: cancel   |   ←/→ Home/End edit",
            Style::default().fg(t.dim),
        ))
        .alignment(Alignment::Center),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(rounded_block(Style::default().fg(t.accent))),
        area,
    );
}

pub fn react_input(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    // 6 content lines (title · target · blank · input · blank · hint)
    // + 2 borders — 7 rows clipped the hint, so reserve 8.
    let area = center_rect(50, 8, frame.area());
    frame.render_widget(Clear, area);

    let target = app
        .selected_msg_idx
        .and_then(|i| app.messages.get(i))
        .map(|m| format!("reacting to #{}  by {}", m.id, m.sender))
        .unwrap_or_else(|| "(no message selected)".to_string());

    let lines = vec![
        Line::from(Span::styled(
            " Reaction (e.g. :+1:) ",
            Style::default().fg(t.accent),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(target, Style::default().fg(t.dim))).alignment(Alignment::Center),
        Line::from(""),
        Line::from(editor_spans(&app.react, true, t)),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: send   |   Esc: cancel",
            Style::default().fg(t.dim),
        ))
        .alignment(Alignment::Center),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(rounded_block(Style::default().fg(t.accent))),
        area,
    );
}
