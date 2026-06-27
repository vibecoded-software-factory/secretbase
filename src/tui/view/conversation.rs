//! Single-conversation detail view — identity bar, conversation header,
//! scrollable message history, the compose pane and the status strip.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::domain::{AttachmentInfo, Message, MessageContent, SystemInfo};
use crate::tui::app::App;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{
    draw_identity_bar, draw_status_strip, editor_spans, identity_content_rows,
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let id_rows = identity_content_rows(app, area.width);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(id_rows + 2), // identity bar
            Constraint::Length(3),           // conversation header
            Constraint::Min(3),              // messages
            Constraint::Length(3),           // compose pane
            Constraint::Length(1),           // status strip
        ])
        .split(area);

    draw_identity_bar(frame, app, layout[0]);
    render_header(frame, app, layout[1]);
    render_messages(frame, app, layout[2]);
    render_compose(frame, app, layout[3]);

    let hint = if app.selected_msg_idx.is_some() {
        "↑/↓ select · e edit · d delete · : react · p pin · Esc back"
    } else if app.edit_target_id.is_some() {
        "Enter save edit · Esc cancel"
    } else {
        "Enter send · ↑/↓ scroll · Alt+V select · Esc back"
    };
    draw_status_strip(frame, app, layout[4], hint);
}

fn render_compose(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let editing = app.edit_target_id.is_some();
    let replying = app.reply_to_id;

    let (title, _border_focus) = if let Some(id) = app.edit_target_id {
        (format!("Editing msg #{id}"), true)
    } else if let Some(id) = replying {
        (format!("Replying to #{id}"), true)
    } else {
        ("Compose".to_string(), true)
    };
    let placeholder = if editing {
        "edit text below — Enter saves, Esc cancels"
    } else if replying.is_some() {
        "type your reply — Enter sends, Esc cancels"
    } else {
        "type and press Enter to send…"
    };
    let line = if app.compose.is_empty() {
        Line::from(Span::styled(
            placeholder,
            Style::default().fg(t.placeholder),
        ))
    } else {
        Line::from(editor_spans(&app.compose, true, t))
    };
    frame.render_widget(
        Paragraph::new(line).block(titled_block(&title, true, app)),
        area,
    );
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let label = app
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
        .unwrap_or_else(|| "(no conversation)".to_string());

    let mut spans = vec![Span::styled(
        label,
        Style::default()
            .fg(t.foreground)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(target_id) = app.pinned_msg_id {
        let snippet = app
            .messages
            .iter()
            .find(|m| m.id == target_id)
            .and_then(|m| match &m.content {
                MessageContent::Text(b) => Some(b.lines().next().unwrap_or("").to_string()),
                MessageContent::Edit { body, .. } => {
                    Some(body.lines().next().unwrap_or("").to_string())
                }
                _ => None,
            })
            .unwrap_or_default();
        let banner = if snippet.is_empty() {
            format!("  📌 pinned #{target_id}")
        } else {
            let s: String = snippet.chars().take(40).collect();
            format!("  📌 #{target_id} {s}")
        };
        spans.push(Span::styled(banner, Style::default().fg(t.conv_unread)));
    }

    let title = format!("Conversation · {} msgs", app.messages.len());
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(titled_block(&title, true, app)),
        area,
    );
}

fn render_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme.clone();
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Optimistic outbox bubbles for this conversation (sending / failed),
    // rendered below the loaded history.
    let outbox = outbox_lines(app, now_s, &t);

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
        frame.render_widget(
            Paragraph::new(lines).block(titled_block("Messages", false, app)),
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
    for (idx, m) in app.messages.iter().enumerate() {
        let is_selected = app.selected_msg_idx == Some(idx);
        if is_selected {
            selected_line = Some(lines.len());
        }
        let mut block = message_lines(m, now_s, app, &t);
        if is_selected {
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
        lines.push(Line::from(Span::raw("")));
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

    let counter = if app.messages_loading_older {
        format!("{} msgs — loading older…", app.messages.len())
    } else if max_back == 0 {
        format!("{} msgs", app.messages.len())
    } else if effective_back == 0 {
        format!("{} msgs — bottom", app.messages.len())
    } else if app.messages_next.is_some() && effective_back == max_back {
        format!("{} msgs — top (more above)", app.messages.len())
    } else {
        format!("{} msgs — ↑{}", app.messages.len(), effective_back)
    };

    let title = format!("Messages · {counter}");
    // Ratatui scroll is u16; saturate so a very long history can't wrap
    // the offset to a tiny value via a truncating cast.
    let scroll_u16 = scroll_y.min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_u16, 0))
            .block(titled_block(&title, false, app)),
        area,
    );

    app.messages_max_back = max_back;
}

