//! Single-conversation detail view — identity bar, conversation header,
//! scrollable message history, the compose pane and the status strip.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::domain::{AttachmentInfo, Message, MessageContent, SystemInfo, message_time};
use crate::tui::app::App;
use crate::tui::screens::Focus;
use crate::tui::view::titled_block;
use crate::tui::view::widgets::{draw_search_box, editor_lines, trim_end_ellipsis};

/// Renders the chat's in-conversation **search** box (`searchregexp`, Ctrl+F)
/// into `area`. The conversation name now lives on the Messages panel title
/// ([`chat_title`]), so the header is just this search box.
pub(crate) fn draw_chat_header(frame: &mut Frame, app: &App, area: Rect) {
    draw_search_box(
        frame,
        app,
        area,
        "Ctrl+F",
        "Search",
        "search this chat",
        &app.conv_search,
        app.focus == Focus::ChatSearch,
        // Unreachable when no conversation is open (Ctrl+F / the Tab stop are
        // gated) — render it disabled (muted) so it reads as unavailable.
        app.open_conv_id.is_none(),
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
    if app.mention_popup_active() {
        draw_mention_popup(frame, app, layout[1]);
    }
}

/// The `@`-mention autocomplete popup, floated just above the compose box.
fn draw_mention_popup(frame: &mut Frame, app: &App, compose_area: Rect) {
    let t = &app.theme;
    let matches = app.mention_matches();
    if matches.is_empty() {
        return;
    }
    let sel = app.mention_selected.min(matches.len() - 1);
    let h = (matches.len() as u16 + 2).min(8);
    let longest = matches.iter().map(|m| m.chars().count()).max().unwrap_or(8) as u16;
    let w = (longest + 6).clamp(16, compose_area.width.max(16));
    let rect = Rect {
        x: compose_area.x,
        y: compose_area.y.saturating_sub(h),
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    let block = titled_block("@mention", true, app);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let rows = inner.height as usize;
    let lines: Vec<Line<'static>> = matches
        .iter()
        .enumerate()
        .take(rows)
        .map(|(i, m)| {
            let selected = i == sel;
            let prefix = if selected { "▶ " } else { "  " };
            let style = if selected {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
                    .bg(t.selected_bg)
            } else {
                Style::default().fg(t.foreground)
            };
            Line::from(Span::styled(format!("{prefix}@{m}"), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The status-strip hint for the chat, by interaction mode.
pub(crate) fn chat_hint(app: &App) -> &'static str {
    if app.conv_search_active {
        "type · Enter search/jump · ↑/↓ pick · Esc close"
    } else if app.selected_msg_idx.is_some() {
        "↑/↓ move · Space mark · e edit · Shift+X del · + react · y copy · Esc back"
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
        let snippet = trim_end_ellipsis(hit.body_summary.lines().next().unwrap_or(""), max_w);
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
    // Absolute (line, rows, msg_id, cache path) reservations for inline images.
    let mut img_reservations: Vec<(usize, u16, u64, String)> = Vec::new();
    // Thumbnail width (indent 4 + a right margin so it isn't glued to the
    // border; capped on wide panels). Symbol images are pre-rendered to lines
    // here so each reserves exactly its real height.
    let img_w = body_width.saturating_sub(6).clamp(10, 72) as u16;
    // Recomputed each frame; `symbol_image_lines` / `image_render_path` set it
    // true when an animated GIF is on screen.
    app.gif_animating = false;
    let symbol_imgs = symbol_image_lines(app, img_w);
    for (idx, m) in app.messages.iter().enumerate() {
        let start = lines.len();
        let is_selected = app.selected_msg_idx == Some(idx);
        // Marked messages (multi-select for copy) get the same shading as the
        // cursor — the cursor is told apart by its action bar below.
        let is_marked = app.msg_marks.contains(&idx);
        if is_selected {
            selected_line = Some(start);
        }
        let mut local_img: Vec<ImgReservation> = Vec::new();
        let mut block = message_lines(m, now_s, app, &t, body_width, &mut local_img, &symbol_imgs);
        // Block-relative row ranges occupied by image thumbnails — these stay
        // unshaded so the selection background doesn't paint over the graphic.
        let img_ranges: Vec<(usize, usize)> = local_img
            .iter()
            .map(|r| (r.block_offset, r.block_offset + r.rows as usize))
            .collect();
        for r in local_img.drain(..) {
            img_reservations.push((start + r.block_offset, r.rows, r.msg_id, r.path));
        }
        if is_selected || is_marked {
            let bg = t.selected_bg;
            for (li, line) in block.iter_mut().enumerate() {
                if img_ranges.iter().any(|&(s, e)| li >= s && li < e) {
                    continue;
                }
                line.spans = line
                    .spans
                    .iter()
                    .map(|s| Span::styled(s.content.clone(), s.style.bg(bg)))
                    .collect();
            }
        }
        lines.extend(block);
        // In select mode, show a contextual action bar under the highlighted
        // message — visual feedback for what can be done with it (the keys
        // still work directly).
        if is_selected && app.selected_msg_idx.is_some() {
            lines.extend(select_actions_lines(m, app, &t, body_width));
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

    // Inline images. Symbols are already rendered in-buffer (pushed as lines by
    // `message_lines`), so only `img_res` for graphics / still-downloading
    // images remains: queue a fetch when not ready, else (true graphics)
    // record a paint rect for the run loop — but only when the whole thumbnail
    // fits the viewport so it can't overflow the panel.
    app.image_areas.clear();
    app.image_to_fetch.clear();
    app.gif_to_decode.clear();
    let graphics = matches!(app.image_proto, Some(p) if p != crate::tui::image::ImgProto::Symbols);
    let img_x = area.x + 1 + 4;
    for (abs, rows, msg_id, path) in img_reservations {
        match img_state(app, &path) {
            // Still downloading → queue the fetch (skeleton shown meanwhile).
            ImgState::Loading => {
                if !app.image_pending.contains(&path) && !app.image_failed.contains(&path) {
                    app.image_to_fetch.push((msg_id, path));
                }
            }
            // Downloaded GIF, not decoded yet → queue the off-thread decode.
            ImgState::Decoding => {
                if !app.gif_pending.contains(&path) {
                    app.gif_to_decode.push(path);
                }
            }
            // Ready → graphics protocols paint the current frame / the file.
            ImgState::Ready => {
                if graphics && abs >= scroll_y && (abs - scroll_y) + rows as usize <= viewport {
                    let rect = Rect {
                        x: img_x,
                        y: area.y + 1 + (abs - scroll_y) as u16,
                        width: img_w,
                        height: rows,
                    };
                    let render_path = image_render_path(app, &path);
                    app.image_areas.push((rect, render_path));
                }
            }
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
        lines.extend(body_lines(
            &MessageContent::Text(p.body.clone()),
            t,
            width,
            &[],
        ));
        lines.push(Line::from(Span::raw("")));
    }
    lines
}

/// A block-relative reservation of blank rows for an inline image, recorded by
/// [`message_lines`] and mapped to a screen rect (after scroll) by the caller.
struct ImgReservation {
    /// Index of the first graphic row within the message's block.
    block_offset: usize,
    /// Number of rows reserved for the image.
    rows: u16,
    /// Message id the image belongs to (to enqueue its download).
    msg_id: u64,
    /// On-disk cache path for the image.
    path: String,
}

/// Max height (rows) for an inline image thumbnail. The actual height follows
/// the image's aspect (chafa doesn't pad), so wide images use fewer rows.
const IMAGE_ROWS: u16 = 24;

/// Braille spinner frame for wall-clock `ms` (≈11 fps) — appended to the image
/// loading / decoding skeleton label so it reads as live, not stuck.
fn spinner_frame_ms(ms: u64) -> &'static str {
    const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    SPIN[((ms / 90) as usize) % SPIN.len()]
}

/// Render state of an inline image — decides skeleton vs. paint.
enum ImgState {
    /// Still downloading.
    Loading,
    /// Downloaded, but its GIF frames are being decoded off-thread.
    Decoding,
    /// Ready to paint (a non-GIF, or a GIF already decoded).
    Ready,
}

/// Classifies an image cache path. A `.gif` is `Decoding` until the worker has
/// populated `gif_anims` for it (the decode runs off the render thread), so the
/// view shows a skeleton instead of blocking on ImageMagick.
fn img_state(app: &App, path: &str) -> ImgState {
    if !app.image_ready.contains(path) {
        return ImgState::Loading;
    }
    if path.to_ascii_lowercase().ends_with(".gif") && !app.gif_anims.contains_key(path) {
        return ImgState::Decoding;
    }
    ImgState::Ready
}

/// An **image placeholder** filling an image's reserved footprint while it
/// loads / decodes: a dim picture *frame* (rounded box) with the thumbnail
/// dimensions, a `⌖` corner mark, and a centered label — reads as "a picture
/// goes here", not a chat-style bar skeleton. Produces exactly `rows` lines,
/// aligned to the column the real thumbnail will paint into.
fn image_skeleton_lines(
    t: &crate::tui::theme::Theme,
    label: &str,
    width: u16,
    rows: u16,
) -> Vec<Line<'static>> {
    let frame = Style::default().fg(t.inactive);
    let text = Style::default().fg(t.dim);
    // 4-space indent: the same column the painted thumbnail uses.
    let pad = || Span::raw("    ");
    let w = (width as usize).clamp(6, 72);
    let inner = w - 2; // span between the two side borders
    let rows = rows.max(2) as usize;

    // A line of interior: side borders around `mid` content spans (centered).
    let interior = |mid: Vec<Span<'static>>| -> Line<'static> {
        let used: usize = mid.iter().map(|s| s.content.chars().count()).sum();
        let slack = inner.saturating_sub(used);
        let left = slack / 2;
        let right = slack - left;
        let mut spans = vec![pad(), Span::styled("│", frame), Span::raw(" ".repeat(left))];
        spans.extend(mid);
        spans.push(Span::raw(" ".repeat(right)));
        spans.push(Span::styled("│", frame));
        Line::from(spans)
    };

    let mut out: Vec<Line<'static>> = Vec::with_capacity(rows);
    // Top border carries the thumbnail size, like a real frame's caption.
    let dims = format!(" {}×{} ", width, rows);
    let top = if dims.chars().count() + 2 <= inner {
        let rest = inner - dims.chars().count();
        format!("╭{}{}╮", dims, "─".repeat(rest))
    } else {
        format!("╭{}╮", "─".repeat(inner))
    };
    out.push(Line::from(vec![pad(), Span::styled(top, frame)]));

    let body = rows - 2;
    let mid_row = body / 2;
    for i in 0..body {
        if i == mid_row {
            out.push(interior(vec![
                Span::styled("⌖ ", frame),
                Span::styled(label.to_string(), text),
            ]));
        } else {
            out.push(interior(vec![]));
        }
    }
    out.push(Line::from(vec![
        pad(),
        Span::styled(format!("╰{}╯", "─".repeat(inner)), frame),
    ]));
    out
}

/// The path to actually paint for a **ready** image: the current animation
/// frame for an animated GIF (decoded off-thread into `gif_anims`), or the file
/// itself for a still image. Sets `gif_animating` when a GIF is animating.
///
/// Only called once the image is [`ImgState::Ready`] — the expensive GIF decode
/// runs on the worker ([`crate::tui::flows::chat::handle_decode_gif_response`]),
/// never here, so this never blocks the render thread.
fn image_render_path(app: &mut App, path: &str) -> String {
    if let Some(Some(g)) = app.gif_anims.get(path) {
        app.gif_animating = true;
        return g.frame_at(app.anim_ms).to_string();
    }
    path.to_string()
}

/// Pre-renders every **ready** image attachment in the open conversation to
/// chafa symbol lines (cached), keyed by message id, trimmed to the image's
/// real height. Only for the `symbols` protocol — those render in-buffer, so we
/// need the exact line count to reserve precisely (no blank gap below).
fn symbol_image_lines(
    app: &mut App,
    img_w: u16,
) -> std::collections::HashMap<u64, Vec<Line<'static>>> {
    let mut map = std::collections::HashMap::new();
    let Some(proto) = app.image_proto else {
        return map;
    };
    if proto != crate::tui::image::ImgProto::Symbols {
        return map;
    }
    let conv_id = app.open_conv_id.clone().unwrap_or_default();
    let items: Vec<(u64, String)> = app
        .messages
        .iter()
        .filter_map(|m| match &m.content {
            MessageContent::Attachment(a)
                if crate::tui::image::is_image(&a.mime_type, &a.filename) =>
            {
                let path = crate::tui::flows::chat::image_path_for(&conv_id, m.id, &a.filename);
                // Only render once Ready — a GIF still decoding shows a skeleton
                // (and the reservation loop queues its decode).
                matches!(img_state(app, &path), ImgState::Ready).then_some((m.id, path))
            }
            _ => None,
        })
        .collect();
    for (id, path) in items {
        let render_path = image_render_path(app, &path);
        // The cache runs chafa + ANSI-parse once per (frame,size); steady-state
        // GIF animation is then a pure cache hit — no per-tick subprocess.
        let lines = app
            .image_render_cache
            .symbol_lines(proto, &render_path, img_w, IMAGE_ROWS, 4);
        if !lines.is_empty() {
            map.insert(id, lines);
        }
    }
    map
}

fn message_lines(
    m: &Message,
    now_s: u64,
    app: &App,
    t: &crate::tui::theme::Theme,
    width: usize,
    img_res: &mut Vec<ImgReservation>,
    symbol_imgs: &std::collections::HashMap<u64, Vec<Line<'static>>>,
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
    if m.edited {
        header_spans.push(Span::styled(
            " (edited)",
            Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
        ));
    }
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
        lines.push(reply_quote_line(target, app, t, width));
    }
    // Inline image attachment: the filename above, then reserved rows the run
    // loop paints the thumbnail into (kitty / sixel / chafa). Falls back to the
    // normal attachment text when images are off or the download failed.
    let mut rendered_image = false;
    if let MessageContent::Attachment(att) = &m.content
        && app.image_proto.is_some()
        && crate::tui::image::is_image(&att.mime_type, &att.filename)
    {
        let conv_id = app.open_conv_id.as_deref().unwrap_or("");
        let path = crate::tui::flows::chat::image_path_for(conv_id, m.id, &att.filename);
        // A failed download drops through to the normal attachment text.
        if !app.image_failed.contains(&path) {
            // Name above the photo, size + type on a dim line below.
            lines.push(Line::from(Span::styled(
                format!("    🖼 {}", att.filename),
                Style::default()
                    .fg(t.conv_team)
                    .add_modifier(Modifier::BOLD),
            )));
            let mime = if att.mime_type.is_empty() {
                "image".to_string()
            } else {
                att.mime_type.clone()
            };
            lines.push(Line::from(Span::styled(
                format!("      {} · {mime}", format_size(att.size)),
                Style::default().fg(t.dim),
            )));
            if let Some(sym) = symbol_imgs.get(&m.id) {
                // Symbols, ready: the pre-rendered lines *are* the image — push
                // them directly at their real height (no reserved blank gap).
                // Recorded in `img_res` only so the selection shading skips
                // them; the mapping loop ignores ready symbol entries.
                let block_offset = lines.len();
                lines.extend(sym.iter().cloned());
                img_res.push(ImgReservation {
                    block_offset,
                    rows: sym.len() as u16,
                    msg_id: m.id,
                    path,
                });
            } else {
                // Graphics protocols are painted by the run loop once Ready; a
                // skeleton fills the reserved rows while it downloads / decodes.
                let block_offset = lines.len();
                let thumb_w = (width.saturating_sub(6)).clamp(10, 72) as u16;
                let spin = spinner_frame_ms(app.anim_ms);
                match img_state(app, &path) {
                    ImgState::Loading => lines.extend(image_skeleton_lines(
                        t,
                        &format!("loading image… {spin}"),
                        thumb_w,
                        IMAGE_ROWS,
                    )),
                    ImgState::Decoding => lines.extend(image_skeleton_lines(
                        t,
                        &format!("decoding GIF… {spin}"),
                        thumb_w,
                        IMAGE_ROWS,
                    )),
                    ImgState::Ready => {
                        for _ in 0..IMAGE_ROWS {
                            lines.push(Line::from(Span::raw("")));
                        }
                    }
                }
                img_res.push(ImgReservation {
                    block_offset,
                    rows: IMAGE_ROWS,
                    msg_id: m.id,
                    path,
                });
            }
            rendered_image = true;
        }
    }
    if !rendered_image {
        lines.extend(body_lines(&m.content, t, width, &m.mentions));
    }
    if !m.reactions.is_empty() {
        lines.extend(reaction_lines(&m.reactions, app, t, width));
    }
    lines
}

/// A dim, italic quote of the message a reply targets (`↩ sender · snippet`),
/// rendered just above the reply's own body. Falls back to `↩ #id` when the
/// parent isn't in the loaded history.
fn reply_quote_line(
    target: u64,
    app: &App,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Line<'static> {
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
            let snippet = body.lines().next().unwrap_or("");
            if snippet.is_empty() {
                format!("{} · …", m.sender)
            } else {
                format!("{} · {snippet}", m.sender)
            }
        })
        // Parent not in the loaded window — no raw message id, just an ellipsis.
        .unwrap_or_else(|| "…".to_string());
    // A quote is a one-line preview: show it whole if it fits, else trim to
    // the panel width with a trailing `…` (adapts to the screen, not a fixed
    // character count).
    let full = format!("   ↩ {label}");
    let line = trim_end_ellipsis(&full, width.max(8));
    Line::from(Span::styled(
        line,
        Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
    ))
}

/// Reaction chips (`{glyph} {count}`) collapsed under a message, **wrapping**
/// Discord-style: fill a row left-to-right, then continue on a new row below —
/// never truncated with a `…`. Returns one [`Line`] per row.
fn reaction_lines(
    reactions: &[crate::domain::Reaction],
    app: &App,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'static>> {
    const INDENT: usize = 4;
    const SEP: usize = 2; // spaces between chips
    let style = Style::default().fg(t.conv_unread);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(INDENT))];
    let mut used = INDENT;
    let mut on_row = 0usize;
    for r in reactions {
        let glyph = reaction_display(app, &r.emoji);
        let piece = format!("{glyph} {}", r.usernames.len());
        // +1: emoji glyphs often render two columns wide.
        let piece_w = piece.chars().count() + 1;
        let sep = if on_row == 0 { 0 } else { SEP };
        // Wrap to a new row when this chip would overflow (but never wrap an
        // empty row — a single over-wide chip just overflows).
        if on_row > 0 && used + sep + piece_w > width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans = vec![Span::raw(" ".repeat(INDENT))];
            used = INDENT;
            on_row = 0;
        }
        if on_row > 0 {
            spans.push(Span::raw(" ".repeat(SEP)));
            used += SEP;
        }
        spans.push(Span::styled(piece, style));
        used += piece_w;
        on_row += 1;
    }
    if on_row > 0 {
        lines.push(Line::from(spans));
    }
    lines
}

/// Contextual action bar shown under the selected message in select mode:
/// the actions available for *this* message, each with its key. Own
/// messages add edit/delete; attachments add download. Pure visual feedback
/// — the keys work directly regardless.
fn select_actions_lines(
    m: &Message,
    app: &App,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let marks = app.msg_marks.len();
    // With a multi-selection active the action set collapses to copy / react
    // (plus mark / done); otherwise it's the full per-message menu.
    let actions: Vec<(&str, &str)> = if marks > 0 {
        vec![
            ("y", "copy all"),
            ("c", "content"),
            ("+", "react"),
            ("Space", "±"),
            ("Esc", "done"),
        ]
    } else {
        let is_me = !app.identity.username.is_empty() && m.sender == app.identity.username;
        let is_attachment = matches!(m.content, MessageContent::Attachment(_));
        let is_image_att = matches!(&m.content, MessageContent::Attachment(a)
            if crate::tui::image::is_image(&a.mime_type, &a.filename));
        // For an image, `c` copies the picture itself, not the caption text.
        let copy_label = if is_image_att {
            "copy image"
        } else {
            "content"
        };
        let mut actions: Vec<(&str, &str)> = vec![
            ("Space", "select"),
            ("y", "copy"),
            ("c", copy_label),
            ("+", "react"),
            ("r", "reply"),
        ];
        // Offer open/copy-link only when the message actually has a link.
        let body_text = match &m.content {
            MessageContent::Text(b) => b.as_str(),
            MessageContent::Edit { body, .. } => body.as_str(),
            MessageContent::Attachment(a) => a.title.as_str(),
            _ => "",
        };
        if !crate::domain::extract_urls(body_text).is_empty() {
            actions.push(("o", "open link"));
            actions.push(("l", "copy link"));
        }
        if is_me {
            actions.push(("e", "edit"));
            actions.push(("Shift+X", "delete"));
        }
        actions.push(("p", "pin"));
        if is_attachment {
            actions.push(("s", "download"));
        }
        actions
    };
    // Pack actions onto as few lines as fit `width` — they all show on one
    // line when there's room, otherwise wrap onto continuation lines (each
    // flush-left, aligned with the message's "→" arrow). Never truncated.
    let key_style = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(t.dim);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let mut line_w = 1usize; // the leading gutter
    let mut first = true;
    if marks > 0 {
        let pfx = format!("{marks} sel   ");
        line_w += pfx.chars().count();
        spans.push(Span::styled(pfx, key_style));
    }
    for (k, label) in &actions {
        let piece = k.chars().count() + 1 + label.chars().count(); // "k label"
        if !first && line_w + 2 + piece > width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans.push(Span::raw(" "));
            line_w = 1;
            first = true;
        }
        if !first {
            spans.push(Span::raw("  "));
            line_w += 2;
        }
        spans.push(Span::styled((*k).to_string(), key_style));
        spans.push(Span::styled(format!(" {label}"), label_style));
        line_w += piece;
        first = false;
    }
    lines.push(Line::from(spans));
    lines
}

