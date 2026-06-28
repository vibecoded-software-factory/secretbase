//! Single-conversation detail view — identity bar, conversation header,
//! scrollable message history, the compose pane and the status strip.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::domain::{AttachmentInfo, Message, MessageContent, SystemInfo, message_time};
use crate::tui::app::App;
use crate::tui::screens::Focus;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{draw_search_box, editor_lines};

/// Renders the chat's in-conversation **search** box (`searchregexp`, Ctrl+F)
/// into `area`. The conversation name now lives on the Messages panel title
/// ([`chat_title`]), so the header is just this search box.
pub(crate) fn draw_chat_header(frame: &mut Frame, app: &App, area: Rect) {
    draw_search_box(
        frame,
        app,
        area,
        "Alt+F",
        "Search",
        "search this chat",
        &app.conv_search,
        app.focus == Focus::ChatSearch,
    );
}

/// Renders the chat **body** — message history + compose box — into `area`.
/// The header is drawn separately ([`draw_chat_header`]) so it can live on the
/// unified Home's shared top row.
pub(crate) fn draw_chat(frame: &mut Frame, app: &mut App, area: Rect) {
    // Compose grows with its line count (multi-line via Alt+Enter), capped.
    let compose_lines = app.compose.text().split('\n').count().max(1) as u16;
    let compose_h = (compose_lines + 2).clamp(3, 8);
    let layout = Layout::vertical([
        Constraint::Min(3),            // messages
        Constraint::Length(compose_h), // compose pane (dynamic)
    ])
    .split(area);

    render_messages(frame, app, layout[0]);
    render_compose(frame, app, layout[1]);
}

/// The status-strip hint for the chat, by interaction mode.
pub(crate) fn chat_hint(app: &App) -> &'static str {
    if app.conv_search_active {
        "type · Enter search/jump · ↑/↓ pick · Esc close"
    } else if app.selected_msg_idx.is_some() {
        "↑/↓ move · Shift+↑/↓ range · Space mark · y/c copy · : react · Esc back"
    } else if app.edit_target_id.is_some() {
        "Enter save edit · Esc cancel"
    } else {
        "Enter send · Alt+A attach · Alt+V select · Esc back"
    }
}

fn render_compose(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let editing = app.edit_target_id.is_some();
    let replying = app.reply_to_id;

    let title = if let Some(id) = app.edit_target_id {
        format!("Editing msg #{id}")
    } else if let Some(id) = replying {
        format!("Replying to #{id}")
    } else {
        "Compose".to_string()
    };
    let placeholder = if editing {
        "edit text below — Enter saves, Esc cancels"
    } else if replying.is_some() {
        "type your reply — Enter sends, Esc cancels"
    } else {
        "type and press Enter to send…"
    };
    let lines: Vec<Line> = if app.compose.is_empty() {
        vec![Line::from(Span::styled(
            placeholder,
            Style::default().fg(t.placeholder),
        ))]
    } else {
        editor_lines(&app.compose, t)
    };
    // Vertically scroll so the cursor's row stays visible when the draft has
    // more lines than the (capped) box can show.
    let inner_h = area.height.saturating_sub(2) as usize;
    let cur = app.compose.cursor().min(app.compose.text().len());
    let cursor_row = app.compose.text()[..cur].matches('\n').count();
    let scroll = cursor_row.saturating_sub(inner_h.saturating_sub(1)) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .block(titled_block(&title, app.focus == Focus::Chat, app)),
        area,
    );
}

/// The Messages-panel title: `Messages — <conversation name>` (plus `📌 #id`
/// when a message is pinned). Folding the name in here lets us drop the
/// separate "Conversation" header box.
fn chat_title(app: &App) -> String {
    let name = app
        .open_conv_id
        .as_deref()
        .and_then(|id| {
            app.conversations
                .iter()
                .position(|c| c.id == id)
                .and_then(|idx| {
                    app.conversations_lowered
                        .get(idx)
                        .map(|l| l.display_label.clone())
                })
        })
        .unwrap_or_default();
    // `─[Alt+M]-` border tag → Alt+M focuses the chat (works mid-compose).
    let mut title = if name.is_empty() {
        "─[Alt+M]-Messages".to_string()
    } else {
        format!("─[Alt+M]-Messages — {name}")
    };
    if let Some(pid) = app.pinned_msg_id {
        title.push_str(&format!("  📌 #{pid}"));
    }
    title
}

