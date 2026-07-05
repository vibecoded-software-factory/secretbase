//! In-conversation search modal (`Ctrl+F` / `/` in Select → `searchregexp`):
//! a query box over the match list, jumping to the picked message. Each hit
//! is a two-line item — sender + snippet, then a dim day/time line. Rendered
//! on the shared [`draw_picker_modal`] skeleton; hits are retained for the
//! Select-mode `n`/`N` cycle.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    style::Style,
    text::{Line, Span},
};

use crate::domain::message_time;
use crate::tui::App;
use crate::tui::view::widgets::{
    PickerModal, PickerRow, draw_picker_modal, modal_inner_width, trim_end_ellipsis,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let max_w = modal_inner_width(frame).saturating_sub(4).max(8);

    let rows: Vec<PickerRow> = app
        .conv_search_results
        .iter()
        .map(|hit| {
            let snippet = trim_end_ellipsis(hit.body_summary.lines().next().unwrap_or(""), max_w);
            let when = if hit.sent_at > 0 {
                message_time(hit.sent_at, now_s)
            } else {
                "—".to_string()
            };
            PickerRow::Item(vec![
                Line::from(vec![
                    Span::styled(format!("{}: ", hit.sender), Style::default().fg(t.dim)),
                    Span::styled(snippet, Style::default().fg(t.foreground)),
                ]),
                // dim, not placeholder: the time is why this row exists —
                // secondary-but-needed content never goes in the recessive band.
                Line::from(Span::styled(
                    format!("    {when}"),
                    Style::default().fg(t.dim),
                )),
            ])
        })
        .collect();

    let empty_msg = if app.conv_search.text().trim().is_empty() {
        "type a query, then Enter"
    } else {
        "no matches — Enter to search"
    };
    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: "Search conversation".to_string(),
            query: Some((&app.conv_search, "search this chat…")),
            selected: app
                .conv_search_selected
                .min(app.conv_search_results.len().saturating_sub(1)),
            rows,
            empty: vec![Line::from(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(t.dim),
            ))],
            legend: &[
                ("Enter", "search / jump"),
                ("↑↓", "pick"),
                ("n/N", "cycle after jump"),
                ("Esc", "close"),
            ],
            footer: None,
        },
    );
}