/// Maps a stored reaction key (a `:shortcode:`) to its glyph via the emoji
/// catalogue, so the chat shows 🫡 rather than `:saluting_face:`. Falls back
/// to the key as-is (already a glyph, or an unknown/custom shortcode).
/// How a stored reaction (`:alias:` for custom, or a raw Unicode glyph for
/// stock) is shown, honouring the `emoji_style` setting: the **glyph** (default)
/// or the **`:shortcode:`** (legible even when the terminal renders emoji as
/// tofu / monochrome — the only lever a TUI has, since it can't pick the font).
fn reaction_display(app: &App, key: &str) -> String {
    let alias = key.trim_matches(':');
    // Match by alias (shortcode key) or by display glyph (stock emoji sent raw).
    let entry = app
        .emojis
        .iter()
        .find(|e| e.alias == alias || e.display == key);
    if app.settings_cache.emoji_style == "shortcode" {
        match entry {
            Some(e) => format!(":{}:", e.alias),
            None if key.starts_with(':') => key.to_string(),
            None => format!(":{alias}:"),
        }
    } else {
        entry
            .map(|e| e.display.clone())
            .unwrap_or_else(|| key.to_string())
    }
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
    mentions: &[String],
) -> Vec<Line<'static>> {
    match content {
        MessageContent::Text(body) => render_text_body(body, t, width, mentions),
        MessageContent::Edit { target_id, body } => {
            let label = format!("(edited msg #{target_id})");
            let mut lines = vec![Line::from(Span::styled(
                format!("    {label}"),
                Style::default().fg(t.dim),
            ))];
            lines.extend(render_text_body(body, t, width, mentions));
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
        MessageContent::Attachment(att) => render_attachment(att, t, width),
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

fn render_text_body(
    body: &str,
    t: &crate::tui::theme::Theme,
    width: usize,
    mentions: &[String],
) -> Vec<Line<'static>> {
    if body.is_empty() {
        return placeholder("(empty)", t);
    }
    // Parse each line into styled runs (markdown + mentions), then wrap the
    // runs to the panel width (minus the 4-space body indent), preserving the
    // styles across line breaks. Block elements (``` fences, > quotes) are
    // handled at the line level around the inline pass.
    let wrap_w = width.saturating_sub(4).max(8);
    let mut lines = Vec::new();
    // Accumulates a fenced block between ``` markers as `(language, lines)` so
    // the whole block can be syntax-highlighted at once (the language token is
    // captured from the opening fence).
    let mut fence: Option<(String, Vec<String>)> = None;
    for l in body.lines() {
        // A ``` line opens or closes a fenced code block (the marker is hidden).
        if let Some(after) = l.trim_start().strip_prefix("```") {
            if let Some((lang, code)) = fence.take() {
                // Closing fence → flush the accumulated block.
                push_code_block(&mut lines, &code, &lang, wrap_w, t);
            } else if let Some(close) = after.rfind("```") {
                // A single-line ```lang code``` is a one-line code block, not a
                // multi-line fence — render it now. Treated as a fence opener it
                // would capture no content and emit nothing, so the whole
                // message body rendered invisibly.
                let (lang, code) = split_inline_fence(&after[..close]);
                push_code_block(&mut lines, std::slice::from_ref(&code), &lang, wrap_w, t);
            } else {
                // Opening a multi-line fence; only the first token is the language.
                let lang = after.split_whitespace().next().unwrap_or("").to_string();
                fence = Some((lang, Vec::new()));
            }
            continue;
        }
        if let Some((_, code)) = fence.as_mut() {
            code.push(l.to_string());
            continue;
        }
        // Blockquote: `>` (with an optional space) → a dim `▏` bar + content.
        if let Some(rest) = l.trim_start().strip_prefix('>') {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            let runs = crate::domain::parse_inline(rest, mentions);
            for spans in wrap_runs(&runs, wrap_w.saturating_sub(2), t) {
                let mut row = vec![
                    Span::raw("   "),
                    Span::styled("▏ ", Style::default().fg(t.dim)),
                ];
                row.extend(spans);
                lines.push(Line::from(row));
            }
            continue;
        }
        let runs = crate::domain::parse_inline(l, mentions);
        for spans in wrap_runs(&runs, wrap_w, t) {
            let mut row = vec![Span::raw("    ")];
            row.extend(spans);
            lines.push(Line::from(row));
        }
    }
    // An unterminated fence (no closing ```) still renders its accumulated code.
    if let Some((lang, code)) = fence.take() {
        push_code_block(&mut lines, &code, &lang, wrap_w, t);
    }
    // Safety net: a non-empty body must never render to nothing (e.g. a
    // malformed or empty fence). Fall back to the raw text so the message is
    // never invisible.
    if lines.is_empty() {
        for raw in body.lines() {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::raw(raw.to_string()),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::raw("    ")));
        }
    }
    lines
}

