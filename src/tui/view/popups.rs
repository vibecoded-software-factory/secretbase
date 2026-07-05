//! Small input popups: reaction picker + quick switcher, both on the shared
//! [`draw_picker_modal`] skeleton.
//!
//! The destructive confirmations (logout, delete message) render through
//! the shared `widgets::draw_confirm_popup` from `view::mod`.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{PickerModal, PickerRow, draw_picker_modal};

/// Searchable reaction picker: a query over the cached emoji catalogue
/// (frecency-sorted), a custom-`:shortcode:` fallback when nothing matches.
/// GIF-search popup — the shared picker-modal skeleton: query on top,
/// one row per hit (title + dim media path tail).
pub fn giphy_search_input(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let rows: Vec<PickerRow> = app
        .giphy_results
        .iter()
        .map(|h| {
            // The media path tail (`/media/<id>/giphy.gif`) is the only
            // distinguishing part of the URL — the host is always giphy.
            let tail = h.url.split("/media/").nth(1).unwrap_or("");
            PickerRow::Item(vec![Line::from(vec![
                Span::styled(format!("{}  ", h.title), Style::default().fg(t.foreground)),
                Span::styled(format!("…/{tail}"), Style::default().fg(t.dim)),
            ])])
        })
        .collect();
    let empty_msg = if app.giphy_input.text().trim().is_empty() {
        "type a query, then Enter"
    } else {
        "no GIFs — Enter to search"
    };
    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!("GIF search · {} hits", app.giphy_results.len()),
            query: Some((&app.giphy_input, "search giphy…")),
            selected: app
                .giphy_selected
                .min(app.giphy_results.len().saturating_sub(1)),
            rows,
            empty: vec![Line::from(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(t.dim),
            ))],
            legend: &[
                ("Enter", "search / send GIF"),
                ("↑↓", "pick"),
                ("Esc", "close"),
            ],
            footer: None,
        },
    );
}

pub fn react_input(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let title = if app.react_to_compose {
        "Insert emoji".to_string()
    } else {
        app.selected_msg_idx
            .and_then(|i| app.messages.get(i))
            .map(|m| format!("React to #{} · by {}", m.id, m.sender))
            .unwrap_or_else(|| "React".to_string())
    };

    let filtered = app.filtered_emoji_indices();
    let rows: Vec<PickerRow> = filtered
        .iter()
        .map(|&ei| {
            let e = &app.emojis[ei];
            PickerRow::Item(vec![Line::from(vec![
                Span::styled(
                    format!("{}  ", e.display),
                    Style::default().fg(t.foreground),
                ),
                Span::styled(format!(":{}:", e.alias), Style::default().fg(t.dim)),
            ])])
        })
        .collect();
    let empty_msg = if app.emojis_loading {
        "loading emojis…"
    } else if app.react.text().trim().is_empty() {
        "type to search, or a :shortcode:"
    } else {
        "no match — Enter sends it as a custom :shortcode:"
    };
    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title,
            query: Some((&app.react, "type to search…")),
            selected: app.react_selected.min(filtered.len().saturating_sub(1)),
            rows,
            empty: vec![Line::from(Span::styled(
                format!("  {empty_msg}"),
                Style::default().fg(t.dim),
            ))],
            legend: if app.react_to_compose {
                &[
                    ("↑↓", "select"),
                    ("Enter", "inserts ▶ into the draft"),
                    ("Esc", "cancel"),
                ]
            } else {
                &[
                    ("↑↓", "select"),
                    ("Enter", "reacts with ▶ (empty query = most-used)"),
                    ("Esc", "cancel"),
                ]
            },
            footer: None,
        },
    );
}

/// Quick switcher (Ctrl+K): Drafts/Unread/Recent sections over all
/// conversations; typing collapses to a flat fuzzy list.
pub fn quick_switcher(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let switcher_rows = app.switcher_rows();
    let mut conv_i = 0usize;
    let rows: Vec<PickerRow> = switcher_rows
        .iter()
        .map(|r| match r {
            crate::tui::app::SwitcherRow::Header(h) => PickerRow::Header(Line::from(Span::styled(
                format!(" {}", h.to_uppercase()),
                Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
            ))),
            crate::tui::app::SwitcherRow::Conv(ci) => {
                let selected = conv_i == app.switcher_selected;
                conv_i += 1;
                // Effective unread (mute-gated) + the shared dot span, so the
                // switcher can't drift from the tree's emphasis.
                let unread = app
                    .conversations
                    .get(*ci)
                    .is_some_and(|c| app.conv_is_unread(c));
                let label = app
                    .conversations_lowered
                    .get(*ci)
                    .map(|l| l.display_label.clone())
                    .unwrap_or_default();
                let mut spans = Vec::new();
                if unread {
                    spans.push(crate::tui::view::widgets::unread_dot(t));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    label,
                    if selected {
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.foreground)
                    },
                ));
                PickerRow::Item(vec![Line::from(spans)])
            }
        })
        .collect();

    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: "Go to a conversation".to_string(),
            query: Some((&app.switcher, "type to search…")),
            selected: app.switcher_selected,
            rows,
            empty: vec![Line::from(Span::styled(
                "  no conversation matches",
                Style::default().fg(t.dim),
            ))],
            legend: &[
                ("↑↓", "select"),
                ("Enter", "go"),
                ("", "type to search"),
                ("Esc", "cancel"),
            ],
            footer: None,
        },
    );
}
