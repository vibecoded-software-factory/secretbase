//! Home renderer — the two-pane shell: identity chip + conversation tree on
//! the left, the active section on the right with the `Messages · Teams · Find`
//! tabs woven into its own top border (never a floating row), command log +
//! status strip below. Teams and the channel browser render here as sections,
//! not separate screens.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Row,
};

use crate::tui::app::{App, TreeRow};
use crate::tui::screens::{Focus, Screen};
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{
    cmdlog_height, draw_cmd_log, draw_status_strip, favorite_star, list_table, list_title,
    middle_ellipsis, section_tab_rects, tree_pane_width, unread_dot, unread_style,
};

/// Records the three section-tab hit rects (in `panel`'s top border) so the
/// mouse layer can switch sections on click. Keeps the render and the hit map
/// reading from the same [`section_tab_rects`] geometry.
fn record_section_tabs(app: &mut App, panel: Rect) {
    let (msg, teams, find) = section_tab_rects(panel);
    app.mouse_areas.tab_messages = msg;
    app.mouse_areas.tab_teams = teams;
    app.mouse_areas.tab_find = find;
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // No identity bar on the Home — the user knows who they're logged in as.
    let main = Layout::vertical([
        Constraint::Min(8), // tree column | chat column (both full-height here)
        Constraint::Length(cmdlog_height(area.height, app.settings_cache.cmdlog_rows)), // command log (responsive)
        Constraint::Length(1), // status strip
    ])
    .split(area);
    let (topbody, cmdlog, status) = (main[0], main[1], main[2]);

    // Zoomed (`Ctrl+W z`): the chat column takes everything above the
    // status strip — no tree, no command log — until toggled back.
    if app.pane_zoomed && app.open_conv_id.is_some() {
        let full = Layout::vertical([Constraint::Min(8), Constraint::Length(1)]).split(area);
        // The dynamic island stays above the chat even zoomed (option A).
        let z = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).split(full[0]);
        crate::tui::view::island::draw(frame, app, z[0]);
        crate::tui::view::conversation::draw_chat(frame, app, z[1]);
        record_section_tabs(app, z[1]);
        let hint = if app.pending_pane_nav {
            "Ctrl+W move: h/j/k/l or arrows · z unzoom · Esc exit"
        } else {
            "zoomed — Ctrl+W z restore … "
        };
        draw_status_strip(frame, app, full[1], hint);
        if app.pending_pane_nav {
            crate::tui::view::widgets::draw_which_key(
                frame,
                &app.theme,
                &[("h/j/k/l", "move focus"), ("z", "unzoom"), ("Esc", "exit")],
            );
        }
        return;
    }

    // Two columns: the tree pane (width-responsive) on the left, the chat on the
    // right. The **chat column spans the full height** — its own optional
    // adaptive header (a pin / topic, or nothing) + messages + compose — so no
    // permanent header row is reserved above it.
    let tree_w = tree_pane_width(area.width);
    let cols = Layout::horizontal([Constraint::Length(tree_w), Constraint::Min(24)]).split(topbody);
    let (left_col, right_col) = (cols[0], cols[1]);

    // The right column always reserves the dynamic island (a fixed 3-row band)
    // above the active section — option A: stable height, never reflows.
    let right = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).split(right_col);
    let (island_area, chat_area) = (right[0], right[1]);
    crate::tui::view::island::draw(frame, app, island_area);

    // Left column: the identity chip (3 rows) atop the conversation tree. The
    // chat filter now folds into the Chats panel title (Teams `/query`
    // contract), so no separate search box is reserved.
    let left = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).split(left_col);
    let (identity_area, tree_area) = (left[0], left[1]);

    render_identity(frame, app, identity_area);
    render_tree(frame, app, tree_area);
    // Right pane: the active section fills it, with the `Messages · Teams ·
    // Find` tab bar woven into its own top border (the app's title grammar) —
    // no floating header row, so it stays harmonic with the rest of the UI.
    let section_focused = app.focus == Focus::Chat;
    match app.screen {
        Screen::Teams => {
            crate::tui::view::teams::render_list(frame, app, chat_area, section_focused)
        }
        // The channel browser is the Teams section's drill-down (team → channels).
        Screen::ChannelBrowser => {
            crate::tui::view::channels::render_in_pane(frame, app, chat_area, section_focused)
        }
        _ if app.open_conv_id.is_some() => {
            crate::tui::view::conversation::draw_chat(frame, app, chat_area);
        }
        // No conversation open → the Messages overview (Unread/Mentions) by
        // default, or the Find search landing when the Find tab is active.
        _ if app.find_active => render_find_landing(frame, app, chat_area, section_focused),
        _ => render_messages_overview(frame, app, chat_area, section_focused),
    }
    record_section_tabs(app, chat_area);
    let cmdlog_focused = app.focus == Focus::CmdLog;
    draw_cmd_log(frame, app, cmdlog, cmdlog_focused, "Alt+L");
    let hint = if app.pending_pane_nav {
        "Ctrl+W move: h/j/k/l or arrows · z zoom · two keys = diagonal · Esc exit"
    } else if app.focus == Focus::Chat && app.open_conv_id.is_some() {
        crate::tui::view::conversation::chat_hint(app)
    } else {
        footer_hint(app)
    };
    draw_status_strip(frame, app, status, hint);
    if app.pending_pane_nav {
        crate::tui::view::widgets::draw_which_key(
            frame,
            &app.theme,
            &[
                ("h/j/k/l", "move focus (two keys = diagonal)"),
                ("z", "zoom the chat"),
                ("Esc", "exit"),
            ],
        );
    }

    // The filter lives in the Chats title now; clicking the panel's title +
    // header rows (the `─[Alt+C]-Chats` border and the `Chats · #` header)
    // focuses Search so a mouse user can start filtering. The conversation rows
    // below stay the tree target.
    app.mouse_areas.search = ratatui::layout::Rect {
        x: tree_area.x,
        y: tree_area.y,
        width: tree_area.width,
        height: 2.min(tree_area.height),
    };
    app.mouse_areas.source = tree_area;
    app.mouse_areas.list = chat_area;
    app.mouse_areas.cmd_log = cmdlog;
}