/// Splits the inner text of a single-line ```…``` fence into `(language, code)`.
/// The first whitespace-delimited token is taken as the language only when it
/// looks like a language tag and code follows it (so ```rust foo``` →
/// `("rust", "foo")`); otherwise the whole thing is code with no language (so
/// ```{"a":1}``` stays intact and just renders flat).
fn split_inline_fence(inner: &str) -> (String, String) {
    let trimmed = inner.trim();
    if let Some((first, rest)) = trimmed.split_once(char::is_whitespace) {
        let rest = rest.trim_start();
        let looks_like_lang = !first.is_empty()
            && first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-' | '_'));
        if looks_like_lang && !rest.is_empty() {
            return (first.to_string(), rest.to_string());
        }
    }
    (String::new(), trimmed.to_string())
}

/// Renders a whole fenced code block. When the fence named a language
/// `syntect` recognises, each line is syntax-highlighted; otherwise it falls
/// back to the flat single-colour renderer so the code is never dropped.
fn push_code_block(
    lines: &mut Vec<Line<'static>>,
    code_lines: &[String],
    lang: &str,
    width: usize,
    t: &crate::tui::theme::Theme,
) {
    let code = code_lines.join("\n");
    match crate::tui::syntax::highlight(&code, lang, code_theme_is_dark(t)) {
        Some(per_line) => {
            let inner = width.saturating_sub(2).max(4);
            for segs in &per_line {
                push_colored_code_line(lines, segs, inner, t);
            }
        }
        None => {
            for l in code_lines {
                push_code_block_line(lines, l, width, t);
            }
        }
    }
}

