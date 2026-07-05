//! Teams screen renderer — identity bar + a `list_table` of the user's
//! team memberships (role + member count), command log, status strip.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::Span,
    widgets::{Cell, Row},
};

use crate::domain::TeamRole;
use crate::tui::app::App;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{
    cmdlog_height, draw_cmd_log, draw_status_strip, empty_state_lines, list_table, list_title,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // No identity bar here — the Teams screen is a focused list; the identity /
    // unread chrome belongs on the inbox home, not on this drill-down.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(cmdlog_height(area.height, app.settings_cache.cmdlog_rows)), // responsive command log
            Constraint::Length(1),
        ])
        .split(area);

    render_list(frame, app, chunks[0]);
    draw_cmd_log(frame, app, chunks[1], false, "");
    let hint = if app.teams_filtering {
        "type to filter · Enter keep · Esc clear"
    } else if !app.teams_filter.is_empty() {
        "filtered — Esc clear · / edit · Enter channels"
    } else {
        "↑/↓ nav · Enter channels · / filter · r refresh · Esc inbox"
    };
    draw_status_strip(frame, app, chunks[2], hint);
}

fn render_list(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let t = app.theme.clone();
    let icon_set = crate::tui::icons::resolve(&app.settings_cache.icon_style);
    let total = app.teams.len();

    // Empty state that teaches (the tree/channels/members treatment) —
    // this screen used to render a bare table with no way forward.
    if app.teams.is_empty() {
        let block = titled_block(" Teams ", true, app);
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

    let filtered = app.teams_filtered();
    let rows: Vec<Row<'static>> = filtered
        .iter()
        .filter_map(|&i| app.teams.get(i))
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
    if app.teams_filtering || !app.teams_filter.is_empty() {
        title = format!("{title} /{}", app.teams_filter.text());
    }
    // Size the name column to the visible rows and keep the only
    // stretching `Min` on the LAST column, so slack lands on the right
    // instead of pushing the trailing columns away (the "gap" bug).
    let name_w = filtered
        .iter()
        .filter_map(|&i| app.teams.get(i))
        .map(|tm| tm.name.chars().count() + 2)
        .max()
        .unwrap_or(12)
        .clamp(12, 40) as u16;
    let mut scroll = app.teams_scroll;
    list_table(
        frame,
        &t,
        area,
        &title,
        true,
        &["Team", "Role", "Members"],
        &[
            Constraint::Length(name_w),
            Constraint::Length(8),
            Constraint::Min(14),
        ],
        rows,
        app.teams_selected.min(shown.saturating_sub(1)),
        &mut scroll,
    );
    // Persist the offset the table computed — it was reset every frame,
    // snapping the viewport to the top on each redraw.
    app.teams_scroll = scroll;
}
