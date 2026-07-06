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
use crate::tui::view::widgets::{PickerModal, PickerRow, ScrollTarget, draw_picker_modal};

/// Searchable reaction picker: a query over the cached emoji catalogue
/// (frecency-sorted), a custom-`:shortcode:` fallback when nothing matches.
/// GIF-search popup — the shared picker-modal skeleton: query on top,
/// one row per hit (title + dim media path tail).
pub fn giphy_search_input(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let rows: Vec<PickerRow> = app
        .giphy
        .results
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
    let empty = if app.giphy.query.text().trim().is_empty() {
        crate::tui::view::widgets::empty_state_lines(
            "Search giphy",
            &["type a query, then Enter", "Enter on a hit sends the GIF"],
            t,
        )
    } else {
        crate::tui::view::widgets::empty_state_lines(
            "No GIFs",
            &["Enter re-search", "edit the query", "Esc close"],
            t,
        )
    };
    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!("GIF search · {} hits", app.giphy.results.len()),
            query: Some((&app.giphy.query, "search giphy…")),
            selected: app
                .giphy
                .selected
                .min(app.giphy.results.len().saturating_sub(1)),
            rows,
            empty,
            legend: &[
                ("Enter", "search / send GIF"),
                ("↑↓", "pick"),
                ("Esc", "close"),
            ],
            footer: None,
            scroll_target: Some(ScrollTarget::Giphy),
        },
    );
}

pub fn react_input(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let n = app.emoji.filtered().len();
    let title = if app.react_to_compose {
        format!("Insert emoji · {n}")
    } else {
        app.select
            .cursor
            .and_then(|i| app.thread.messages.get(i))
            .map(|m| format!("React to #{} · by {} · {n}", m.id, m.sender))
            .unwrap_or_else(|| format!("React · {n}"))
    };

    let filtered = app.emoji.filtered();
    let rows: Vec<PickerRow> = filtered
        .iter()
        .map(|&ei| {
            let e = &app.emoji.all[ei];
            PickerRow::Item(vec![Line::from(vec![
                Span::styled(
                    format!("{}  ", e.display),
                    Style::default().fg(t.foreground),
                ),
                Span::styled(format!(":{}:", e.alias), Style::default().fg(t.dim)),
            ])])
        })
        .collect();
    let empty = if app.emoji.loading {
        crate::tui::view::widgets::empty_state_lines("Loading emojis…", &[], t)
    } else if app.react.text().trim().is_empty() {
        crate::tui::view::widgets::empty_state_lines(
            "Pick an emoji",
            &["type to search", "empty query = your most-used"],
            t,
        )
    } else {
        crate::tui::view::widgets::empty_state_lines(
            "No match",
            &["Enter sends it as a custom :shortcode:"],
            t,
        )
    };
    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title,
            query: Some((&app.react, "type to search…")),
            selected: app.react_selected.min(filtered.len().saturating_sub(1)),
            rows,
            empty,
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
            scroll_target: Some(ScrollTarget::React),
        },
    );
}

/// Quick switcher (Ctrl+K): Drafts/Unread/Recent sections over all
/// conversations; typing collapses to a flat fuzzy list.
pub fn quick_switcher(frame: &mut Frame, app: &App) {
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
                let selected = conv_i == app.switcher.selected;
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
                // Unseen @mention — the same red badge the tree shows.
                if app
                    .conversations
                    .get(*ci)
                    .is_some_and(|c| app.mentioned.contains(&c.id))
                {
                    spans.push(Span::styled("@ ", t.danger_title()));
                }
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
                // Second line: what we know locally without a fetch — kind,
                // freshness, and your unsent draft's first line (the one
                // preview that actually prevents a wrong jump).
                let meta = app
                    .conversations
                    .get(*ci)
                    .map(|c| {
                        let kind = if c.channel.members_type.is_team() {
                            match c.channel.topic_name.as_deref() {
                                Some(topic) => format!("#{topic}"),
                                None => "team".to_string(),
                            }
                        } else {
                            "dm".to_string()
                        };
                        let age = if c.active_at > 0 && now_s >= c.active_at {
                            crate::domain::format_duration(std::time::Duration::from_secs(
                                now_s - c.active_at,
                            ))
                        } else {
                            String::new()
                        };
                        let mut line = vec![Span::styled(
                            format!("    {kind}{}{age}", if age.is_empty() { "" } else { " · " }),
                            Style::default().fg(t.dim),
                        )];
                        if let Some(d) = app.drafts.get(&c.id).filter(|d| !d.trim().is_empty()) {
                            line.push(Span::styled(" · ✎ ", Style::default().fg(t.accent)));
                            line.push(Span::styled(
                                crate::tui::view::widgets::trim_end_ellipsis(
                                    d.lines().next().unwrap_or(""),
                                    32,
                                ),
                                Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
                            ));
                        }
                        Line::from(line)
                    })
                    .unwrap_or_else(|| Line::from(""));
                PickerRow::Item(vec![Line::from(spans), meta])
            }
        })
        .collect();

    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!("Go to a conversation · {}", app.switcher_selectable().len()),
            query: Some((&app.switcher.query, "type to search…")),
            selected: app.switcher.selected,
            rows,
            empty: crate::tui::view::widgets::empty_state_lines(
                "No conversation matches",
                &["edit the query", "Esc close"],
                t,
            ),
            legend: &[
                ("↑↓", "select"),
                ("Enter", "go"),
                ("", "type to search"),
                ("Esc", "cancel"),
            ],
            footer: None,
            scroll_target: Some(ScrollTarget::Switcher),
        },
    );
}
