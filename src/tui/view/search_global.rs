//! Server-side `searchinbox` popup (Ctrl+G): a query over every
//! conversation; each hit is a two-line item (conversation + sender + id,
//! then the snippet). Enter jumps to the conversation **and** the matched
//! message. Rendered on the shared [`draw_picker_modal`] skeleton — the
//! documented sibling of the in-conversation search.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{PickerModal, PickerRow, draw_picker_modal};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;

    let rows: Vec<PickerRow> = app
        .search_global_results
        .iter()
        .map(|h| {
            let body_summary: String = h.body_summary.lines().next().unwrap_or("").to_string();
            PickerRow::Item(vec![
                Line::from(vec![
                    Span::styled(
                        format!("{} ", h.conv_name),
                        Style::default()
                            .fg(t.conv_team)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("@{} ", h.sender), Style::default().fg(t.dim)),
                    Span::styled(format!("#{}", h.message_id), Style::default().fg(t.dim)),
                ]),
                Line::from(Span::styled(
                    format!("    {body_summary}"),
                    Style::default().fg(t.foreground),
                )),
            ])
        })
        .collect();

    let empty_msg = if app.search_global_input.text().trim().is_empty() {
        "type a query, then Enter"
    } else {
        "no hits — Enter to search"
    };
    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!("Global search · {} hits", app.search_global_results.len()),
            query: Some((&app.search_global_input, "search every conversation…")),
            selected: app
                .search_global_selected
                .min(app.search_global_results.len().saturating_sub(1)),
            rows,
            empty: vec![Line::from(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(t.dim),
            ))],
            legend: &[
                ("Enter", "search / jump to message"),
                ("↑↓", "pick"),
                ("Esc", "close"),
            ],
            footer: None,
        },
    );
}
