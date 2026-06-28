//! Main inbox renderer — jewel-style `split_main` stack:
//! identity · search · (filters | list) · command log · status.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Row},
};

use crate::domain::{InboxSource, STATUS_FILTERS, StatusFilter};
use crate::tui::app::App;
use crate::tui::screens::Focus;
use crate::tui::view::widgets::{
    draw_cmd_log, draw_identity_bar, draw_search_box, draw_status_strip, identity_content_rows,
    list_table, list_title, middle_ellipsis,
};
use crate::tui::view::{split_main, titled_block};

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.clamp_list_selected();
    let area = frame.area();
    let id_rows = identity_content_rows(app, area.width);
    let [identity, header, body, cmdlog, status] = split_main(area, id_rows);

    // Header row: a compact horizontal status filter on the left, the search
    // box filling the rest.
    let head = Layout::horizontal([Constraint::Length(28), Constraint::Min(20)]).split(header);
    let filters_area = head[0];
    let search_area = head[1];

    // Body: a compact source rail (just wide enough for "Direct messages")
    // on the left, the list filling the rest.
    let cols = Layout::horizontal([Constraint::Length(26), Constraint::Min(20)]).split(body);
    let source_area = cols[0];
    let list_area = cols[1];

    draw_identity_bar(frame, app, identity);
    render_filters_bar(frame, app, filters_area);
    render_search(frame, app, search_area);
    render_source(frame, app, source_area);
    render_list(frame, app, list_area);
    let cmdlog_focused = app.focus == Focus::CmdLog;
    draw_cmd_log(frame, app, cmdlog, cmdlog_focused, 4);
    let hint = footer_hint(app);
    draw_status_strip(frame, app, status, hint);

    app.mouse_areas.search = search_area;
    app.mouse_areas.source = source_area;
    app.mouse_areas.filters = filters_area;
    app.mouse_areas.list = list_area;
    app.mouse_areas.cmd_log = cmdlog;
}

fn footer_hint(app: &App) -> &'static str {
    match app.focus {
        Focus::Search => "type to filter · Enter/Esc leave",
        Focus::Source => "↑/↓ pick DMs / a team · Enter apply",
        Focus::Filters => "←/→ All / Unread · Enter apply",
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
        "─[2]-Spaces",
        app.focus == Focus::Source,
        &["Space", "#"],
        &[Constraint::Length(label_budget as u16), Constraint::Min(2)],
        rows,
        sel,
        &mut scroll,
    );
}

/// Compact horizontal status filter in the header (left of search): the
/// All / Unread choices as inline pills, scoped to the active source.
fn render_filters_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, f) in STATUS_FILTERS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let active = *f == app.status_filter;
        let (icon, color) = status_icon_color(*f, t);
        let label = format!(" {icon}{} {} ", f.label(), app.count_status(*f));
        let style = if active {
            Style::default()
                .fg(t.accent)
                .bg(t.selected_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        spans.push(Span::styled(label, style));
    }
    let block = titled_block("─[1]-Filter", app.focus == Focus::Filters, app);
    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
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

    // Single label column so the `▶ ` cursor sits right against the name
    // (jewel-style). Unread is shown by colour + bold, not a marker column.
    let label_budget = (area.width as usize).saturating_sub(6).max(12);
    // Inside a team source the rows are that team's channels, so drop the
    // redundant "team#" prefix and show just the channel name.
    let in_team = matches!(app.inbox_source, InboxSource::Team(_));

    let rows: Vec<Row<'static>> = app
        .filtered_cache
        .iter()
        .map(|&idx| {
            let conv = &app.conversations[idx];
            let raw_label = conv
                .channel
                .topic_name
                .clone()
                .filter(|s| in_team && !s.is_empty())
                .or_else(|| {
                    app.conversations_lowered
                        .get(idx)
                        .map(|l| l.display_label.clone())
                })
                .unwrap_or_default();
            let label = middle_ellipsis(&raw_label, label_budget);
            let style = if conv.unread {
                Style::default()
                    .fg(t.conv_unread)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.foreground)
            };
            Row::new(vec![ratatui::widgets::Cell::from(Span::styled(
                label, style,
            ))])
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
        &["Conversation"],
        &[Constraint::Min(10)],
        rows,
        app.list_selected,
        &mut scroll,
    );
    app.list_scroll = scroll;
}