/// Renders the `searchregexp` match list in the message viewport while the
/// in-conversation search is active. `Enter` jumps to the highlighted hit.
fn render_conv_search_results(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let results = &app.conv_search_results;
    let sel = app
        .conv_search_selected
        .min(results.len().saturating_sub(1));
    // Each result spans two rows (snippet + a dim day/time line), so the
    // viewport and scroll are computed in result units, not raw rows.
    let vh = area.height.saturating_sub(2).max(1) as usize;
    let per_view = (vh / 2).max(1);
    let scroll = if sel >= per_view {
        sel + 1 - per_view
    } else {
        0
    };
    let max_w = area.width.saturating_sub(6).max(8) as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, hit) in results.iter().enumerate().skip(scroll).take(per_view) {
        let selected = i == sel;
        let snippet: String = hit
            .body_summary
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(max_w)
            .collect();
        let mut spans = vec![
            Span::styled(
                if selected { "▶ " } else { "  " }.to_string(),
                Style::default().fg(t.accent),
            ),
            Span::styled(format!("{}: ", hit.sender), Style::default().fg(t.dim)),
            Span::styled(snippet, Style::default().fg(t.foreground)),
        ];
        // Day + time under the snippet, for context (à la the chat header).
        let when = if hit.sent_at > 0 {
            message_time(hit.sent_at, now_s)
        } else {
            "—".to_string()
        };
        let mut time_spans = vec![Span::styled(
            format!("      {when}"),
            Style::default().fg(t.placeholder),
        )];
        if selected {
            for s in &mut spans {
                s.style = s.style.bg(t.selected_bg);
            }
            for s in &mut time_spans {
                s.style = s.style.bg(t.selected_bg);
            }
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(time_spans));
    }
    let title = format!("Matches · {}", results.len());
    frame.render_widget(
        Paragraph::new(lines).block(titled_block(&title, true, app)),
        area,
    );
}