/// Renders one highlighted source line: a dim `▏` bar plus the coloured token
/// segments, hard-wrapped (no word-wrap) to `inner` so alignment is preserved.
fn push_colored_code_line(
    lines: &mut Vec<Line<'static>>,
    segs: &[(Color, String)],
    inner: usize,
    t: &crate::tui::theme::Theme,
) {
    let bar = Style::default().fg(t.dim);
    let prefix = || vec![Span::raw("   "), Span::styled("▏ ", bar)];
    let flat: Vec<(Color, char)> = segs
        .iter()
        .flat_map(|(c, s)| s.chars().map(move |ch| (*c, ch)))
        .collect();
    if flat.is_empty() {
        lines.push(Line::from(prefix()));
        return;
    }
    for chunk in flat.chunks(inner) {
        let mut row = prefix();
        let mut color = chunk[0].0;
        let mut buf = String::new();
        for &(c, ch) in chunk {
            // Coalesce consecutive same-colour chars into one span.
            if c != color && !buf.is_empty() {
                row.push(Span::styled(
                    std::mem::take(&mut buf),
                    Style::default().fg(color),
                ));
            }
            color = c;
            buf.push(ch);
        }
        if !buf.is_empty() {
            row.push(Span::styled(buf, Style::default().fg(color)));
        }
        lines.push(Line::from(row));
    }
}

