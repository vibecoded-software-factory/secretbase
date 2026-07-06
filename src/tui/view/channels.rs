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

use ratatui::layout::Rect;

use crate::domain::MemberStatus;
use crate::tui::app::App;
use crate::tui::view::widgets::{
    PickerModal, PickerRow, draw_picker_tabbed, inline_confirm_line, inline_input_line,
    section_tabs_line,
};

/// Renders the channel browser **in the Home shell's right pane** (the Teams
/// section's drill-down: team → its channels), not as a floating modal. Same
/// picker skeleton, just drawn into `area`; `focused` accents its border.
pub(crate) fn render_in_pane(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    let t = &app.theme;
    let team = app.channel_browser.team.clone().unwrap_or_default();

    let filtered = app.channel_browser.filtered();
    let rows: Vec<PickerRow> = filtered
        .iter()
        .filter_map(|&i| app.channel_browser.channels.get(i))
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
            if topic == "general" || app.channel_browser.defaults.contains(&topic) {
                spans.push(Span::styled("  ★ default", Style::default().fg(t.accent)));
            }
            PickerRow::Item(vec![Line::from(spans)])
        })
        .collect();

    // Bottom row: create / rename input, delete confirm, else the legend.
    let footer = if app.channel_browser.creating {
        Some(inline_input_line(
            "new channel #",
            &app.channel_browser.new_name,
            "create",
            t,
        ))
    } else if app.channel_browser.renaming.is_some() {
        Some(inline_input_line(
            "rename to #",
            &app.channel_browser.new_name,
            "rename",
            t,
        ))
    } else {
        app.channel_browser.confirm_delete.as_ref().map(|topic| {
            inline_confirm_line(
                &format!("Delete #{topic}?"),
                "irreversible",
                "delete",
                app.channel_browser.delete_yes,
                t,
            )
        })
    };

    draw_picker_tabbed(
        frame,
        t,
        area,
        focused,
        section_tabs_line(app),
        PickerModal {
            // The `Teams` tab carries the section; the title is the drill-down
            // detail (team → its channel count).
            title: format!(
                "#{team} · {} of {}",
                filtered.len(),
                app.channel_browser.channels.len()
            ),
            query: if app.channel_browser.filtering || !app.channel_browser.filter.is_empty() {
                Some((&app.channel_browser.filter, "filter channels…"))
            } else {
                None
            },
            selected: app
                .channel_browser
                .selected
                .min(filtered.len().saturating_sub(1)),
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
