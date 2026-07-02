//! Members view (`m` in the channel browser, `Alt+P` on an open team channel).
//!
//! Lists a conversation's members (`keybase chat api listmembers`) grouped by
//! role. `a` adds member(s), `x`/`d` removes the selected one, `Esc` closes.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, ListState, Paragraph},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{
    MODAL_HEIGHT, MODAL_WIDTH_PCT, center_rect, editor_spans, rounded_block,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, frame.area());
    frame.render_widget(Clear, area);

    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    let items: Vec<ListItem> = app
        .members
        .iter()
        .map(|m| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {}", m.username),
                    Style::default().fg(t.foreground),
                ),
                Span::styled(format!("   {}", m.role.label()), Style::default().fg(t.dim)),
            ]))
        })
        .collect();

    let mut state = ListState::default();
    state.select(if app.members.is_empty() {
        None
    } else {
        Some(app.members_selected.min(app.members.len() - 1))
    });

    let title = format!(" Members — {} · {} ", app.members_label, app.members.len());
    frame.render_stateful_widget(
        List::new(items)
            .block(
                rounded_block(Style::default().fg(t.accent)).title(Span::styled(
                    title,
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                )),
            )
            .highlight_style(Style::default().bg(t.selected_bg))
            .highlight_symbol("▶ "),
        layout[0],
        &mut state,
    );

    // Bottom row: add input, remove confirm, else the hint.
    let bottom = if app.member_adding {
        let mut spans = vec![Span::styled(
            " add (comma/space): ",
            Style::default().fg(t.dim),
        )];
        spans.extend(editor_spans(&app.member_add_input, true, t));
        spans.push(Span::styled(
            "   (Enter add · Esc cancel)",
            Style::default().fg(t.dim),
        ));
        Line::from(spans)
    } else if let Some(username) = &app.member_confirm_remove {
        Line::from(vec![
            Span::styled(
                format!(" Remove {username}? "),
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled("y: remove · n/Esc: cancel", Style::default().fg(t.dim)),
        ])
    } else {
        Line::from(Span::styled(
            " a add · Shift+X remove · F5 refresh · Esc close ",
            Style::default().fg(t.dim),
        ))
    };
    frame.render_widget(Paragraph::new(bottom), layout[1]);
}
