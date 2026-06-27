//! Main inbox renderer — jewel-style `split_main` stack:
//! identity · search · (filters | list) · command log · status.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::Row,
};

use crate::domain::{InboxSource, MembersType, STATUS_FILTERS, StatusFilter};
use crate::tui::app::App;
use crate::tui::screens::Focus;
use crate::tui::view::split_main;
use crate::tui::view::widgets::{
    draw_cmd_log, draw_identity_bar, draw_search_box, draw_status_strip, identity_content_rows,
    list_table, list_title, middle_ellipsis,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.clamp_list_selected();
    let area = frame.area();
    let id_rows = identity_content_rows(app, area.width);
    let [identity, header, body, cmdlog, status] = split_main(area, id_rows);

    // Discord-style three rails: the source picker (DMs + teams) on the far
    // left, the status filter next, then the conversation list.
    let cols = Layout::horizontal([
        Constraint::Percentage(20), // source rail
        Constraint::Percentage(18), // status filters
        Constraint::Percentage(62), // inbox
    ])
    .split(body);
    let source_area = cols[0];
    let filters_area = cols[1];
    let list_area = cols[2];

    draw_identity_bar(frame, app, identity);
    render_search(frame, app, header);
    render_source(frame, app, source_area);
    render_filters(frame, app, filters_area);
    render_list(frame, app, list_area);
    let cmdlog_focused = app.focus == Focus::CmdLog;
    draw_cmd_log(frame, app, cmdlog, cmdlog_focused, 4);
    let hint = footer_hint(app);
    draw_status_strip(frame, app, status, hint);

    app.mouse_areas.search = header;
    app.mouse_areas.source = source_area;
    app.mouse_areas.filters = filters_area;
    app.mouse_areas.list = list_area;
    app.mouse_areas.cmd_log = cmdlog;
}

fn footer_hint(app: &App) -> &'static str {
    match app.focus {
        Focus::Search => "type to filter · Enter/Esc leave",
        Focus::Source => "↑/↓ pick DMs / a team · Enter apply",
        Focus::Filters => "↑/↓ status filter · Enter apply",
        Focus::List => "↑/↓ nav · Enter open · Alt+N new · Tab focus",
        Focus::CmdLog => "↑/↓ scroll · Tab focus",
    }
}

fn render_search(frame: &mut Frame, app: &App, area: Rect) {
    // draw_search_box prepends the `─[/]-` panel tag itself.
    let title = format!("Search · {} results", app.filtered_cache.len());
    draw_search_box(
        frame,
        app,
        area,
        &title,
        "type / to filter…",
        &app.search,
        app.focus == Focus::Search,
    );
}

/// Far-left rail: the source picker — "Direct messages" + one row per team.
fn render_source(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    let sources = app.inbox_sources();
    let label_budget = (area.width as usize).saturating_sub(9).max(6);

    let rows: Vec<Row<'static>> = sources
        .iter()
        .map(|s| {
            let (icon, color) = match s {
                InboxSource::Dms => ("󰭹 ", t.conv_dm),
                InboxSource::Team(_) => ("󰀎 ", t.conv_team),
            };
            let label = middle_ellipsis(s.label(), label_budget);
            filter_row(format!("{icon}{label}"), color, app.count_source(s), &t)
        })
        .collect();
    let sel = sources
        .iter()
        .position(|s| *s == app.inbox_source)
        .unwrap_or(0);
    let mut scroll = 0usize;
    list_table(
        frame,
        &t,
        area,
        "─[1]-Spaces",
        app.focus == Focus::Source,
        &["Space", "#"],
        &[Constraint::Length(label_budget as u16), Constraint::Min(2)],
        rows,
        sel,
        &mut scroll,
    );
}

/// Status filter rail: All / Unread (scoped to the active source).
fn render_filters(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    let rows: Vec<Row<'static>> = STATUS_FILTERS
        .iter()
        .map(|f| {
            let (icon, color) = status_icon_color(*f, &t);
            filter_row(
                format!("{icon}{}", f.label()),
                color,
                app.count_status(*f),
                &t,
            )
        })
        .collect();
    let sel = STATUS_FILTERS
        .iter()
        .position(|f| *f == app.status_filter)
        .unwrap_or(0);
    let mut scroll = 0usize;
    list_table(
        frame,
        &t,
        area,
        "─[2]-Filters",
        app.focus == Focus::Filters,
        &["Filter", "#"],
        &[Constraint::Length(10), Constraint::Min(3)],
        rows,
        sel,
        &mut scroll,
    );
}

/// Builds one filter row: `<icon+label>` (colored) + a dim count.
fn filter_row(
    label: String,
    color: ratatui::style::Color,
    count: usize,
    t: &crate::tui::theme::Theme,
) -> Row<'static> {
    Row::new(vec![
        ratatui::widgets::Cell::from(Span::styled(label, Style::default().fg(color))),
        ratatui::widgets::Cell::from(Span::styled(count.to_string(), Style::default().fg(t.dim))),
    ])
}

fn status_icon_color(
    f: StatusFilter,
    t: &crate::tui::theme::Theme,
) -> (&'static str, ratatui::style::Color) {
    match f {
        StatusFilter::All => ("  ", t.foreground),
        StatusFilter::Unread => ("● ", t.conv_unread),
    }
}

fn render_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    let total = app.conversations.len();
    let shown = app.filtered_cache.len();

    // Budget for the conversation-label column: total width minus the
    // fixed chrome (border + `▶ ` + marker + spacings + `[TAG]` + border).
    // Head-biased ellipsis keeps the distinguishing front of the name on
    // narrow terminals.
    let label_budget = (area.width as usize).saturating_sub(15).max(12);

    let rows: Vec<Row<'static>> = app
        .filtered_cache
        .iter()
        .map(|&idx| {
            let conv = &app.conversations[idx];
            let tag = conv.channel.members_type.label();
            let tag_color = match conv.channel.members_type {
                MembersType::Team => t.conv_team,
                _ => t.conv_dm,
            };
            let unread_marker = if conv.unread { "●" } else { " " };
            let raw_label = app
                .conversations_lowered
                .get(idx)
                .map(|l| l.display_label.clone())
                .unwrap_or_default();
            let label = middle_ellipsis(&raw_label, label_budget);
            Row::new(vec![
                ratatui::widgets::Cell::from(Span::styled(
                    unread_marker.to_string(),
                    Style::default().fg(t.conv_unread),
                )),
                ratatui::widgets::Cell::from(Span::styled(
                    format!("[{tag}]"),
                    Style::default().fg(tag_color),
                )),
                ratatui::widgets::Cell::from(Span::styled(
                    label,
                    Style::default()
                        .fg(t.foreground)
                        .add_modifier(Modifier::BOLD),
                )),
            ])
        })
        .collect();

    let title = format!("─[3]-{}", list_title("Inbox", shown, total));
    let mut scroll = app.list_scroll;
    list_table(
        frame,
        &t,
        area,
        &title,
        app.focus == Focus::List,
        &["", "Type", "Conversation"],
        &[
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Min(10),
        ],
        rows,
        app.list_selected,
        &mut scroll,
    );
    app.list_scroll = scroll;
}
