//! Channel browser (Alt+K on a team).
//!
//! Lists **every** channel of a team (`keybase chat api listconvsonname`),
//! marking the ones you're in. `Enter` opens a joined channel or joins one you
//! aren't in; `x` leaves; `Esc` closes.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, ListState, Paragraph},
};

use crate::domain::MemberStatus;
use crate::tui::app::App;
use crate::tui::view::widgets::{
    MODAL_HEIGHT, MODAL_WIDTH_PCT, center_rect, editor_spans, rounded_block,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, frame.area());
    frame.render_widget(Clear, area);

    let team = app.channel_browser_team.clone().unwrap_or_default();
    let layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

    let items: Vec<ListItem> = app
        .channels
        .iter()
        .map(|c| {
            let topic = c.channel.topic_name.clone().unwrap_or_default();
            let joined = c.member_status == MemberStatus::Active;
            let (mark, mark_style) = if joined {
                (
                    "✓",
                    Style::default().fg(t.success).add_modifier(Modifier::BOLD),
                )
            } else {
                (" ", Style::default().fg(t.dim))
            };
            let mut spans = vec![
                Span::styled(format!(" {mark} "), mark_style),
                Span::styled(format!("#{topic}"), Style::default().fg(t.foreground)),
            ];
            if !joined {
                spans.push(Span::styled("   · join", Style::default().fg(t.dim)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    state.select(if app.channels.is_empty() {
        None
    } else {
        Some(app.channel_selected.min(app.channels.len() - 1))
    });

    let title = format!(" Channels — {team} · {} ", app.channels.len());
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

    // Bottom row: the create-channel input in create mode, else the hint.
    let bottom = if app.channel_creating {
        let mut spans = vec![Span::styled(" new channel #", Style::default().fg(t.dim))];
        spans.extend(editor_spans(&app.channel_new_name, true, t));
        spans.push(Span::styled(
            "   (Enter create · Esc cancel)",
            Style::default().fg(t.dim),
        ));
        Line::from(spans)
    } else {
        Line::from(Span::styled(
            " Enter: open / join   |   x: leave   |   Alt+N: new   |   F5: refresh   |   Esc: close ",
            Style::default().fg(t.dim),
        ))
    };
    frame.render_widget(Paragraph::new(bottom), layout[1]);
}