/// Whether to highlight code with a dark or light `syntect` theme, inferred
/// from the active theme's body-text brightness (bright text ⇒ dark terminal).
/// `Color::Reset`/indexed foregrounds default to dark, the common terminal.
fn code_theme_is_dark(t: &crate::tui::theme::Theme) -> bool {
    match t.foreground {
        Color::Rgb(r, g, b) => {
            let lum = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
            lum > 128.0
        }
        _ => true,
    }
}

/// Renders one verbatim line of a fenced code block: a dim `▏` bar plus the
/// raw text in the code colour, hard-wrapped (no markup, no word-wrap).
fn push_code_block_line(
    lines: &mut Vec<Line<'static>>,
    raw: &str,
    width: usize,
    t: &crate::tui::theme::Theme,
) {
    let style = Style::default().fg(t.conv_team);
    let bar = Style::default().fg(t.dim);
    let inner = width.saturating_sub(2).max(4);
    let chars: Vec<char> = raw.chars().collect();
    if chars.is_empty() {
        lines.push(Line::from(vec![Span::raw("   "), Span::styled("▏ ", bar)]));
        return;
    }
    for chunk in chars.chunks(inner) {
        let s: String = chunk.iter().collect();
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled("▏ ", bar),
            Span::styled(s, style),
        ]));
    }
}

