//! Main inbox renderer — the `split_main` stack:
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
    cmdlog_height, draw_cmd_log, draw_search_box, draw_status_strip, favorite_star, list_table,
    list_title, middle_ellipsis, tree_pane_width, unread_dot, unread_style,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // No identity bar on the Home — the user knows who they're logged in as;
    // the rows go to the chat instead.
    let main = Layout::vertical([
        Constraint::Length(3), // shared header row (filter / name / search)
        Constraint::Min(5),    // body (tree | chat)
        Constraint::Length(cmdlog_height(area.height)), // command log (responsive)
        Constraint::Length(1), // status strip
    ])
    .split(area);
    let (header, body, cmdlog, status) = (main[0], main[1], main[2], main[3]);

    // One shared header row: the tree filter (above the tree) + the in-chat
    // search (above the chat). The conversation name lives on the Messages
    // panel title, so it doesn't need its own slot here. The tree column is
    // width-responsive (`tree_pane_width`) — the header + body must match.
    let tree_w = tree_pane_width(area.width);
    let head = Layout::horizontal([Constraint::Length(tree_w), Constraint::Min(20)]).split(header);
    let search_area = head[0];
    let chat_search_area = head[1];

    // Body: the conversation tree (DMs + teams) on the left, the open chat
    // on the right — the unified two-pane "Home".
    let cols = Layout::horizontal([Constraint::Length(tree_w), Constraint::Min(24)]).split(body);
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
    let hint = if app.pending_pane_nav {
        "Ctrl+W move: h/j/k/l or arrows · two keys = diagonal · Esc exit"
    } else if app.focus == Focus::Chat && app.open_conv_id.is_some() {
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
        // Only panel-local actions here — the go-to keys already live in each
        // section's border tag, so don't repeat them.
        Focus::Tree => "↑/↓ nav · l open · n new · r refresh · Shift+I ignore · / filter · Tab",
        Focus::Chat => "Enter send · Ctrl+F search · Alt+V select · Esc back · Tab",
        Focus::CmdLog => "↑/↓ move · Alt+Shift+K/J range · Space mark · y/c copy · Tab",
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
        false, // the tree filter is always available
    );
}

