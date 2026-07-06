//! Teams section renderer — the user's team memberships (role + member count),
//! rendered **in the Home shell's right pane** through the shared tabbed picker
//! skeleton, so it reads exactly like the Find / channel-browser sections (one
//! consistent surface with the `Messages · Teams · Find` tabs woven into its
//! top border). It owns only the list, not the command log / status strip.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::domain::TeamRole;
use crate::tui::app::App;
use crate::tui::view::widgets::{
    PickerModal, PickerRow, ScrollTarget, draw_picker_tabbed, empty_state_lines, section_tabs_line,
};

/// Renders the teams list into `area` (the Home right pane). `focused` accents
/// the panel border when the section holds focus.
pub(crate) fn render_list(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    focused: bool,
) {
    let t = &app.theme;
    let icon_set = crate::tui::icons::resolve(&app.settings_cache.icon_style);
    let total = app.teams.list.len();
    let filtered = app.teams.filtered();

    let rows: Vec<PickerRow> = filtered
        .iter()
        .filter_map(|&i| app.teams.list.get(i))
        .map(|tm| {
            let role_color = match tm.role {
                TeamRole::Owner => t.error,
                TeamRole::Admin => t.conv_unread,
                TeamRole::Writer => t.conv_dm,
                TeamRole::Reader => t.dim,
                TeamRole::Bot | TeamRole::RestrictedBot => t.conv_team,
                _ => t.foreground,
            };
            let icon = if tm.is_implicit_team {
                String::new()
            } else {
                format!("{} ", icon_set.group_team())
            };
            PickerRow::Item(vec![Line::from(vec![
                Span::styled(
                    format!("{icon}{}", tm.name),
                    Style::default().fg(t.foreground),
                ),
                Span::styled(
                    format!("   {}", tm.role.label()),
                    Style::default().fg(role_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("   · {} members", tm.member_count),
                    Style::default().fg(t.dim),
                ),
            ])])
        })
        .collect();

    let query = if app.teams.filtering || !app.teams.filter.is_empty() {
        Some((&app.teams.filter, "filter teams…"))
    } else {
        None
    };
    draw_picker_tabbed(
        frame,
        t,
        area,
        focused,
        section_tabs_line(app),
        PickerModal {
            // The `Teams` tab carries the section identity; the title is the
            // right-aligned count detail.
            title: format!("{} of {}", filtered.len(), total),
            query,
            selected: app.teams.selected.min(filtered.len().saturating_sub(1)),
            rows,
            empty: empty_state_lines("No teams loaded", &["r / F5 refresh"], t),
            legend: &[
                ("↑/↓", "nav"),
                ("Enter", "channels"),
                ("/", "filter"),
                ("r", "refresh"),
            ],
            footer: None,
            scroll_target: Some(ScrollTarget::Teams),
        },
    );
}