/// The ratatui style for a parsed markdown [`crate::domain::Run`].
fn run_style(r: &crate::domain::Run, t: &crate::tui::theme::Theme) -> Style {
    let fg = if r.mention {
        t.accent
    } else if r.code {
        t.conv_team
    } else {
        t.foreground
    };
    let mut m = Modifier::empty();
    if r.bold || r.mention {
        m |= Modifier::BOLD;
    }
    if r.italic {
        m |= Modifier::ITALIC;
    }
    if r.strike {
        m |= Modifier::CROSSED_OUT;
    }
    Style::default().fg(fg).add_modifier(m)
}

/// Word-wraps styled runs to `width` columns, producing one span list per
/// output line. Styles are carried across the wrap; over-long words hard-split.
fn wrap_runs(
    runs: &[crate::domain::Run],
    width: usize,
    t: &crate::tui::theme::Theme,
) -> Vec<Vec<Span<'static>>> {
    let mut flat: Vec<(char, Style)> = Vec::new();
    for r in runs {
        let st = run_style(r, t);
        for ch in r.text.chars() {
            flat.push((ch, st));
        }
    }
    let mut out: Vec<Vec<(char, Style)>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut word: Vec<(char, Style)> = Vec::new();
    for (ch, st) in flat {
        if ch == ' ' {
            commit_word(&mut out, &mut cur, &mut word, width);
            if cur.len() + 1 > width {
                out.push(std::mem::take(&mut cur));
            } else if !cur.is_empty() {
                cur.push((' ', st));
            }
        } else {
            word.push((ch, st));
        }
    }
    commit_word(&mut out, &mut cur, &mut word, width);
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out.into_iter().map(coalesce_spans).collect()
}