/// A friendly notice inside the Chats panel (empty inbox / no matches): a bold
/// headline + dim hint lines, reusing the panel chrome so it reads as a state,
/// not a glitch.
fn draw_tree_notice(frame: &mut Frame, app: &App, area: Rect, head: &str, hints: &[&str]) {
    let t = &app.theme;
    let focused = app.focus == Focus::Tree;
    let block = titled_block("─[Alt+C]-Chats", focused, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {head}"),
            Style::default()
                .fg(t.foreground)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for h in hints {
        lines.push(Line::from(Span::styled(
            format!("  {h}"),
            Style::default().fg(t.dim),
        )));
    }
    frame.render_widget(
        ratatui::widgets::Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
        inner,
    );
}

/// Left pane: the conversation **tree** — Direct messages + a group per team,
/// collapsible, each (unless folded) followed by its conversations.
fn render_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    // Persistent failure state: the load errored and we have nothing to show.
    // The feedback toast expires after ~1.5 s, so without this the user is left
    // with a blank inbox and only the command log. Show what failed + how to
    // retry, right in the panel.
    if app.conversations.is_empty()
        && let Some(err) = app.inbox_error.clone()
    {
        let focused = app.focus == Focus::Tree;
        let block = titled_block("─[Alt+C]-Chats", focused, app);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let lines = vec![
            Line::from(Span::styled(
                "  ⚠ Couldn't load chats",
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(format!("  {err}"), Style::default().fg(t.dim))),
            Line::from(""),
            Line::from(Span::styled(
                "  r / F5 to retry",
                Style::default().fg(t.foreground),
            )),
        ];
        frame.render_widget(
            ratatui::widgets::Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
            inner,
        );
        return;
    }
    // Signed in and loaded, but the inbox is genuinely empty (no conversations
    // at all) — a friendly welcome instead of a blank list.
    if app.conversations.is_empty() {
        draw_tree_notice(
            frame,
            app,
            area,
            "No conversations yet",
            &["n to start one", "r / F5 to refresh"],
        );
        return;
    }
    let icon_set = crate::tui::icons::resolve(&app.settings_cache.icon_style);
    let model = app.tree_rows();
    // Conversations exist, but the active filter / search matches none of them.
    if model.is_empty() {
        let q = app.search.text().trim().to_string();
        let (head, hint) = if !q.is_empty() {
            (
                format!("No chats match \u{201c}{q}\u{201d}"),
                "Esc clears the filter",
            )
        } else if matches!(app.status_filter, crate::domain::StatusFilter::Unread) {
            ("No unread chats".to_string(), "everything's read \u{2713}")
        } else {
            ("No chats to show".to_string(), "n to start one")
        };
        draw_tree_notice(frame, app, area, &head, &[hint]);
        return;
    }
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
                let icon = if *is_team {
                    icon_set.group_team()
                } else {
                    icon_set.group_dm()
                };
                let color = if *is_team { t.conv_team } else { t.conv_dm };
                let count = if *unread > 0 {
                    unread.to_string()
                } else {
                    String::new()
                };
                // Groups with unread conversations are bold and get a golden
                // ● next to the name (on top of the count) so they stand out.
                let mut style = Style::default().fg(color);
                let mut name_spans = vec![Span::styled(format!("{arrow} {icon} {label}"), style)];
                if *unread > 0 {
                    style = style.add_modifier(Modifier::BOLD);
                    name_spans[0].style = style;
                    name_spans.push(Span::raw(" "));
                    name_spans.push(unread_dot(&t));
                }
                Row::new(vec![
                    ratatui::widgets::Cell::from(Line::from(name_spans)),
                    ratatui::widgets::Cell::from(Span::styled(count, unread_style(&t))),
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
                // Unread (and not locally muted) conversations get a ● symbol
                // AND bold so they're easy to pick out. A locally-muted conv is
                // dimmed and never shows the dot — that's its "quieted" marker.
                let muted = app.is_muted(&conv.id);
                let unread = app.conv_is_unread(conv);
                let style = if unread {
                    unread_style(&t)
                } else if muted {
                    Style::default().fg(t.dim)
                } else {
                    Style::default().fg(t.foreground)
                };
                // Local-only favourites carry a golden ★ (our own star — see
                // `App::favorites`; never synced to Keybase).
                let fav = app.is_favorite(&conv.id);
                let label = middle_ellipsis(&raw, budget.saturating_sub(if fav { 6 } else { 4 }));
                let mut spans = vec![Span::raw("  ")];
                if fav {
                    spans.push(favorite_star(&t));
                    spans.push(Span::raw(" "));
                }
                if unread {
                    spans.push(unread_dot(&t));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(label, style));
                Row::new(vec![
                    ratatui::widgets::Cell::from(Line::from(spans)),
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

/// Linearly blends `a` toward `b` by `f` (0 = `a`, 1 = `b`), for RGB theme
/// colours; a non-RGB colour (a hand-set named value) falls back to `a`. Used
/// to place the placeholder legend a step below `placeholder` toward `muted`.
fn blend(a: ratatui::style::Color, b: ratatui::style::Color, f: f32) -> ratatui::style::Color {
    use ratatui::style::Color::Rgb;
    if let (Rgb(ar, ag, ab), Rgb(br, bg, bb)) = (a, b) {
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
        Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
    } else {
        a
    }
}

/// Right pane shown when no conversation is open. The Chat pane can't be
/// focused with nothing open (Tab skips it, `Alt+M` is gated), so its border is
/// `disabled_block` (muted) — it reads as unreachable, not just unfocused.
fn render_chat_placeholder(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = crate::tui::view::disabled_block("─[Alt+M]-Messages", app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The pane is unreachable (nothing open), so its legend recedes: a touch
    // dimmer than `placeholder`, blended toward the border's `muted`, but kept
    // clear of `muted` itself so it stays legible.
    let legend = blend(t.placeholder, t.muted, 0.5);
    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Select a conversation to start chatting",
            Style::default().fg(legend),
        )),
        Line::from(Span::styled(
            "  Tab to Chats · ↑/↓ pick · Enter to open",
            Style::default().fg(legend),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
