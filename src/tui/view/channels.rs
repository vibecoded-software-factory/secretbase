//! Channel browser (`c` on the inbox, or Enter on a team in the Teams view).
//!
//! Lists **every** channel of a team (`keybase chat api listconvsonname`),
//! marking the ones you're in. `Enter` opens a joined channel or joins one
//! you aren't in; `Shift+L` leaves; `Esc` closes. Rendered on the shared
//! [`draw_picker_modal`] skeleton; the bottom row swaps to the inline
//! create/rename input or the navigable delete confirm.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::domain::MemberStatus;
use crate::tui::app::App;
use crate::tui::view::widgets::{
    PickerModal, PickerRow, draw_picker_modal, inline_confirm_line, inline_input_line,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let team = app.channel_browser_team.clone().unwrap_or_default();

    let filtered = app.channels_filtered();
    let rows: Vec<PickerRow> = filtered
        .iter()
        .filter_map(|&i| app.channels.get(i))
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
                Span::styled(format!("{mark} "), mark_style),
                Span::styled(format!("#{topic}"), Style::default().fg(t.foreground)),
            ];
            if !joined {
                spans.push(Span::styled("   · join", Style::default().fg(t.dim)));
            }
            // Default-channel badge (new members auto-join); `#general` always.
            if topic == "general" || app.default_channels.contains(&topic) {
                spans.push(Span::styled("  ★ default", Style::default().fg(t.accent)));
            }
            PickerRow::Item(vec![Line::from(spans)])
        })
        .collect();

    // Bottom row: create / rename input, delete confirm, else the legend.
    let footer = if app.channel_creating {
        Some(inline_input_line(
            "new channel #",
            &app.channel_new_name,
            "create",
            t,
        ))
    } else if app.channel_renaming.is_some() {
        Some(inline_input_line(
            "rename to #",
            &app.channel_new_name,
            "rename",
            t,
        ))
    } else {
        app.channel_confirm_delete.as_ref().map(|topic| {
            inline_confirm_line(
                &format!("Delete #{topic}?"),
                "irreversible",
                "delete",
                app.channel_delete_yes,
                t,
            )
        })
    };

    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!(
                "Channels — {team} · {} of {}",
                filtered.len(),
                app.channels.len()
            ),
            query: if app.channel_filtering || !app.channel_filter.is_empty() {
                Some((&app.channel_filter, "filter channels…"))
            } else {
                None
            },
            selected: app.channel_selected.min(filtered.len().saturating_sub(1)),
            rows,
            empty: crate::tui::view::widgets::empty_state_lines(
                "No channels loaded",
                &["F5 to refresh"],
                t,
            ),
            legend: &[
                ("/", "filter"),
                ("Enter", "open/join"),
                ("n", "new"),
                ("r", "rename"),
                ("t", "default"),
                ("m", "members"),
                ("Shift+L", "leave"),
                ("x", "del"),
                ("F5", "refresh"),
                ("Esc", "close"),
            ],
            footer,
        },
    );
}
