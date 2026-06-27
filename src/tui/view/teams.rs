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
use crate::tui::view::widgets::{
    draw_cmd_log, draw_identity_bar, draw_status_strip, identity_content_rows, list_table,
    list_title,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let id_rows = identity_content_rows(app, area.width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(id_rows + 2),
            Constraint::Min(5),
            Constraint::Length(6),
            Constraint::Length(1),
        ])
        .split(area);

    draw_identity_bar(frame, app, chunks[0]);
    render_list(frame, app, chunks[1]);
    draw_cmd_log(frame, app, chunks[2], false, 2);
    draw_status_strip(
        frame,
        app,
        chunks[3],
        "↑/↓ nav · Alt+R refresh · Alt+I/Esc inbox",
    );
}

fn render_list(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    let t = app.theme.clone();
    let total = app.teams.len();

    let rows: Vec<Row<'static>> = app
        .teams
        .iter()
        .map(|tm| {
            let role_color = match tm.role {
                TeamRole::Owner => t.error,
                TeamRole::Admin => t.conv_unread,
                TeamRole::Writer => t.conv_dm,
                TeamRole::Reader => t.dim,
                TeamRole::Bot | TeamRole::RestrictedBot => t.conv_team,
                _ => t.foreground,
            };
            let icon = if tm.is_implicit_team { "  " } else { "󰀎 " };
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

    let title = format!("─[1]-{}", list_title("Teams", total, total));
    // Size the name column to the visible rows and keep the only
    // stretching `Min` on the LAST column, so slack lands on the right
    // instead of pushing the trailing columns away (the "gap" bug).
    let name_w = app
        .teams
        .iter()
        .map(|tm| tm.name.chars().count() + 2)
        .max()
        .unwrap_or(12)
        .clamp(12, 40) as u16;
    let mut scroll = 0usize;
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
        app.teams_selected,
        &mut scroll,
    );
}