fn render_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    // While searching the conversation, the body shows the match list.
    if app.conv_search_active && !app.conv_search_results.is_empty() {
        render_conv_search_results(frame, app, area);
        return;
    }
    let t = app.theme.clone();
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Content width inside the panel borders — message bodies wrap to it.
    let body_width = area.width.saturating_sub(2) as usize;
    // Optimistic outbox bubbles for this conversation (sending / failed),
    // rendered below the loaded history.
    let outbox = outbox_lines(app, now_s, &t, body_width);

    if app.messages.is_empty() && outbox.is_empty() {
        // Distinguish the initial fetch (LoadMessages in flight) from a
        // genuinely empty conversation — showing "empty" while we're still
        // downloading the history is misleading.
        let loading = matches!(
            app.in_flight,
            Some(crate::tui::worker::InFlight::LoadMessages)
        );
        let lines = if loading {
            vec![
                Line::from(Span::raw("")),
                Line::from(Span::styled(
                    "  Loading messages…",
                    Style::default().fg(t.dim),
                )),
            ]
        } else {
            vec![
                Line::from(Span::raw("")),
                Line::from(Span::styled(
                    "  this conversation is empty",
                    Style::default().fg(t.dim),
                )),
                Line::from(Span::styled(
                    "  type below and press Enter to start it",
                    Style::default().fg(t.placeholder),
                )),
            ]
        };
        let title = chat_title(app);
        frame.render_widget(
            Paragraph::new(lines).block(titled_block(&title, app.focus == Focus::Chat, app)),
            area,
        );
        app.messages_max_back = 0;
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(app.messages.len() * 3 + outbox.len());

    if !app.messages.is_empty() {
        if app.messages_next.is_some() {
            lines.push(Line::from(Span::styled(
                "  ↑ press Up to load older messages",
                Style::default().fg(t.dim),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  · beginning of conversation",
                Style::default().fg(t.dim),
            )));
        }
        lines.push(Line::from(Span::raw("")));
    }

    let mut selected_line: Option<usize> = None;
    // Line span [start, end) each message occupies, for click-to-select.
    let mut spans_map: Vec<(usize, usize, usize)> = Vec::with_capacity(app.messages.len());
    for (idx, m) in app.messages.iter().enumerate() {
        let start = lines.len();
        let is_selected = app.selected_msg_idx == Some(idx);
        // Marked messages (multi-select for copy) get the same shading as the
        // cursor — the cursor is told apart by its action bar below.
        let is_marked = app.msg_marks.contains(&idx);
        if is_selected {
            selected_line = Some(start);
        }
        let mut block = message_lines(m, now_s, app, &t, body_width);
        if is_selected || is_marked {
            for line in &mut block {
                let bg = t.selected_bg;
                line.spans = line
                    .spans
                    .iter()
                    .map(|s| {
                        let style = s.style.bg(bg);
                        Span::styled(s.content.clone(), style)
                    })
                    .collect();
            }
        }
        lines.extend(block);
        // In select mode, show a contextual action bar under the highlighted
        // message — visual feedback for what can be done with it (the keys
        // still work directly).
        if is_selected && app.selected_msg_idx.is_some() {
            lines.push(select_actions_line(m, app, &t));
        }
        // Compact: no blank line between messages — each message's
        // "→ sender · time" header already separates them.
        spans_map.push((start, lines.len(), idx));
    }

    // Optimistic sends sit at the very bottom, after the loaded history.
    lines.extend(outbox);

    let total_lines = lines.len();
    let viewport = area.height.saturating_sub(2).max(1) as usize;
    let max_back = total_lines.saturating_sub(viewport);
    let effective_back = app.messages_scroll.min(max_back);
    let mut scroll_y = max_back.saturating_sub(effective_back);

    // In Select mode, keep the highlighted message inside the viewport —
    // cursor navigation alone never scrolls, so it could otherwise drift
    // above the fold with no way back.
    if let Some(sel) = selected_line.filter(|_| app.selected_msg_idx.is_some()) {
        if sel < scroll_y {
            scroll_y = sel;
        } else if sel >= scroll_y + viewport {
            scroll_y = sel + 1 - viewport;
        }
    }

    // Count + scroll position live in the bottom-right border (dim), the
    // same place every other list panel shows its count — the title stays
    // a plain "Messages". (Pagination means there's no true total, so this
    // is loaded-count + scroll position, not an "X of Y".)
    // `n msgs` = messages currently loaded (paginated; older ones load on
    // scroll-up). The word reports where the viewport sits — no raw line
    // offset, which mixed units (lines vs messages) and read as confusing.
    let n = app.messages.len();
    let counter = if app.messages_loading_older {
        format!("{n} msgs · loading older…")
    } else if max_back == 0 {
        // Everything fits — no scrollback.
        format!("{n} msgs")
    } else if effective_back == 0 {
        // Pinned to the newest message.
        format!("{n} msgs · latest")
    } else if effective_back == max_back {
        // Top of what's loaded: more history on the server, or the very
        // start of the conversation.
        if app.messages_next.is_some() {
            format!("{n} msgs · ↑ more above")
        } else {
            format!("{n} msgs · oldest")
        }
    } else {
        // Scrolled up into older messages, mid-history.
        format!("{n} msgs · ↑ older")
    };

    // Ratatui scroll is u16; saturate so a very long history can't wrap
    // the offset to a tiny value via a truncating cast.
    let scroll_u16 = scroll_y.min(u16::MAX as usize) as u16;
    let dim = app.theme.dim;
    let title = chat_title(app);
    let block = titled_block(&title, app.focus == Focus::Chat, app)
        .title_bottom(Line::from(Span::styled(counter, Style::default().fg(dim))).right_aligned());
    frame.render_widget(
        Paragraph::new(lines).scroll((scroll_u16, 0)).block(block),
        area,
    );

    app.messages_max_back = max_back;

    // Mouse hit-testing: the viewport (for scroll) + a screen rect per
    // visible message (for click-to-select). Content starts one row inside
    // the top border; line `L` shows at `area.y + 1 + (L - scroll_y)`.
    app.mouse_areas.messages = area;
    let inner_top = area.y + 1;
    let mut rows = Vec::new();
    for (start, end, idx) in spans_map {
        let vis_start = start.max(scroll_y);
        let vis_end = end.min(scroll_y + viewport);
        if vis_start < vis_end {
            rows.push((
                Rect {
                    x: area.x,
                    y: inner_top + (vis_start - scroll_y) as u16,
                    width: area.width,
                    height: (vis_end - vis_start) as u16,
                },
                idx,
            ));
        }
    }
    app.mouse_areas.message_rows = rows;
}

