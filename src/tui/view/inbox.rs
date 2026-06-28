//! Main inbox renderer — jewel-style `split_main` stack:
//! identity · search · (filters | list) · command log · status.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Row},
};

use crate::tui::app::{App, TreeRow};
use crate::tui::screens::Focus;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{
    draw_cmd_log, draw_search_box, draw_skeleton, draw_status_strip, list_table, list_title,
    middle_ellipsis,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.clamp_list_selected();
    let area = frame.area();
    // No identity bar on the Home — the user knows who they're logged in as;
    // the rows go to the chat instead.
    let main = Layout::vertical([
        Constraint::Length(3), // shared header row (filter / name / search)
        Constraint::Min(5),    // body (tree | chat)
        Constraint::Length(6), // command log
        Constraint::Length(1), // status strip
    ])
    .split(area);
    let (header, body, cmdlog, status) = (main[0], main[1], main[2], main[3]);

    // One shared header row: the tree filter (above the tree) + the in-chat
    // search (above the chat). The conversation name lives on the Messages
    // panel title, so it doesn't need its own slot here.
    let head = Layout::horizontal([Constraint::Length(28), Constraint::Min(20)]).split(header);
    let search_area = head[0];
    let chat_search_area = head[1];

    // Body: the conversation tree (DMs + teams) on the left, the open chat
    // on the right — the unified two-pane "Home".
    let cols = Layout::horizontal([Constraint::Length(28), Constraint::Min(24)]).split(body);
    let tree_area = cols[0];
    let chat_area = cols[1];

    render_search(frame, app, search_area);
    crate::tui::view::conversation::draw_chat_header(frame, app, chat_search_area);
    render_tree(frame, app, tree_area);
    if app.open_conv_id.is_some() {
        crate::tui::view::conversation::draw_chat(frame, app, chat_area);
    } else {
        render_chat_placeholder(frame, app, chat_area);
    }
    let cmdlog_focused = app.focus == Focus::CmdLog;
    draw_cmd_log(frame, app, cmdlog, cmdlog_focused, "Alt+L");
    let hint = if matches!(app.focus, Focus::Chat | Focus::ChatSearch) && app.open_conv_id.is_some()
    {
        crate::tui::view::conversation::chat_hint(app)
    } else {
        footer_hint(app)
    };
    draw_status_strip(frame, app, status, hint);

    app.mouse_areas.search = search_area;
    app.mouse_areas.source = tree_area;
    app.mouse_areas.list = chat_area;
    app.mouse_areas.cmd_log = cmdlog;
}

fn footer_hint(app: &App) -> &'static str {
    match app.focus {
        Focus::Search => "type to filter chats · Enter/Esc leave",
        Focus::Tree => {
            "↑/↓ nav · Enter open/fold · Alt+F filter · Alt+M chat · Ctrl+F find · Alt+N new"
        }
        Focus::Chat => "Enter send · Esc back · Tab focus",
        Focus::ChatSearch => "type · Enter jump · Esc close · Tab focus",
        Focus::CmdLog => "↑/↓ move · Shift+↑/↓ range · Space mark · y/c copy · Tab",
    }
}

fn render_search(frame: &mut Frame, app: &App, area: Rect) {
    // draw_search_box prepends the `─[Alt+S]-` panel tag itself.
    let title = format!("Search · {} results", app.filtered_cache.len());
    draw_search_box(
        frame,
        app,
        area,
        "Alt+F",
        &title,
        "type to filter chats…",
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
        draw_skeleton(
            frame,
            &t,
            area,
            "─[Alt+C]-Chats",
            app.anim_tick,
            &["Chats", "#"],
            "Loading chats…",
        );
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
                // Groups with unread conversations are bold (on top of the
                // count) so they stand out.
                let mut style = Style::default().fg(color);
                if *unread > 0 {
                    style = style.add_modifier(Modifier::BOLD);
                }
                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(
                        format!("{arrow} {icon} {label}"),
                        style,
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        count,
                        Style::default()
                            .fg(t.conv_unread)
                            .add_modifier(Modifier::BOLD),
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
                // Unread conversations get a ● symbol AND bold so they're
                // easy to pick out; read ones are plain.
                let (prefix, style) = if conv.unread {
                    (
                        "● ",
                        Style::default()
                            .fg(t.conv_unread)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("", Style::default().fg(t.foreground))
                };
                let label = middle_ellipsis(&raw, budget.saturating_sub(4));
                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(format!("  {prefix}{label}"), style)),
                    ratatui::widgets::Cell::from(Span::raw("")),
                ])
            }
        })
        .collect();

    // Count in the bottom-right border: rows currently visible in the tree
    // (group headers + the conversations of any expanded group) of the total
    // conversations — so it tracks what's actually shown as you fold/unfold.
    let title = format!(
        "─[Alt+C]-{}",
        list_title("Chats", model.len(), app.conversations.len())
    );
    let mut scroll = app.list_scroll;
    list_table(
        frame,
        &t,
        area,
        &title,
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
    let block = titled_block("─[Alt+M]-Chat", app.focus == Focus::Chat, app);
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
