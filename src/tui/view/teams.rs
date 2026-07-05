//! Teams section renderer — a `list_table` of the user's team memberships
//! (role + member count), rendered **in the Home shell's right pane** (it used
//! to be a separate full screen; now it's a section, so this owns only the
//! list, not the command log / status strip).

use ratatui::{
    Frame,
    layout::Constraint,
    style::{Modifier, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::domain::TeamRole;
use crate::tui::app::App;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{empty_state_lines, list_table, list_title};

/// Renders the teams list into `area` (the Home right pane's section content).
/// `focused` accents the panel border when the section holds focus.
pub(crate) fn render_list(
    frame: &mut Frame,
    app: &mut App,
    area: ratatui::layout::Rect,
    focused: bool,
) {
    let t = app.theme.clone();
    let icon_set = crate::tui::icons::resolve(&app.settings_cache.icon_style);
    let total = app.teams.list.len();

    // Empty state that teaches (the tree/channels/members treatment) —
    // this screen used to render a bare table with no way forward.
    if app.teams.list.is_empty() {
        let block = titled_block(" Teams ", focused, app);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            ratatui::widgets::Paragraph::new(empty_state_lines(
                "No teams loaded",
                &["r / F5 refresh", "Esc back to inbox"],
                &t,
            )),
            inner,
        );
        return;
    }

    let filtered = app.teams.filtered();
    let rows: Vec<Row<'static>> = filtered
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
                "  ".to_string()
            } else {
                format!("{} ", icon_set.group_team())
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{icon}{}", tm.name),
                    Style::default().fg(t.foreground),
                )),
                Cell::from(Span::styled(
                    tm.role.label().to_string(),
                    Style::default().fg(role_color).add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(
                    format!("{} members", tm.member_count),
                    Style::default().fg(t.dim),
                )),
            ])
        })
        .collect();

    let shown = filtered.len();
    let mut title = list_title("Teams", shown, total);
    if app.teams.filtering || !app.teams.filter.is_empty() {
        title = format!("{title} /{}", app.teams.filter.text());
    }
    // Size the name column to the visible rows and keep the only
    // stretching `Min` on the LAST column, so slack lands on the right
    // instead of pushing the trailing columns away (the "gap" bug).
    let name_w = filtered
        .iter()
        .filter_map(|&i| app.teams.list.get(i))
        .map(|tm| tm.name.chars().count() + 2)
        .max()
        .unwrap_or(12)
        .clamp(12, 40) as u16;
    let mut scroll = app.teams.scroll;
    list_table(
        frame,
        &t,
        area,
        &title,
        focused,
        &["Team", "Role", "Members"],
        &[
            Constraint::Length(name_w),
            Constraint::Length(8),
            Constraint::Min(14),
        ],
        rows,
        app.teams.selected.min(shown.saturating_sub(1)),
        &mut scroll,
    );
    // Persist the offset the table computed — it was reset every frame,
    // snapping the viewport to the top on each redraw.
    app.teams.scroll = scroll;
}