/// Builds the bubbles for the optimistic outbox entries targeting the open
/// conversation: `○ sending…`, a delivered `→` (transient, pruned by the
/// reconciling re-read), and a red `✗ failed · Alt+R to resend`.
fn outbox_lines(
    app: &App,
    now_s: u64,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'static>> {
    use crate::tui::app::SendState;
    let Some(conv_id) = app.open_conv_id.as_deref() else {
        return Vec::new();
    };
    let me = app.identity.username.clone();
    let sender_style = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for p in app.outbox.iter().filter(|p| p.conv_id == conv_id) {
        let (icon, icon_color, status) = match p.state {
            SendState::Pending => (
                "○",
                t.dim,
                Span::styled(
                    "sending…".to_string(),
                    Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
                ),
            ),
            SendState::Delivered => (
                "→",
                t.dim,
                Span::styled(
                    message_time(p.sent_at_ms / 1000, now_s),
                    Style::default().fg(t.dim),
                ),
            ),
            SendState::Failed => (
                "✗",
                t.error,
                Span::styled(
                    "failed · Alt+R to resend".to_string(),
                    Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                ),
            ),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {icon} "), Style::default().fg(icon_color)),
            Span::styled(format!("{me} "), sender_style),
            status,
        ]));
        lines.extend(body_lines(&MessageContent::Text(p.body.clone()), t, width));
        lines.push(Line::from(Span::raw("")));
    }
    lines
}

fn message_lines(
    m: &Message,
    now_s: u64,
    app: &App,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let is_system = matches!(
        m.content,
        MessageContent::System(_)
            | MessageContent::Metadata { .. }
            | MessageContent::Headline { .. }
            | MessageContent::Join { .. }
            | MessageContent::Leave { .. }
            | MessageContent::Pin { .. }
    );
    let is_me = !app.identity.username.is_empty() && m.sender == app.identity.username;
    let sender_style = if is_system {
        Style::default().fg(t.dim).add_modifier(Modifier::ITALIC)
    } else if is_me {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.conv_dm).add_modifier(Modifier::BOLD)
    };
    let when = message_time(m.sent_at, now_s);
    let header_icon = if is_me && !is_system {
        "→"
    } else {
        content_icon(&m.content)
    };
    let is_pinned = app.pinned_msg_id == Some(m.id);
    let mut header_spans: Vec<Span<'static>> = vec![
        Span::styled(format!(" {header_icon} "), Style::default().fg(t.dim)),
        Span::styled(format!("{} ", m.sender), sender_style),
        Span::styled(when, Style::default().fg(t.dim)),
    ];
    if is_pinned {
        header_spans.push(Span::styled(
            "  📌 pinned",
            Style::default().fg(t.conv_unread),
        ));
    }
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(3);
    lines.push(Line::from(header_spans));
    // Threaded reply: quote the message being replied to, above the body.
    if let Some(target) = m.reply_to {
        lines.push(reply_quote_line(target, app, t));
    }
    lines.extend(body_lines(&m.content, t, width));
    if !m.reactions.is_empty() {
        lines.push(reactions_line(&m.reactions, app, t));
    }
    lines
}

/// A dim, italic quote of the message a reply targets (`↩ sender · snippet`),
/// rendered just above the reply's own body. Falls back to `↩ #id` when the
/// parent isn't in the loaded history.
fn reply_quote_line(target: u64, app: &App, t: &crate::tui::theme::Theme) -> Line<'static> {
    let label = app
        .messages
        .iter()
        .find(|m| m.id == target)
        .map(|m| {
            let body = match &m.content {
                MessageContent::Text(b) => b.as_str(),
                MessageContent::Edit { body, .. } => body.as_str(),
                MessageContent::Attachment(a) => a.filename.as_str(),
                _ => "",
            };
            let snippet: String = body.lines().next().unwrap_or("").chars().take(48).collect();
            if snippet.is_empty() {
                format!("#{target}")
            } else {
                format!("{} · {snippet}", m.sender)
            }
        })
        .unwrap_or_else(|| format!("#{target}"));
    Line::from(Span::styled(
        format!("   ↩ {label}"),
        Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
    ))
}

fn reactions_line(
    reactions: &[crate::domain::Reaction],
    app: &App,
    t: &crate::tui::theme::Theme,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("    ")];
    for (i, r) in reactions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let count = r.usernames.len();
        spans.push(Span::styled(
            format!("{} {count}", resolve_reaction_glyph(app, &r.emoji)),
            Style::default().fg(t.conv_unread),
        ));
    }
    Line::from(spans)
}

