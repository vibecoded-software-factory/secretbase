//! Members view (`m` in the channel browser, `Alt+P` on an open team
//! channel): a conversation's members via `listmembers`, role-sorted.
//! Rendered on the shared [`draw_picker_modal`] skeleton; the bottom row
//! swaps to the inline add input or the navigable remove confirm.

use ratatui::{
    Frame,
    style::Style,
    text::{Line, Span},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{
    PickerModal, PickerRow, draw_picker_modal, inline_confirm_line, inline_input_line,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;

    let filtered = app.members.filtered();
    let rows: Vec<PickerRow> = filtered
        .iter()
        .filter_map(|&i| app.members.list.get(i))
        .map(|m| {
            PickerRow::Item(vec![Line::from(vec![
                Span::styled(m.username.clone(), Style::default().fg(t.foreground)),
                Span::styled(format!("   {}", m.role.label()), Style::default().fg(t.dim)),
            ])])
        })
        .collect();

    let footer = if app.members.adding {
        Some(inline_input_line(
            "add (comma/space): ",
            &app.members.add_input,
            "add",
            t,
        ))
    } else {
        app.members.confirm_remove.as_ref().map(|username| {
            inline_confirm_line(
                &format!("Remove {username}?"),
                "",
                "remove",
                app.members.remove_yes,
                t,
            )
        })
    };

    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!(
                "Members — {} · {} of {}",
                app.members.label,
                filtered.len(),
                app.members.list.len()
            ),
            query: if app.members.filtering || !app.members.filter.is_empty() {
                Some((&app.members.filter, "filter members…"))
            } else {
                None
            },
            selected: app.members.selected.min(filtered.len().saturating_sub(1)),
            rows,
            empty: crate::tui::view::widgets::empty_state_lines(
                "No members loaded",
                &["F5 to reload"],
                t,
            ),
            legend: &[
                ("/", "filter"),
                ("a", "add"),
                ("x", "remove"),
                ("F5", "refresh"),
                ("Esc", "close"),
            ],
            footer,
        },
    );
}
