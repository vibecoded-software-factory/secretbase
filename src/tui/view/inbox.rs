//! Main inbox renderer — jewel-style `split_main` stack:
//! identity · search · (filters | list) · command log · status.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Row},
};

use crate::domain::{STATUS_FILTERS, StatusFilter};
use crate::tui::app::{App, TreeRow};
use crate::tui::screens::Focus;
use crate::tui::view::widgets::{
    draw_cmd_log, draw_identity_bar, draw_search_box, draw_skeleton, draw_status_strip,
    identity_content_rows, list_table, middle_ellipsis,
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

    // Body: the conversation tree (DMs + teams) on the left, the open chat
    // on the right — the unified two-pane "Home".
    let cols = Layout::horizontal([Constraint::Length(28), Constraint::Min(24)]).split(body);
    let tree_area = cols[0];
    let chat_area = cols[1];

    draw_identity_bar(frame, app, identity);
    render_filters_bar(frame, app, filters_area);
    render_search(frame, app, search_area);
    render_tree(frame, app, tree_area);
    if app.open_conv_id.is_some() {
        crate::tui::view::conversation::draw_chat(frame, app, chat_area);
    } else {
        render_chat_placeholder(frame, app, chat_area);
    }
    let cmdlog_focused = app.focus == Focus::CmdLog;
    draw_cmd_log(frame, app, cmdlog, cmdlog_focused, 4);
    let hint = if app.focus == Focus::Chat && app.open_conv_id.is_some() {
        crate::tui::view::conversation::chat_hint(app)
    } else {
        footer_hint(app)
    };
    draw_status_strip(frame, app, status, hint);

    app.mouse_areas.search = search_area;
    app.mouse_areas.source = tree_area;
    app.mouse_areas.filters = filters_area;
    app.mouse_areas.list = chat_area;
    app.mouse_areas.cmd_log = cmdlog;
}

fn footer_hint(app: &App) -> &'static str {
    match app.focus {
        Focus::Search => "type to filter · Enter/Esc leave",
        Focus::Filters => "←/→ All / Unread · Enter apply",
        Focus::Tree => "↑/↓ nav · Enter open / fold · Alt+N new · Tab focus",
        Focus::Chat => "Enter send · Esc back · Tab focus",
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

/// Left pane: the conversation **tree** — Direct messages + a group per team,
/// collapsible, each (unless folded) followed by its conversations.
fn render_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    // First (expensive) inbox load — show a skeleton instead of an empty tree.
    if app.conversations.is_empty() && app.is_busy() {
        draw_skeleton(frame, &t, area, "─[2]-Chats", app.anim_tick);
        return;
    }
    let model = app.tree_rows();
    let budget = (area.width as usize).saturating_sub(7).max(6);

    let rows: Vec<Row<'static>> = model
        .iter()
        .map(|r| match r {
            TreeRow::Group {
                label,
                is_team,
                collapsed,
                unread,
                ..
            } => {
                let arrow = if *collapsed { "▸" } else { "▾" };
                let icon = if *is_team { "󰀎" } else { "󰭹" };
                let color = if *is_team { t.conv_team } else { t.conv_dm };
                let count = if *unread > 0 {
                    unread.to_string()
                } else {
                    String::new()
                };
                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(
                        format!("{arrow} {icon} {label}"),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        count,
                        Style::default().fg(t.conv_unread),
                    )),
                ])
            }
            TreeRow::Conv { idx } => {
                let conv = &app.conversations[*idx];
                let raw = if conv.channel.members_type.is_team() {
                    conv.channel.topic_name.clone().filter(|s| !s.is_empty())
                } else {
                    None
                }
                .or_else(|| {
                    app.conversations_lowered
                        .get(*idx)
                        .map(|l| l.display_label.clone())
                })
                .unwrap_or_default();
                let label = middle_ellipsis(&raw, budget.saturating_sub(2));
                let style = if conv.unread {
                    Style::default()
                        .fg(t.conv_unread)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.foreground)
                };
                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(format!("  {label}"), style)),
                    ratatui::widgets::Cell::from(Span::raw("")),
                ])
            }
        })
        .collect();

    let mut scroll = app.list_scroll;
    list_table(
        frame,
        &t,
        area,
        "─[2]-Chats",
        app.focus == Focus::Tree,
        &["Chats", "#"],
        &[Constraint::Length(budget as u16), Constraint::Min(2)],
        rows,
        app.tree_selected,
        &mut scroll,
    );
    app.list_scroll = scroll;
}

/// Right pane shown when no conversation is open.
fn render_chat_placeholder(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = titled_block("─[3]-Chat", app.focus == Focus::Chat, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Select a conversation to start chatting",
            Style::default().fg(t.dim),
        )),
        Line::from(Span::styled(
            "  Tab to Chats · ↑/↓ pick · Enter to open",
            Style::default().fg(t.placeholder),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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

fn status_icon_color(
    f: StatusFilter,
    t: &crate::tui::theme::Theme,
) -> (&'static str, ratatui::style::Color) {
    match f {
        StatusFilter::All => ("  ", t.foreground),
        StatusFilter::Unread => ("● ", t.conv_unread),
    }
}