/// Contextual action bar shown under the selected message in select mode:
/// the actions available for *this* message, each with its key. Own
/// messages add edit/delete; attachments add download. Pure visual feedback
/// — the keys work directly regardless.
fn select_actions_line(m: &Message, app: &App, t: &crate::tui::theme::Theme) -> Line<'static> {
    let marks = app.msg_marks.len();
    // With a multi-selection active the action set collapses to copy / react
    // (plus mark / done); otherwise it's the full per-message menu.
    let actions: Vec<(&str, &str)> = if marks > 0 {
        vec![
            ("y", "copy all"),
            ("c", "content"),
            (":", "react"),
            ("Space", "±"),
            ("Esc", "done"),
        ]
    } else {
        let is_me = !app.identity.username.is_empty() && m.sender == app.identity.username;
        let is_attachment = matches!(m.content, MessageContent::Attachment(_));
        let mut actions: Vec<(&str, &str)> = vec![
            ("Space", "select"),
            ("y", "copy"),
            ("c", "content"),
            (":", "react"),
            ("r", "reply"),
        ];
        if is_me {
            actions.push(("e", "edit"));
            actions.push(("d", "delete"));
        }
        actions.push(("p", "pin"));
        if is_attachment {
            actions.push(("s", "download"));
        }
        actions
    };
    // Flush to the left of the section (aligned with the message's "→"
    // arrow), not the body indent — keeps the chat compact at half-width.
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    if marks > 0 {
        spans.push(Span::styled(
            format!("{marks} sel   "),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }
    for (i, (k, label)) in actions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            k.to_string(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(t.dim),
        ));
    }
    Line::from(spans)
}

/// Maps a stored reaction key (a `:shortcode:`) to its glyph via the emoji
/// catalogue, so the chat shows 🫡 rather than `:saluting_face:`. Falls back
/// to the key as-is (already a glyph, or an unknown/custom shortcode).
fn resolve_reaction_glyph(app: &App, key: &str) -> String {
    let alias = key.trim_matches(':');
    app.emojis
        .iter()
        .find(|e| e.alias == alias)
        .map(|e| e.display.clone())
        .unwrap_or_else(|| key.to_string())
}

fn content_icon(c: &MessageContent) -> &'static str {
    match c {
        MessageContent::Text(_) => "▸",
        MessageContent::Edit { .. } => "✎",
        MessageContent::Delete { .. } => "✗",
        MessageContent::Reaction { .. } => "♥",
        MessageContent::Attachment(_) => "📎",
        MessageContent::System(_) => "★",
        MessageContent::Metadata { .. } => "⚙",
        MessageContent::Headline { .. } => "❡",
        MessageContent::Pin { .. } => "📌",
        MessageContent::Join { .. } => "→",
        MessageContent::Leave { .. } => "←",
        MessageContent::SendPayment { .. } => "✦",
        MessageContent::RequestPayment { .. } => "✧",
        MessageContent::Unknown { .. } => "?",
        MessageContent::Empty => " ",
    }
}

fn body_lines(
    content: &MessageContent,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'static>> {
    match content {
        MessageContent::Text(body) => render_text_body(body, t, width),
        MessageContent::Edit { target_id, body } => {
            let label = format!("(edited msg #{target_id})");
            let mut lines = vec![Line::from(Span::styled(
                format!("    {label}"),
                Style::default().fg(t.dim),
            ))];
            lines.extend(render_text_body(body, t, width));
            lines
        }
        MessageContent::Delete { target_ids } => {
            let s = if target_ids.is_empty() {
                "(message deleted)".to_string()
            } else {
                let ids = target_ids
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("(deleted msg #{ids})")
            };
            placeholder(&s, t)
        }
        MessageContent::Reaction { target_id, body } => {
            placeholder(&format!("reacted {body} on msg #{target_id}"), t)
        }
        MessageContent::Attachment(att) => render_attachment(att, t),
        MessageContent::System(sys) => render_system(sys, t),
        MessageContent::Metadata { title } => {
            placeholder(&format!("channel title set to: {title}"), t)
        }
        MessageContent::Headline { headline } => {
            placeholder(&format!("channel headline: {headline}"), t)
        }
        MessageContent::Pin { target_id } => placeholder(&format!("pinned msg #{target_id}"), t),
        MessageContent::Join { joiner } => placeholder(&format!("{joiner} joined the channel"), t),
        MessageContent::Leave { leaver } => placeholder(&format!("{leaver} left the channel"), t),
        MessageContent::SendPayment { text } => stellar_lines("payment", text, t),
        MessageContent::RequestPayment { text } => stellar_lines("request", text, t),
        MessageContent::Unknown { type_name } => {
            placeholder(&format!("(unsupported message type: {type_name})"), t)
        }
        MessageContent::Empty => placeholder("(no content)", t),
    }
}