/// Flushes the pending `word` onto `cur`, wrapping `cur` first if it wouldn't
/// fit; a word longer than `width` is hard-split across lines.
fn commit_word(
    out: &mut Vec<Vec<(char, Style)>>,
    cur: &mut Vec<(char, Style)>,
    word: &mut Vec<(char, Style)>,
    width: usize,
) {
    if word.is_empty() {
        return;
    }
    if word.len() > width {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
        let mut chunk: Vec<(char, Style)> = Vec::new();
        for cs in word.drain(..) {
            if chunk.len() == width {
                out.push(std::mem::take(&mut chunk));
            }
            chunk.push(cs);
        }
        *cur = chunk;
        return;
    }
    if !cur.is_empty() && cur.len() + word.len() > width {
        out.push(std::mem::take(cur));
    }
    cur.append(word);
}

/// Coalesces a line of `(char, style)` into the fewest spans.
fn coalesce_spans(line: Vec<(char, Style)>) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur: Option<Style> = None;
    for (ch, st) in line {
        if cur != Some(st) {
            if let Some(s) = cur.take() {
                spans.push(Span::styled(std::mem::take(&mut buf), s));
            }
            cur = Some(st);
        }
        buf.push(ch);
    }
    if let Some(s) = cur {
        spans.push(Span::styled(buf, s));
    }
    spans
}