fn footer_hint(app: &App) -> &'static str {
    match app.focus {
        Focus::Search => "type to filter chats · Enter/Esc leave",
        // Only panel-local actions here — the go-to keys already live in each
        // section's border tag, so don't repeat them.
        Focus::Tree => "↑/↓ nav · l open · n new · e read · r refresh · / filter · Tab",
        Focus::Chat => "Enter send · Ctrl+F search · Alt+V select · Esc back · Tab",
        Focus::CmdLog => "↑/↓ move · Alt+Shift+K/J range · Space mark · y/c copy · Tab",
    }
}

/// The account chip atop the tree — your signed-in identity, Discord-style, in
/// the slot the search box used to occupy (its username moved out of the
/// footer). Display-only.
fn render_identity(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let block = titled_block("─ You", false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut spans = vec![Span::raw(" ")];
    if app.identity.logged_in && !app.identity.username.is_empty() {
        // A signed-in dot + the username in accent (this is *you* — primary).
        spans.push(Span::styled("● ", Style::default().fg(t.success)));
        spans.push(Span::styled(
            format!("@{}", app.identity.username),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
        // Device type (desktop / mobile / paper) dim, only if it fits.
        if !app.identity.device_type.is_empty() {
            let extra = format!("  {}", app.identity.device_type);
            let used = app.identity.username.chars().count() + 4; // "● @" + " "
            if used + extra.chars().count() < inner.width as usize {
                spans.push(Span::styled(extra, Style::default().fg(t.dim)));
            }
        }
    } else {
        spans.push(Span::styled("not signed in", Style::default().fg(t.dim)));
    }
    frame.render_widget(ratatui::widgets::Paragraph::new(Line::from(spans)), inner);
}

/// A friendly notice inside the Chats panel (empty inbox / no matches): a bold
/// headline + dim hint lines, reusing the panel chrome so it reads as a state,
/// not a glitch.
fn draw_tree_notice(frame: &mut Frame, app: &App, area: Rect, head: &str, hints: &[&str]) {
    let t = &app.theme;
    let focused = matches!(app.focus, Focus::Tree | Focus::Search);
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
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let model = app.tree_rows();
    // Conversations exist, but the active filter / search matches none of them.
    if model.is_empty() {
        let q = app.search.text().trim().to_string();
        let (head, hint) = if !q.is_empty() {
            (
                format!("No chats match \u{201c}{q}\u{201d}"),
                "Esc clears the filter",
            )
        } else {
            ("No chats to show".to_string(), "n to start one")
        };
        draw_tree_notice(frame, app, area, &head, &[hint]);
        return;
    }
    // Give the name column everything the age column doesn't need: measure
    // the widest age actually shown (2–4 chars — "4d" … "364d") instead of
    // reserving a fixed worst case, and subtract only the real chrome
    // (2 border cols + 1 column-spacing). Short-changing the age clips the
    // unit ("31d" → "31"), so it keeps its measured width whole.
    let age_w = model
        .iter()
        .map(|r| match r {
            TreeRow::Conv { idx } => app
                .conversations
                .get(*idx)
                .filter(|c| c.active_at > 0 && now_s >= c.active_at)
                .map(|c| {
                    crate::domain::format_duration(std::time::Duration::from_secs(
                        now_s - c.active_at,
                    ))
                    .chars()
                    .count()
                })
                .unwrap_or(0),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
        .max(1); // the `#` header
    // Chrome around the name column: 2 border cols + 2 column-spacing +
    // 2 for the `▶ ` highlight gutter.
    let budget = (area.width as usize).saturating_sub(age_w + 6).max(6);

    let rows: Vec<Row<'static>> = model
        .iter()
        .map(|r| match r {
            TreeRow::Group {
                label,
                is_team,
                collapsed,
                unread,
                mentioned,
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
                if *mentioned {
                    name_spans.push(Span::raw(" "));
                    name_spans.push(Span::styled("@", t.danger_title()));
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
                let mentioned = app.mentioned.contains(&conv.id);
                // The label gets exactly what the row's own markers leave:
                // a fixed worst-case subtraction wasted 2–4 columns on every
                // clean row (most of the tree).
                // A stashed draft and a local mute both need a *positive*
                // marker: an unsent draft was invisible outside the quick
                // switcher, and a muted row (dim, no dot) read the same as
                // "just read".
                let has_draft = app.drafts.contains_key(&conv.id);
                let prefix_w = 2
                    + if fav { 2 } else { 0 }
                    + if unread { 2 } else { 0 }
                    + if mentioned { 2 } else { 0 }
                    + if has_draft { 2 } else { 0 }
                    + if muted { 2 } else { 0 };
                let label = middle_ellipsis(&raw, budget.saturating_sub(prefix_w));
                let mut spans = vec![Span::raw("  ")];
                if fav {
                    spans.push(favorite_star(&t));
                    spans.push(Span::raw(" "));
                }
                if unread {
                    spans.push(unread_dot(&t));
                    spans.push(Span::raw(" "));
                }
                // Unseen @mention of you — the strongest pull in the tree
                // (Keybase GUI uses red for the same signal).
                if mentioned {
                    spans.push(Span::styled("@ ", t.danger_title()));
                }
                // Unsent draft — accent pencil, the strongest "you left
                // something here" cue short of the mention badge.
                if has_draft {
                    spans.push(Span::styled("✎ ", Style::default().fg(t.accent)));
                }
                // Local mute — a positive glyph, not just the absence of a
                // dot (matches the [M]-marker convention of chat TUIs).
                if muted {
                    spans.push(Span::styled("⊘ ", Style::default().fg(t.dim)));
                }
                spans.push(Span::styled(label, style));
                // Compact relative age in the `#` column (empty on group
                // rows): how stale is this conversation, at a glance.
                let age = if conv.active_at > 0 && now_s >= conv.active_at {
                    crate::domain::format_duration(std::time::Duration::from_secs(
                        now_s - conv.active_at,
                    ))
                } else {
                    String::new()
                };
                Row::new(vec![
                    ratatui::widgets::Cell::from(Line::from(spans)),
                    ratatui::widgets::Cell::from(Span::styled(age, Style::default().fg(t.dim))),
                ])
            }
        })
        .collect();

    // Count in the bottom-right border: rows currently visible in the tree
    // (group headers + the conversations of any expanded group) of the total
    // conversations — so it tracks what's actually shown as you fold/unfold.
    let mut title = format!(
        "─[Alt+C]-{}",
        list_title("Chats", model.len(), app.conversations.len())
    );
    // The chat filter folds into the title (Teams `/query` contract) — `/` or
    // Alt+F starts filtering, the query shows live, Esc clears and leaves.
    let filtering = app.focus == Focus::Search;
    if filtering || !app.search.text().trim().is_empty() {
        title = format!("{title} /{}", app.search.text());
    }
    let mut scroll = app.list_scroll;
    list_table(
        frame,
        &t,
        area,
        &title,
        matches!(app.focus, Focus::Tree | Focus::Search),
        &["Chats", "#"],
        &[Constraint::Length(budget as u16), Constraint::Min(4)],
        rows,
        app.tree_selected,
        &mut scroll,
    );
    app.list_scroll = scroll;
    crate::tui::view::widgets::register_scroll(area, crate::tui::view::widgets::ScrollTarget::Tree);
}

/// Right pane when no conversation is open — a spacious **"Find a
/// conversation"** landing (Discord's Friends view): the shared `/` filter
/// query + the filtered conversations as rich two-line rows, on the same picker
/// skeleton every overlay uses (so it's one aesthetic, not a one-off). `↑/↓`
/// pick, `Enter` opens; `/` filters (the same `app.search` query as the tree —
/// one query, two views). Replaces the old dead "Select a conversation" notice.
/// A rich two-line conversation row for the Find landing / Messages overview:
/// attention badges + name, then a dim `kind · age` meta line. Shared so both
/// list surfaces read identically.
fn conv_row(app: &App, i: usize, now_s: u64) -> crate::tui::view::widgets::PickerRow {
    use crate::tui::view::widgets::PickerRow;
    let t = &app.theme;
    let conv = &app.conversations[i];
    let name = app
        .conversations_lowered
        .get(i)
        .map(|l| l.display_label.clone())
        .unwrap_or_default();
    let unread = app.conv_is_unread(conv);
    let mut l1: Vec<Span<'static>> = vec![Span::raw(" ")];
    if app.is_favorite(&conv.id) {
        l1.push(favorite_star(t));
        l1.push(Span::raw(" "));
    }
    if unread {
        l1.push(unread_dot(t));
        l1.push(Span::raw(" "));
    }
    if app.mentioned.contains(&conv.id) {
        l1.push(Span::styled("@ ", t.danger_title()));
    }
    if app
        .drafts
        .get(&conv.id)
        .is_some_and(|d| !d.trim().is_empty())
    {
        l1.push(Span::styled("✎ ", Style::default().fg(t.accent)));
    }
    let name_style = if unread {
        unread_style(t)
    } else {
        Style::default().fg(t.foreground)
    };
    l1.push(Span::styled(name, name_style));
    let kind = conv.channel.members_type.label();
    let meta = if conv.active_at > 0 && now_s >= conv.active_at {
        let age =
            crate::domain::format_duration(std::time::Duration::from_secs(now_s - conv.active_at));
        format!("   {kind} · {age}")
    } else {
        format!("   {kind}")
    };
    PickerRow::Item(vec![
        Line::from(l1),
        Line::from(Span::styled(meta, Style::default().fg(t.dim))),
    ])
}

/// The **Messages overview** — the right pane when the Messages section is
/// active and no conversation is open (the default landing): the day's activity
/// grouped into **Unread** and **Mentions**, browsable, `Enter` opens. Distinct
/// from the Find search landing (the `Find` tab).
fn render_messages_overview(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    use crate::tui::view::widgets::{
        PickerModal, PickerRow, ScrollTarget, draw_picker_tabbed, empty_state_lines,
        section_tabs_line,
    };
    let t = &app.theme;
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let unread = app.overview_unread();
    let mentions = app.overview_mentions();
    let section_header = |label: String| {
        PickerRow::Header(Line::from(Span::styled(
            format!("  {label}"),
            Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
        )))
    };
    let mut rows: Vec<PickerRow> = Vec::new();
    if !unread.is_empty() {
        rows.push(section_header(format!("Unread ({})", unread.len())));
        rows.extend(unread.iter().map(|&i| conv_row(app, i, now_s)));
    }
    if !mentions.is_empty() {
        rows.push(section_header(format!("Mentions ({})", mentions.len())));
        rows.extend(mentions.iter().map(|&i| conv_row(app, i, now_s)));
    }
    let items = unread.len() + mentions.len();
    draw_picker_tabbed(
        frame,
        t,
        area,
        focused,
        section_tabs_line(app),
        PickerModal {
            title: format!("{} unread · {} mentions", unread.len(), mentions.len()),
            query: None,
            selected: app.overview_selected.min(items.saturating_sub(1)),
            rows,
            empty: empty_state_lines(
                "All caught up ✨",
                &["/ or the Find tab to search", "Ctrl+K to jump"],
                t,
            ),
            legend: &[
                ("↑/↓", "pick"),
                ("Enter", "open"),
                ("/", "find"),
                ("Tab", "section"),
            ],
            footer: None,
            scroll_target: Some(ScrollTarget::Overview),
        },
    );
}

fn render_find_landing(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    use crate::tui::view::widgets::{
        PickerModal, PickerRow, ScrollTarget, draw_picker_tabbed, empty_state_lines,
        section_tabs_line,
    };
    let t = &app.theme;
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut rows: Vec<PickerRow> = app
        .filtered_cache
        .iter()
        .map(|&i| conv_row(app, i, now_s))
        .collect();
    // A dim column header over the list — the same `Chats` header the tree
    // panel carries, so the Find landing frames its rows identically. Headers
    // are non-selectable, so they don't shift the `find_selected` item indices.
    if !rows.is_empty() {
        rows.insert(
            0,
            PickerRow::Header(Line::from(Span::styled(
                "  Chats",
                Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
            ))),
        );
    }
    draw_picker_tabbed(
        frame,
        t,
        area,
        focused,
        section_tabs_line(app),
        PickerModal {
            // The `Find` tab carries the section identity; the title is just the
            // right-aligned count detail.
            title: format!(
                "{} of {}",
                app.filtered_cache.len(),
                app.conversations.len()
            ),
            query: Some((&app.search, "search your chats…")),
            selected: app
                .find_selected
                .min(app.filtered_cache.len().saturating_sub(1)),
            rows,
            empty: empty_state_lines(
                "No conversations to show",
                &["/ to filter", "n to start one", "Ctrl+K to jump"],
                t,
            ),
            legend: &[
                ("↑/↓", "pick"),
                ("Enter", "open"),
                ("/", "filter"),
                ("n", "new"),
                ("t", "teams"),
            ],
            footer: None,
            scroll_target: Some(ScrollTarget::Find),
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::domain::{TeamMembership, TeamRole};
    use crate::tui::app::App;
    use crate::tui::screens::{Focus, Screen};

    struct NoClip;
    impl crate::ports::ClipboardPort for NoClip {
        fn write(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    struct NoOpen;
    impl crate::ports::OpenerPort for NoOpen {
        fn open(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    struct NoSettings;
    impl crate::ports::SettingsPort for NoSettings {
        fn read(&self) -> crate::ports::UserSettings {
            crate::ports::UserSettings::default()
        }
        fn write_setting(&self, _: &str, _: &str) -> bool {
            true
        }
        fn write_theme_name(&self, _: &str) -> bool {
            true
        }
        fn config_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::from(".")
        }
    }

    fn app() -> App {
        use std::sync::mpsc::channel;
        let (tx, _r1) = channel();
        let (bg, _r2) = channel();
        let (_t3, rx) = channel();
        App::new(
            tx,
            bg,
            rx,
            None,
            Box::new(NoClip),
            Box::new(NoOpen),
            Box::new(NoSettings),
        )
    }

    fn render_to_text(app: &mut App) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut term = Terminal::new(TestBackend::new(90, 30)).unwrap();
        term.draw(|f| crate::tui::view::draw(f, app)).unwrap();
        let buf = term.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                if let Some(c) = buf.cell((x, y)) {
                    s.push_str(c.symbol());
                }
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn teams_section_renders_in_the_right_pane_with_a_tab_header() {
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.teams.list = vec![TeamMembership {
            name: "phoenix".into(),
            is_implicit_team: false,
            member_count: 3,
            role: TeamRole::Owner,
        }];
        app.screen = Screen::Teams;
        let text = render_to_text(&mut app);
        // The section tab bar names both apartados; the team renders on the right.
        assert!(text.contains("Messages"), "tab bar shows Messages:\n{text}");
        assert!(text.contains("Teams"), "tab bar shows Teams:\n{text}");
        assert!(
            text.contains("phoenix"),
            "the team renders in the right pane:\n{text}"
        );
    }

    #[test]
    fn identity_chip_shows_the_signed_in_username() {
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "511v3str1".into();
        app.screen = Screen::Inbox;
        let text = render_to_text(&mut app);
        assert!(
            text.contains("@511v3str1"),
            "the identity chip shows the username:\n{text}"
        );
    }

    #[test]
    fn channel_browser_renders_in_pane_as_a_teams_drilldown() {
        use crate::domain::{Channel, Conversation, MemberStatus, MembersType};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.channel_browser.team = Some("phoenix".into());
        app.channel_browser.channels = vec![Conversation {
            id: "c1".into(),
            channel: Channel {
                name: "phoenix".into(),
                members_type: MembersType::Team,
                topic_name: Some("general".into()),
            },
            unread: false,
            active_at: 0,
            active_at_ms: 0,
            member_status: MemberStatus::Active,
            creator_info: None,
        }];
        app.screen = Screen::ChannelBrowser;
        app.focus = Focus::Chat;
        let text = render_to_text(&mut app);
        // The drill-down renders in the right pane (not a centered modal): the
        // section identity lives in the `Teams` tab woven into the border, and
        // the team name is the right-aligned drill-down detail.
        assert!(
            text.contains("#phoenix"),
            "drill-down shows the team:\n{text}"
        );
        assert!(text.contains("general"), "the channel renders:\n{text}");
        assert!(text.contains("Teams"), "Teams tab stays active:\n{text}");
    }

    #[test]
    fn open_conversation_shows_the_messages_tab_and_name_in_its_border() {
        use crate::domain::{Channel, Conversation, MembersType};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.conversations = vec![Conversation {
            id: "cv1".into(),
            channel: Channel {
                name: "me,zoe".into(),
                members_type: MembersType::ImpTeamNative,
                topic_name: None,
            },
            unread: false,
            active_at: 0,
            active_at_ms: 0,
            member_status: crate::domain::MemberStatus::Active,
            creator_info: None,
        }];
        app.rebuild_lowered();
        app.rebuild_filter_preserving_cursor();
        app.open_conv_id = Some("cv1".into());
        app.screen = Screen::Inbox;
        app.focus = Focus::Chat;
        let text = render_to_text(&mut app);
        // The Messages section carries the same tabbed border as the others: the
        // `Messages` tab is active and the conversation name is the detail.
        assert!(text.contains("Messages"), "Messages tab in border:\n{text}");
        assert!(
            text.contains("Teams"),
            "the section tabs are woven in:\n{text}"
        );
        assert!(
            text.contains("zoe"),
            "the conversation name is the detail:\n{text}"
        );
    }

    #[test]
    fn wheel_over_the_find_landing_moves_its_selection() {
        // End-to-end: a real draw registers the Find landing as a scroll region;
        // a wheel event over the right pane dispatches through the centralized
        // registry → `apply_scroll` → the Find cursor. No per-screen mouse code.
        use crate::domain::{Channel, Conversation, MembersType};
        use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.conversations = (0..4)
            .map(|i| Conversation {
                id: format!("cv{i}"),
                channel: Channel {
                    name: format!("chat{i}"),
                    members_type: MembersType::ImpTeamNative,
                    topic_name: None,
                },
                unread: false,
                active_at: 0,
                active_at_ms: 0,
                member_status: crate::domain::MemberStatus::Active,
                creator_info: None,
            })
            .collect();
        app.rebuild_lowered();
        app.rebuild_filter_preserving_cursor();
        app.screen = Screen::Inbox; // no conversation open → the Find landing
        app.find_active = true; // exercise the Find landing (overview is default)
        app.focus = Focus::Chat;
        let _ = render_to_text(&mut app); // populates the scroll registry
        // The TestBackend is 90×30; align the resize guard so the click counts.
        app.last_terminal_size = app.mouse_areas.frame_size;
        assert_eq!(app.find_selected, 0);
        // Wheel down over the right pane (col 60 is inside the chat column).
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 60,
                row: 10,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(app.find_selected, 1, "the wheel moved the Find selection");
    }

    #[test]
    fn dynamic_island_shows_attention_when_unread_and_is_padded() {
        use crate::domain::{Channel, Conversation, MembersType};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.conversations = vec![Conversation {
            id: "cv1".into(),
            channel: Channel {
                name: "me,zoe".into(),
                members_type: MembersType::ImpTeamNative,
                topic_name: None,
            },
            unread: true,
            active_at: 0,
            active_at_ms: 0,
            member_status: crate::domain::MemberStatus::Active,
            creator_info: None,
        }];
        app.rebuild_lowered();
        app.rebuild_filter_preserving_cursor();
        app.screen = Screen::Inbox;
        let text = render_to_text(&mut app);
        // The island band morphs to the Attention state and reads roomy — the
        // count is padded away from the border and the dot breathes.
        assert!(
            text.contains("Attention"),
            "island shows the Attention state:\n{text}"
        );
        assert!(
            text.contains("● 1 unread"),
            "dot + count breathe, not glued:\n{text}"
        );
        assert!(
            text.contains("│  ●") || text.contains("  ● 1 unread"),
            "content is inset from the left border:\n{text}"
        );
    }

    #[test]
    fn clicking_the_confirm_button_commits_and_cancel_dismisses() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        // Render the logout confirm so its `[ confirm ] [ cancel ]` buttons
        // register their rects; a click on `[ confirm ]` commits (leaves the
        // popup). center_rect(80,22) on 90×30 → modal x=9,y=4; the action row is
        // the modal's bottom line (y=24), `[ confirm ]` at x≈12.
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.screen = Screen::ConfirmLogout;
        let _ = render_to_text(&mut app);
        app.last_terminal_size = app.mouse_areas.frame_size;
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 15,
                row: 24,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_ne!(
            app.screen,
            Screen::ConfirmLogout,
            "clicking [ confirm ] committed / left the popup"
        );
    }

    #[test]
    fn clicking_the_help_anchor_opens_help() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.screen = Screen::Inbox;
        let _ = render_to_text(&mut app);
        app.last_terminal_size = app.mouse_areas.frame_size;
        // The `F1 help · F10 settings` anchor is right-aligned on the status row
        // (row 29 of 90×30); `F1 help` occupies cols 68–74.
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 70,
                row: 29,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.screen,
            Screen::Help,
            "clicking the F1 anchor opened help"
        );
    }

    #[test]
    fn clicking_a_settings_sidebar_section_selects_it() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.screen = Screen::Settings;
        app.settings_ui.section = 0; // Identity
        let text = render_to_text(&mut app);
        app.last_terminal_size = app.mouse_areas.frame_size;
        // Find the "Theme" section label in the sidebar (the panel title is
        // "Identity" here, so "Theme" only appears as a sidebar row).
        let (col, row) = text
            .lines()
            .enumerate()
            .find_map(|(y, line)| line.find("Theme").map(|x| (x as u16, y as u16)))
            .expect("Theme section row is rendered");
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: col,
                row,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.settings_ui.section, 1,
            "clicking the Theme sidebar row selected it (index 1)"
        );
    }

    #[test]
    fn clicking_a_login_field_focuses_it() {
        use crate::tui::app::LoginField;
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = app();
        app.screen = Screen::Login;
        app.login.focus = LoginField::PaperKey;

        // Render tall enough for the whole form + locate the Username label.
        let mut term = Terminal::new(TestBackend::new(90, 44)).unwrap();
        term.draw(|f| crate::tui::view::draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        let mut text = String::new();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                if let Some(c) = buf.cell((x, y)) {
                    text.push_str(c.symbol());
                }
            }
            text.push('\n');
        }
        app.last_terminal_size = app.mouse_areas.frame_size;
        let (col, label_y) = text
            .lines()
            .enumerate()
            .find_map(|(y, line)| line.find("Username").map(|x| (x as u16, y as u16)))
            .expect("Username label is rendered");
        // The input block sits on the row just below its label.
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: col + 1,
                row: label_y + 1,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.login.focus,
            LoginField::Username,
            "clicking the Username input focused it"
        );
    }

    #[test]
    fn command_log_shows_a_scrollbar_when_it_overflows() {
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.settings_cache.cmdlog_rows = 6;
        for i in 0..40 {
            app.push_cmd(format!("cmd {i}"), true, "ok");
        }
        app.screen = Screen::Inbox;
        let text = render_to_text(&mut app);
        assert!(
            text.contains('┃'),
            "the command log renders a scrollbar thumb when it overflows:\n{text}"
        );
    }

    #[test]
    fn clicking_the_chats_header_focuses_search() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.screen = Screen::Inbox;
        app.focus = Focus::Tree;
        let _ = render_to_text(&mut app); // populates the Chats header hit rect
        app.last_terminal_size = app.mouse_areas.frame_size;
        // The Chats panel's title row sits at the top-left; click its border row.
        let r = app.mouse_areas.search;
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: r.x + 2,
                row: r.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.focus,
            Focus::Search,
            "clicking the Chats header focuses the filter"
        );
    }

    #[test]
    fn clicking_the_menu_anchor_opens_the_command_palette() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.screen = Screen::Inbox;
        let _ = render_to_text(&mut app);
        app.last_terminal_size = app.mouse_areas.frame_size;
        // The `☰ menu` anchor is the left-most of the right-aligned status anchor
        // (`☰ menu · F1 help · F10 settings`, 31 chars) on the status row (29).
        // area starts after the mode badge (`-- NORMAL -- `, 13); ends at 90 →
        // the anchor starts at col 59, `☰ menu` spans 59–64.
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 61,
                row: 29,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert_eq!(
            app.screen,
            Screen::CommandPalette,
            "clicking the ☰ menu anchor opened the command palette"
        );
    }

    #[test]
    fn find_landing_carries_a_chats_header_over_its_list() {
        use crate::domain::{Channel, Conversation, MembersType};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.conversations = vec![Conversation {
            id: "cv1".into(),
            channel: Channel {
                name: "me,zoe".into(),
                members_type: MembersType::ImpTeamNative,
                topic_name: None,
            },
            unread: false,
            active_at: 0,
            active_at_ms: 0,
            member_status: crate::domain::MemberStatus::Active,
            creator_info: None,
        }];
        app.rebuild_lowered();
        app.rebuild_filter_preserving_cursor();
        app.screen = Screen::Inbox; // no conversation open → the Find landing
        app.find_active = true; // exercise the Find landing (overview is default)
        let text = render_to_text(&mut app);
        // The tree carries `Chats` in its title + header; the Find landing adds
        // a third — its own `Chats` list header — so it frames its rows the same.
        assert!(
            text.matches("Chats").count() >= 3,
            "the Find landing renders a `Chats` list header like the tree:\n{text}"
        );
    }

    #[test]
    fn messages_overview_is_the_default_and_groups_unread() {
        use crate::domain::{Channel, Conversation, MembersType};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.conversations = vec![Conversation {
            id: "cv1".into(),
            channel: Channel {
                name: "me,zoe".into(),
                members_type: MembersType::ImpTeamNative,
                topic_name: None,
            },
            unread: true,
            active_at: 0,
            active_at_ms: 0,
            member_status: crate::domain::MemberStatus::Active,
            creator_info: None,
        }];
        app.rebuild_lowered();
        app.rebuild_filter_preserving_cursor();
        app.screen = Screen::Inbox; // no conversation open, find_active = false
        let text = render_to_text(&mut app);
        // The default landing is the Messages overview (not Find): it groups the
        // day's activity under an `Unread` header, and the Messages tab is active.
        assert!(
            text.contains("Unread (1)"),
            "the overview groups unread conversations:\n{text}"
        );
        assert!(
            !text.contains("search your chats"),
            "the default landing is the overview, not the Find search:\n{text}"
        );
    }

    #[test]
    fn clicking_the_find_tab_switches_from_the_overview_to_search() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        app.identity.logged_in = true;
        app.identity.username = "me".into();
        app.screen = Screen::Inbox;
        assert!(!app.find_active, "overview is the default");
        let _ = render_to_text(&mut app); // records the section-tab hit rects
        app.last_terminal_size = app.mouse_areas.frame_size;
        let ft = app.mouse_areas.tab_find;
        crate::tui::input::mouse::handle(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: ft.x + 1,
                row: ft.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert!(
            app.find_active,
            "clicking the Find tab flipped the pane to the search landing"
        );
    }
}