fn placeholder(s: &str, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        format!("    {s}"),
        Style::default().fg(t.dim),
    ))]
}

fn render_text_body(body: &str, t: &crate::tui::theme::Theme, width: usize) -> Vec<Line<'static>> {
    if body.is_empty() {
        return placeholder("(empty)", t);
    }
    // Wrap each line to the panel width (minus the 4-space body indent) so
    // long messages flow onto continuation lines instead of being cut off.
    let wrap_w = width.saturating_sub(4).max(8);
    let mut lines = Vec::new();
    for l in body.lines() {
        for piece in wrap_line(l, wrap_w) {
            lines.push(Line::from(Span::styled(
                format!("    {piece}"),
                Style::default().fg(t.foreground),
            )));
        }
    }
    lines
}

/// Word-wraps `line` to `width` columns (char-based; long words are
/// hard-split). Returns at least one piece.
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 || line.chars().count() <= width {
        return vec![line.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for word in line.split(' ') {
        let ww = word.chars().count();
        if cur_w > 0 && cur_w + 1 + ww > width {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if cur_w == 0 {
            if ww > width {
                // Hard-split a word longer than the whole width.
                let mut chunk = String::new();
                let mut cw = 0usize;
                for ch in word.chars() {
                    if cw == width {
                        out.push(std::mem::take(&mut chunk));
                        cw = 0;
                    }
                    chunk.push(ch);
                    cw += 1;
                }
                cur = chunk;
                cur_w = cw;
            } else {
                cur = word.to_string();
                cur_w = ww;
            }
        } else {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + ww;
        }
    }
    out.push(cur);
    out
}

fn render_attachment(att: &AttachmentInfo, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
    let size = format_size(att.size);
    let mime = if att.mime_type.is_empty() {
        "—".to_string()
    } else {
        att.mime_type.clone()
    };
    let state = if att.uploaded { "" } else { " (uploading…)" };
    let mut lines = Vec::with_capacity(3);
    if !att.title.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("    {}", att.title),
            Style::default().fg(t.foreground),
        )));
    }
    // Filename + size share one line (the size fits in the space next to the
    // usually-short name); the longer MIME type drops to a dim line below.
    lines.push(Line::from(vec![
        Span::styled(
            format!("    {}", att.filename),
            Style::default()
                .fg(t.conv_team)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {size}{state}"), Style::default().fg(t.dim)),
    ]));
    lines.push(Line::from(Span::styled(
        format!("      {mime}"),
        Style::default().fg(t.dim),
    )));
    lines
}

fn stellar_lines(kind: &str, text: &str, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled("    [", Style::default().fg(t.dim)),
        Span::styled(
            format!("stellar {kind}"),
            Style::default().fg(t.conv_unread),
        ),
        Span::styled("] ", Style::default().fg(t.dim)),
        Span::styled(text.to_string(), Style::default().fg(t.foreground)),
    ])]
}

fn render_system(sys: &SystemInfo, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
    let tag = sys.kind.label();
    vec![Line::from(vec![
        Span::styled("    [", Style::default().fg(t.dim)),
        Span::styled(tag, Style::default().fg(t.conv_team)),
        Span::styled("] ", Style::default().fg(t.dim)),
        Span::styled(sys.description.clone(), Style::default().fg(t.foreground)),
    ])]
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_buckets() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn icon_for_each_variant_is_stable() {
        let variants = [
            MessageContent::Text("x".into()),
            MessageContent::Edit {
                target_id: 1,
                body: "y".into(),
            },
            MessageContent::Delete {
                target_ids: vec![1],
            },
            MessageContent::Reaction {
                target_id: 1,
                body: ":+1:".into(),
            },
            MessageContent::Attachment(AttachmentInfo::default()),
            MessageContent::System(SystemInfo::default()),
            MessageContent::Metadata { title: "g".into() },
            MessageContent::Headline {
                headline: "h".into(),
            },
            MessageContent::Pin { target_id: 1 },
            MessageContent::Join { joiner: "a".into() },
            MessageContent::Leave { leaver: "a".into() },
            MessageContent::SendPayment {
                text: "1 XLM".into(),
            },
            MessageContent::RequestPayment {
                text: "1 XLM".into(),
            },
            MessageContent::Unknown {
                type_name: "x".into(),
            },
            MessageContent::Empty,
        ];
        for v in variants {
            assert!(!content_icon(&v).is_empty());
        }
    }
}
