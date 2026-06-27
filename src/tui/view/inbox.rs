//! Main inbox renderer — jewel-style `split_main` stack:
//! identity · search · (filters | list) · command log · status.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::Row,
};

use crate::domain::{ConversationFilter, MembersType};
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

    let cols =
        Layout::horizontal([Constraint::Percentage(22), Constraint::Percentage(78)]).split(body);
    let filters_area = cols[0];
    let list_area = cols[1];

    draw_identity_bar(frame, app, identity);
    render_search(frame, app, header);
    render_filters(frame, app, filters_area);
    render_list(frame, app, list_area);
    let cmdlog_focused = app.focus == Focus::CmdLog;
    draw_cmd_log(frame, app, cmdlog, cmdlog_focused, 4);
    let hint = footer_hint(app);
    draw_status_strip(frame, app, status, hint);

    app.mouse_areas.search = header;
    app.mouse_areas.filters = filters_area;
    app.mouse_areas.list = list_area;
    app.mouse_areas.cmd_log = cmdlog;
}

fn footer_hint(app: &App) -> &'static str {
    match app.focus {
        Focus::Search => "type to filter · Enter/Esc leave",
        Focus::Filters => "↑/↓ filter · Enter apply",
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

fn render_filters(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    // Two stacked panels: general filters (All/Unread) on top, by-type
    // filters (DMs/Teams) below — so the type split reads as its own group.
    let panels = Layout::vertical([
        Constraint::Length(5), // border + header + All + Unread + border
        Constraint::Min(5),    // border + header + DMs + Teams + border
    ])
    .split(area);
    render_filter_group(
        frame,
        app,
        &t,
        panels[0],
        "─[1]-Filters",
        &[ConversationFilter::All, ConversationFilter::Unread],
    );
    render_filter_group(
        frame,
        app,
        &t,
        panels[1],
        "─[2]-By type",
        &[ConversationFilter::Dms, ConversationFilter::Teams],
    );
}

/// Renders one filter panel (a `list_table` over `group`). Only the panel
/// holding `app.active_filter` shows a selection highlight — the others get
/// `usize::MAX`, which `list_table` renders unselected.
fn render_filter_group(
    frame: &mut Frame,
    app: &App,
    t: &crate::tui::theme::Theme,
    area: Rect,
    title: &str,
    group: &[ConversationFilter],
) {
    let rows: Vec<Row<'static>> = group
        .iter()
        .map(|f| {
            let count = app.count_for(f);
            let (icon, color) = filter_icon_and_color(f, t);
            Row::new(vec![
                ratatui::widgets::Cell::from(Span::styled(
                    format!("{icon}{}", f.label()),
                    Style::default().fg(color),
                )),
                ratatui::widgets::Cell::from(Span::styled(
                    count.to_string(),
                    Style::default().fg(t.dim),
                )),
            ])
        })
        .collect();

    let selected = group
        .iter()
        .position(|f| *f == app.active_filter)
        .unwrap_or(usize::MAX);

    let mut scroll = 0usize;
    list_table(
        frame,
        t,
        area,
        title,
        app.focus == Focus::Filters,
        &["Filter", "#"],
        // Only the LAST column may stretch (`Min`); a `Min` on the label
        // (non-final) column would shove the count to the far right with
        // a gap — the recurring list_table "gap" bug.
        &[Constraint::Length(12), Constraint::Min(3)],
        rows,
        selected,
        &mut scroll,
    );
}

fn filter_icon_and_color(
    f: &ConversationFilter,
    t: &crate::tui::theme::Theme,
) -> (&'static str, ratatui::style::Color) {
    match f {
        ConversationFilter::All => ("  ", t.foreground),
        ConversationFilter::Unread => ("● ", t.conv_unread),
        ConversationFilter::Dms => ("󰭹 ", t.conv_dm),
        ConversationFilter::Teams => ("󰀎 ", t.conv_team),
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
