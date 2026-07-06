//! The **dynamic island** — a persistent 3-row band atop the right pane
//! (all sections), one bordered pill whose title and content morph with
//! context, highest-priority first:
//!
//! 1. **Live activity** — the running / done / error action feedback (the
//!    transient toast, moved here from the status strip).
//! 2. **Pinned message** — when the open conversation has a pin.
//! 3. **Topic** — the open conversation's headline, if any.
//! 4. **Attention** — unread / mention counts when idle (the home of the
//!    old status-strip `●N` badge).
//! 5. **Context** — per-section detail (teams / find counts), else a calm
//!    idle line so the band never collapses (option A: fixed height).

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use ratatui::layout::Rect;

use crate::domain::{MemberStatus, MessageContent};
use crate::tui::app::App;
use crate::tui::screens::Screen;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::trim_end_ellipsis;

const SPINNER: &[&str] = &["⠋", "⠙", "⠸", "⠴"];

/// Renders the dynamic island into `area` (a fixed 3-row band). Display-only.
pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let (title, spans) = content(app, area.width.saturating_sub(2) as usize);
    let block = titled_block(&title, false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

/// Picks the island's title + content line by priority.
fn content(app: &App, w: usize) -> (String, Vec<Span<'static>>) {
    let t = &app.theme;
    // A generous left inset so nothing hugs the border — the island reads as a
    // roomy pill, not a cramped label.
    let lead = || Span::raw("  ");

    // 1. Live action feedback (the transient toast).
    if let Some((title, span)) = activity(app) {
        return (title, vec![lead(), span]);
    }

    // 2. Pinned message (only meaningful inside an open conversation).
    if app.pins.present && app.open_conv_id.is_some() {
        return ("📌 Pinned".to_string(), pin_spans(app, w));
    }

    // 3. Conversation topic / headline.
    if let Some(topic) = app.thread.conv_headline.as_deref() {
        let topic = trim_end_ellipsis(
            topic.lines().next().unwrap_or(""),
            w.saturating_sub(4).max(8),
        );
        return (
            "~ Topic".to_string(),
            vec![
                lead(),
                Span::styled(
                    format!("\u{201c}{topic}\u{201d}"),
                    Style::default().fg(t.foreground),
                ),
            ],
        );
    }

    // 4. Attention — unread / mention counts (idle).
    let mention_n = app.mentioned.len();
    let unread_n = app
        .conversations
        .iter()
        .filter(|c| c.member_status == MemberStatus::Active)
        .filter(|c| app.conv_is_unread(c))
        .count();
    if unread_n > 0 || mention_n > 0 {
        let bold_unread = Style::default()
            .fg(t.conv_unread)
            .add_modifier(Modifier::BOLD);
        let mut spans = vec![lead()];
        if unread_n > 0 {
            // Dot and count breathe — "● 1 unread", not "●1".
            spans.push(Span::styled("● ", bold_unread));
            spans.push(Span::styled(format!("{unread_n} unread"), bold_unread));
        }
        if mention_n > 0 {
            if unread_n > 0 {
                spans.push(Span::styled("   ·   ", Style::default().fg(t.muted)));
            }
            spans.push(Span::styled(
                format!("@{mention_n} mentions"),
                t.danger_title(),
            ));
        }
        spans.push(Span::styled(
            "      Ctrl+N to jump",
            Style::default().fg(t.muted),
        ));
        return ("◉ Attention".to_string(), spans);
    }

    // 5. Per-section context, else a calm idle line.
    context(app)
}

/// The running / done / error toast, if any — the island's top priority.
fn activity(app: &App) -> Option<(String, Span<'static>)> {
    use crate::tui::action::ActionState;
    let t = &app.theme;
    match &app.action_state {
        ActionState::Idle => None,
        ActionState::Running(msg) => {
            let spin = SPINNER[app.action_tick as usize % SPINNER.len()];
            Some((
                format!("{spin} Activity"),
                Span::styled(msg.clone(), Style::default().fg(t.accent)),
            ))
        }
        ActionState::Done(msg) => Some((
            "✓ Done".to_string(),
            Span::styled(msg.clone(), Style::default().fg(t.success)),
        )),
        ActionState::Error(msg) => Some((
            "✗ Error".to_string(),
            Span::styled(msg.clone(), Style::default().fg(t.error)),
        )),
    }
}

/// The pinned-message content line (resolved from the loaded window or the
/// background-fetched body cache), ported from the old adaptive header.
fn pin_spans(app: &App, w: usize) -> Vec<Span<'static>> {
    let t = &app.theme;
    let mut s = vec![Span::raw("  ")];
    let resolved = app.pins.msg_id.and_then(|pid| {
        app.thread
            .msg_index
            .get(&pid)
            .copied()
            .and_then(|i| app.thread.messages.get(i))
            .or_else(|| {
                app.open_conv_id
                    .as_ref()
                    .and_then(|c| app.pins.bodies.get(c))
                    .filter(|m| m.id == pid)
            })
    });
    match resolved {
        Some(m) => {
            let body = match &m.content {
                MessageContent::Text(b) => b.clone(),
                MessageContent::Edit { body, .. } => body.clone(),
                MessageContent::Attachment(a) => format!("[{}]", a.filename),
                _ => String::new(),
            };
            let suffix_w = 1 + " · ".chars().count() + 2; // the “” quotes
            let budget = w.saturating_sub(m.sender.chars().count() + suffix_w).max(8);
            let snippet = trim_end_ellipsis(body.lines().next().unwrap_or(""), budget);
            s.push(Span::styled(
                m.sender.clone(),
                Style::default()
                    .fg(t.user_color(&m.sender))
                    .add_modifier(Modifier::BOLD),
            ));
            s.push(Span::styled(" · ", Style::default().fg(t.muted)));
            s.push(Span::styled(
                format!("\u{201c}{snippet}\u{201d}"),
                Style::default().fg(t.foreground),
            ));
        }
        None => {
            let who = app.pins.sender.clone().unwrap_or_default();
            let label = match app.pins.msg_id {
                Some(pid) => format!("#{pid}"),
                None if !who.is_empty() => format!("{who} pinned a message"),
                None => "a message is pinned".to_string(),
            };
            s.push(Span::styled(label, Style::default().fg(t.dim)));
        }
    }
    s
}

/// Per-section idle context — teams / find counts, or a calm brand line.
fn context(app: &App) -> (String, Vec<Span<'static>>) {
    let t = &app.theme;
    let dim = |s: String| vec![Span::raw("  "), Span::styled(s, Style::default().fg(t.dim))];
    match app.screen {
        Screen::Teams => (
            " Teams".to_string(),
            dim(format!("{} teams", app.teams.list.len())),
        ),
        Screen::ChannelBrowser => {
            let team = app.channel_browser.team.clone().unwrap_or_default();
            (
                " Channels".to_string(),
                dim(format!(
                    "#{team} · {} channels",
                    app.channel_browser.channels.len()
                )),
            )
        }
        _ if app.open_conv_id.is_none() => (
            " Find".to_string(),
            dim(format!(
                "{} of {} chats",
                app.filtered_cache.len(),
                app.conversations.len()
            )),
        ),
        // An open conversation with nothing else to surface: a calm resting
        // state (the band stays a fixed height — option A).
        _ => (
            " ·".to_string(),
            vec![Span::styled("  secretbase", Style::default().fg(t.muted))],
        ),
    }
}
