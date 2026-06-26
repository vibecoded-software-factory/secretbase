//! Server-side `searchinbox` popup (Ctrl+G).
//!
//! Top: input box. Bottom: scrollable results list. Enter on a result
//! jumps to the conversation; Esc closes.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{center_rect, editor_spans};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(80, 22, frame.area());
    frame.render_widget(Clear, area);

    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    // Input box.
    frame.render_widget(
        Paragraph::new(Line::from(editor_spans(&app.search_global_input, true, t))).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    "[/] Global Search",
                    Style::default().fg(t.accent),
                ))
                .title_bottom(
                    Line::from(Span::styled(
                        format!("─{} results─", app.search_global_results.len()),
                        Style::default().fg(t.dim),
                    ))
                    .right_aligned(),
                )
                .border_style(Style::default().fg(t.accent)),
        ),
        layout[0],
    );

    // Results list.
    let items: Vec<ListItem> = app
        .search_global_results
        .iter()
        .map(|h| {
            let header = Line::from(vec![
                Span::styled(
                    format!(" {} ", h.conv_name),
                    Style::default()
                        .fg(t.conv_team)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("@{} ", h.sender), Style::default().fg(t.dim)),
                Span::styled(format!("#{}", h.message_id), Style::default().fg(t.dim)),
            ]);
            let body_summary: String = h.body_summary.lines().next().unwrap_or("").to_string();
            let body = Line::from(Span::styled(
                format!("    {body_summary}"),
                Style::default().fg(t.foreground),
            ));
            ListItem::new(vec![header, body])
        })
        .collect();

    let mut state = ListState::default();
    state.select(if app.search_global_results.is_empty() {
        None
    } else {
        Some(
            app.search_global_selected
                .min(app.search_global_results.len() - 1),
        )
    });

    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled("Hits", Style::default().fg(t.accent)))
                    .border_style(Style::default().fg(t.accent)),
            )
            .highlight_style(Style::default().bg(t.selected_bg))
            .highlight_symbol("▶ "),
        layout[1],
        &mut state,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Enter: open   |   ↑/↓: navigate   |   Esc: close ",
            Style::default().fg(t.dim),
        ))),
        layout[2],
    );
}