fn render_attachment(
    att: &AttachmentInfo,
    t: &crate::tui::theme::Theme,
    width: usize,
) -> Vec<Line<'static>> {
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
    let name = Span::styled(
        format!("    {}", att.filename),
        Style::default()
            .fg(t.conv_team)
            .add_modifier(Modifier::BOLD),
    );
    // Filename + size always share the first line. The MIME type joins them
    // (`· type`) when it fits the panel width; otherwise it drops to a dim
    // line below.
    let meta = format!("  {size}{state}");
    let first_used = att.filename.chars().count() + 4 + meta.chars().count();
    let type_inline = format!("  · {mime}");
    if first_used + type_inline.chars().count() <= width {
        lines.push(Line::from(vec![
            name,
            Span::styled(format!("{meta}{type_inline}"), Style::default().fg(t.dim)),
        ]));
    } else {
        lines.push(Line::from(vec![
            name,
            Span::styled(meta, Style::default().fg(t.dim)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("      {mime}"),
            Style::default().fg(t.dim),
        )));
    }
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
    fn styled_runs_wrap_and_carry_their_style() {
        let t = crate::tui::theme::Theme::default();
        let runs = crate::domain::parse_inline("a *bold* word", &[]);
        // Wide → one line; the "bold" span is bold.
        let wide = wrap_runs(&runs, 40, &t);
        assert_eq!(wide.len(), 1);
        let bold = wide[0].iter().find(|s| s.content == "bold").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        // Narrow → wraps onto multiple lines without losing content.
        let narrow = wrap_runs(&runs, 6, &t);
        assert!(narrow.len() > 1);
    }

    #[test]
    fn block_markdown_hides_fences_and_marks_quotes() {
        let t = crate::tui::theme::Theme::default();
        let body = "before\n```\ncode line\n```\n> quoted";
        let lines = render_text_body(body, &t, 40, &[]);
        let rows: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        // The ``` fence markers are not rendered.
        assert!(rows.iter().all(|r: &String| !r.contains("```")));
        // The code line and the quoted line (with its ▏ bar) survive.
        assert!(rows.iter().any(|r| r.contains("code line")));
        assert!(rows.iter().any(|r| r.contains("▏") && r.contains("quoted")));
    }

    #[test]
    fn fenced_block_with_language_is_syntax_highlighted() {
        let t = crate::tui::theme::Theme::default();
        let body = "```rust\nfn main() { let x = 1; }\n```";
        let lines = render_text_body(body, &t, 60, &[]);
        let rows: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        // Fence hidden, code text preserved.
        assert!(rows.iter().all(|r: &String| !r.contains("```")));
        assert!(rows.iter().any(|r| r.contains("fn main")));
        // Highlighting splits the line into several distinctly-coloured spans;
        // the flat fallback would yield only {indent, bar, one code colour}.
        let colors: std::collections::HashSet<String> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| format!("{:?}", s.style.fg))
            .collect();
        assert!(
            colors.len() > 3,
            "expected multiple token colours from syntect, got {colors:?}"
        );
    }

    #[test]
    fn single_line_fence_is_not_invisible() {
        // Regression: ```json {"ok": true}``` on ONE line used to open a fence
        // that captured no content and rendered zero lines — an invisible
        // message body.
        let t = crate::tui::theme::Theme::default();
        let lines = render_text_body("```json {\"ok\": true}```", &t, 60, &[]);
        assert!(
            !lines.is_empty(),
            "a single-line fence must render something"
        );
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            text.contains("{\"ok\": true}"),
            "code must be visible: {text:?}"
        );
        assert!(!text.contains("```"), "fence markers stay hidden: {text:?}");
    }

    #[test]
    fn single_line_fence_without_language_renders_flat() {
        let t = crate::tui::theme::Theme::default();
        let lines = render_text_body("```{\"a\":1}```", &t, 60, &[]);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("{\"a\":1}"), "got {text:?}");
    }

    #[test]
    fn malformed_fence_body_is_never_invisible() {
        // An opening fence with no content and no close must still render
        // *something* (the raw text), never an empty body.
        let t = crate::tui::theme::Theme::default();
        assert!(!render_text_body("```", &t, 60, &[]).is_empty());
        assert!(!render_text_body("```json", &t, 60, &[]).is_empty());
    }

    #[test]
    fn split_inline_fence_extracts_language_or_keeps_code() {
        assert_eq!(
            split_inline_fence("json {\"ok\": true}"),
            ("json".to_string(), "{\"ok\": true}".to_string())
        );
        // No language tag → the whole thing is code, kept intact.
        assert_eq!(
            split_inline_fence("{\"a\":1}"),
            (String::new(), "{\"a\":1}".to_string())
        );
        assert_eq!(
            split_inline_fence("{\"a\": 1}"),
            (String::new(), "{\"a\": 1}".to_string())
        );
    }

    #[test]
    fn fenced_block_without_language_falls_back_to_flat() {
        // No language → no syntect call; the code still renders (single colour),
        // never dropped.
        let t = crate::tui::theme::Theme::default();
        let lines = render_text_body("```\nplain code\n```", &t, 60, &[]);
        assert!(
            lines
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("plain code")))
        );
    }

    #[test]
    fn attachment_type_inline_when_it_fits_else_below() {
        let t = crate::tui::theme::Theme::default();
        let mut att = AttachmentInfo::default();
        att.filename = "a.txt".into();
        att.size = 1024;
        att.mime_type = "text/plain".into();
        att.uploaded = true;
        // Wide panel → the type joins the size line (one line).
        assert_eq!(render_attachment(&att, &t, 80).len(), 1);
        // Narrow panel → the type drops to its own line below (two lines).
        assert_eq!(render_attachment(&att, &t, 20).len(), 2);
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