/// Builds the bubbles for the optimistic outbox entries targeting the open
/// conversation: `○ sending…`, a delivered `→` (transient, pruned by the
/// reconciling re-read), and a red `✗ failed · Alt+R to resend`.
fn outbox_lines(app: &App, now_s: u64, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
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
                    relative_time(p.sent_at_ms / 1000, now_s),
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
        lines.extend(body_lines(&MessageContent::Text(p.body.clone()), t));
        lines.push(Line::from(Span::raw("")));
    }
    lines
}

fn message_lines(
    m: &Message,
    now_s: u64,
    app: &App,
    t: &crate::tui::theme::Theme,
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
    let when = relative_time(m.sent_at, now_s);
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
    lines.extend(body_lines(&m.content, t));
    if !m.reactions.is_empty() {
        lines.push(reactions_line(&m.reactions, t));
    }
    lines
}

fn reactions_line(
    reactions: &[crate::domain::Reaction],
    t: &crate::tui::theme::Theme,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("    ")];
    for (i, r) in reactions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let count = r.usernames.len();
        spans.push(Span::styled(
            format!("[{} {count}]", r.emoji),
            Style::default().fg(t.conv_unread),
        ));
    }
    Line::from(spans)
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

fn body_lines(content: &MessageContent, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
    match content {
        MessageContent::Text(body) => render_text_body(body, t),
        MessageContent::Edit { target_id, body } => {
            let label = format!("(edited msg #{target_id})");
            let mut lines = vec![Line::from(Span::styled(
                format!("    {label}"),
                Style::default().fg(t.dim),
            ))];
            lines.extend(render_text_body(body, t));
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

fn render_text_body(body: &str, t: &crate::tui::theme::Theme) -> Vec<Line<'static>> {
    if body.is_empty() {
        return placeholder("(empty)", t);
    }
    body.lines()
        .map(|l| {
            Line::from(Span::styled(
                format!("    {l}"),
                Style::default().fg(t.foreground),
            ))
        })
        .collect()
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
    lines.push(Line::from(vec![
        Span::styled("    ", Style::default()),
        Span::styled(
            att.filename.clone(),
            Style::default()
                .fg(t.conv_team)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {size}  {mime}{state}"),
            Style::default().fg(t.dim),
        ),
    ]));
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

fn relative_time(sent_at_s: u64, now_s: u64) -> String {
    if sent_at_s == 0 || now_s == 0 || sent_at_s > now_s {
        return String::new();
    }
    let d = now_s - sent_at_s;
    if d < 60 {
        "now".to_string()
    } else if d < 60 * 60 {
        format!("{}m ago", d / 60)
    } else if d < 24 * 60 * 60 {
        format!("{}h ago", d / 3600)
    } else if d < 30 * 24 * 60 * 60 {
        format!("{}d ago", d / 86400)
    } else if d < 365 * 24 * 60 * 60 {
        format!("{}mo ago", d / (30 * 86400))
    } else {
        format!("{}y ago", d / (365 * 86400))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_now() {
        assert_eq!(relative_time(100, 130), "now");
    }

    #[test]
    fn relative_minutes() {
        assert_eq!(relative_time(0, 600), "");
        assert_eq!(relative_time(60, 60 + 60 * 5), "5m ago");
    }

    #[test]
    fn relative_hours() {
        assert_eq!(relative_time(0, 7200), "");
        assert_eq!(relative_time(1, 1 + 3600 * 3), "3h ago");
    }

    #[test]
    fn relative_days() {
        assert_eq!(relative_time(1, 1 + 86400 * 2), "2d ago");
    }

    #[test]
    fn relative_future_returns_empty() {
        assert_eq!(relative_time(200, 100), "");
    }

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
