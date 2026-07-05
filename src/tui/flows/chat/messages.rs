//! Open-conversation flows: reading history (first page / pagination /
//! live push), composing, editing, replying, deleting, reacting, pinning,
//! and Select-mode mechanics.

use crate::domain::Message;
use crate::ports::KeybaseError;
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::app::{PendingBatch, PendingSend, SendState};
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};

#[allow(unused_imports)]
use super::*;

// ── Open / close conversation detail screen ──────────────────────────

/// Shared conversation-open path: stash the previously-open draft, switch to
/// `id`, restore its draft, and load its messages.
pub(crate) fn enter_conversation(app: &mut App, id: String) {
    // Record how far the outgoing conversation was read before switching, so its
    // next open can place the `new messages` divider above anything since.
    record_conv_seen(app);
    stash_draft(app);
    app.open_conv_id = Some(id.clone());
    // Seed the unread boundary from what we last saw of *this* conversation this
    // session (None on a first-ever open → no divider).
    app.unread_boundary = app.conv_last_seen.get(&id).copied();
    app.messages.clear();
    // Reset the per-history projections (pin / headline / id index) so the
    // loading view can't show the previous conversation's pin or topic.
    app.rebuild_msg_meta();
    app.messages_scroll = 0;
    app.new_since_scroll = 0;
    app.compose_open = true;
    app.edit_target_id = None;
    app.reply_to_id = None;
    app.selected_msg_idx = None;
    app.pending_search_jump = None;
    app.conv_search.clear();
    app.conv_search_results.clear();
    app.conv_search_selected = 0;
    restore_draft(app, &id);
    // Reveal the conversation in the tree (expand its group + move the cursor),
    // so opening from the quick switcher / global search keeps the tree in sync.
    reveal_in_tree(app, &id);
    app.rebuild_conv_members(); // seed @-mention candidates from participants
    app.mention_selected = 0;
    // The unified Home keeps everything on the inbox screen — the chat shows
    // in the right pane and takes focus.
    app.screen = crate::tui::screens::Screen::Inbox;
    app.focus = crate::tui::screens::Focus::Chat;
    request_load_messages(app);
}

/// Expands the group containing `conv_id` and selects its row in the tree.
fn reveal_in_tree(app: &mut App, conv_id: &str) {
    if let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) {
        let key = if conv.channel.members_type.is_team() {
            conv.channel.name.clone()
        } else {
            App::DMS_KEY.to_string()
        };
        app.expanded.insert(key);
        app.rebuild_tree_rows();
    }
    if let Some(pos) = app.tree_rows().iter().position(|r| {
        matches!(r, crate::tui::app::TreeRow::Conv { idx } if app.conversations[*idx].id == conv_id)
    }) {
        app.tree_selected = pos;
    }
}

/// Saves the open conversation's compose text as its draft (in memory).
/// No-op while editing an existing message (that isn't a draft).
fn stash_draft(app: &mut App) {
    if app.edit_target_id.is_some() {
        return;
    }
    if let Some(id) = app.open_conv_id.clone() {
        let txt = app.compose.text().to_string();
        if txt.trim().is_empty() {
            app.drafts.remove(&id);
        } else {
            app.drafts.insert(id, txt);
        }
    }
}

/// Loads the conversation's saved draft into the compose (or clears it).
fn restore_draft(app: &mut App, id: &str) {
    match app.drafts.get(id) {
        Some(d) => app.compose.set(d.clone()),
        None => app.compose.clear(),
    }
}

pub fn close_conversation(app: &mut App) {
    record_conv_seen(app);
    stash_draft(app);
    app.pending_search_jump = None;
    close_conv_search(app);
    app.open_conv_id = None;
    app.unread_boundary = None;
    app.messages.clear();
    app.rebuild_msg_meta();
    app.messages_scroll = 0;
    app.messages_next = None;
    app.messages_loading_older = false;
    app.compose_clear();
    app.screen = crate::tui::screens::Screen::Inbox;
    app.focus = crate::tui::screens::Focus::Tree;
}

/// Records the highest loaded message id of the open conversation as **seen**
/// (session-local), so the next open can anchor the `new messages` divider above
/// whatever arrived since. No-op when nothing is open / loaded.
fn record_conv_seen(app: &mut App) {
    if let Some(id) = app.open_conv_id.clone()
        && let Some(last) = app.messages.last()
    {
        let entry = app.conv_last_seen.entry(id).or_insert(0);
        *entry = (*entry).max(last.id);
    }
}

/// Opens a conversation directly by id (used by the quick switcher, which
/// may target a conversation outside the current inbox filter).
pub fn open_conversation_by_id(app: &mut App, id: String) {
    enter_conversation(app, id);
}

// ── Quick switcher (Ctrl+K) ──────────────────────────────────────────

pub fn open_quick_switcher(app: &mut App) {
    app.switcher_from = app.screen;
    app.switcher.clear();
    app.switcher_selected = 0;
    app.screen = crate::tui::screens::Screen::QuickSwitcher;
}

pub fn close_quick_switcher(app: &mut App) {
    app.switcher.clear();
    app.screen = app.switcher_from;
}

/// Jumps to the highlighted conversation in the switcher.
pub fn quick_switcher_open_selected(app: &mut App) {
    let selectable = app.switcher_selectable();
    let Some(&i) = selectable.get(app.switcher_selected) else {
        close_quick_switcher(app);
        return;
    };
    let id = app.conversations[i].id.clone();
    app.switcher.clear();
    open_conversation_by_id(app, id);
}

// ── Load messages (first page) ───────────────────────────────────────

pub fn request_load_messages(app: &mut App) {
    request_load_messages_num(app, MESSAGES_PER_PAGE);
}

/// Cap on a depth-preserving re-read ([`request_reload_messages`]), so a
/// reader who paged back thousands of messages doesn't turn every control
/// event into a huge fetch. Beyond it the oldest loaded scrollback is
/// dropped (the plain first-page behaviour).
const RELOAD_DEPTH_MAX: u32 = MESSAGES_PER_PAGE * 8;

/// Quiet re-read of the open conversation that asks for **as many messages
/// as are currently loaded** (min one page, capped) instead of just the
/// first page — so a re-read triggered by an edit / delete / reaction /
/// pin doesn't throw away the scrollback the reader paged into.
pub fn request_reload_messages(app: &mut App) {
    let loaded = u32::try_from(app.messages.len()).unwrap_or(RELOAD_DEPTH_MAX);
    request_load_messages_num(app, loaded.clamp(MESSAGES_PER_PAGE, RELOAD_DEPTH_MAX));
}

fn request_load_messages_num(app: &mut App, num: u32) {
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        app.screen = crate::tui::screens::Screen::Inbox;
        app.open_conv_id = None;
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    // Reset pagination before loading the first page so a stale cursor
    // from a previously-open conversation can never be reused (e.g. if
    // this load fails, `messages_next` must not still point at the prior
    // conversation's history).
    app.messages_next = None;
    app.messages_loading_older = false;
    let peek = !app.settings_cache.auto_mark_read;
    app.submit(
        InFlight::LoadMessages,
        "Loading messages…",
        WorkerRequest::ReadMessages {
            channel,
            num,
            peek,
            next_cursor: None,
        },
    );
}

pub fn handle_load_messages_response(
    app: &mut App,
    result: Result<(Vec<Message>, Option<String>), KeybaseError>,
) {
    // The user may have closed the conversation (Esc passes through
    // while busy) before this response landed — drop the orphan so we
    // don't write a closed conversation's messages over the inbox state.
    if app.open_conv_id.is_none() {
        app.messages_loading_older = false;
        return;
    }
    match result {
        Ok((mut msgs, next)) => {
            // Which message the Select-mode cursor sat on, by id — captured
            // before the list is replaced so it can be re-found afterwards.
            let prev_cursor_id = app
                .selected_msg_idx
                .and_then(|i| app.messages.get(i))
                .map(|m| m.id);
            msgs.reverse();
            app.messages = project_messages(msgs);
            app.rebuild_conv_members(); // add the people who've spoken
            let n = app.messages.len();
            app.messages_next = next;
            app.messages_loading_older = false;
            app.rebuild_msg_meta();
            // A control-op re-read (delete/edit/react) keeps the reader where
            // they were; a fresh open/refresh snaps to the latest message.
            if !std::mem::take(&mut app.preserve_msg_scroll) {
                app.messages_scroll = 0;
            }
            // Re-anchor Select mode across the reprojection. Marks are ids:
            // prune any whose message is gone (batch- or remote-deleted).
            // The cursor re-seeks its message by id; only when that message
            // no longer exists does it fall back to clamping the old index
            // (or clearing, if the conversation is now empty).
            app.msg_marks.retain(|id| app.msg_index.contains_key(id));
            if let Some(i) = app.selected_msg_idx {
                app.selected_msg_idx = if app.messages.is_empty() {
                    None
                } else {
                    Some(
                        prev_cursor_id
                            .and_then(|id| app.msg_index.get(&id).copied())
                            .unwrap_or_else(|| i.min(app.messages.len() - 1)),
                    )
                };
            }
            // This fresh read includes any optimistic send that just
            // succeeded, so drop the Delivered bubbles for this
            // conversation — the real messages now stand in for them.
            // Pending/Failed entries stay.
            let open = app.open_conv_id.clone();
            app.outbox.retain(|p| {
                !(p.state == SendState::Delivered && Some(&p.conv_id) == open.as_ref())
            });
            // Opening with auto-mark-read does a non-peek `read`, which
            // marks the conversation read server-side — so clear the inbox
            // unread badge for it locally now, instead of waiting for the
            // next inbox refresh.
            if app.settings_cache.auto_mark_read
                && let Some(id) = app.open_conv_id.clone()
            {
                let cleared = app
                    .conversations
                    .iter_mut()
                    .find(|c| c.id == id)
                    .map(|c| std::mem::replace(&mut c.unread, false))
                    .unwrap_or(false);
                if cleared {
                    app.rebuild_filter_preserving_cursor();
                }
            }
            app.set_action(ActionState::Done(format!("Loaded {n} messages")));
            app.push_cmd("keybase chat api read", true, format!("{n} messages"));
            try_jump_to_search_target(app);
        }
        Err(e) => {
            app.messages_loading_older = false;
            app.pending_search_jump = None;
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api read", false, e.to_string());
        }
    }
}

// ── Incoming push messages (from the api-listen stream) ─────────────

/// Applies a message pushed by the listener: live-appends it to the open
/// conversation (if that's the one being viewed) and bumps the matching
/// inbox conversation (recency + unread). A message for a conversation
/// not yet in the inbox triggers a silent resync to pull it in.
pub fn handle_incoming_message(app: &mut App, conv_id: String, message: Message) {
    use crate::domain::MessageContent;
    let from_me = !app.identity.username.is_empty() && message.sender == app.identity.username;
    let viewing = app.open_conv_id.as_deref() == Some(conv_id.as_str());
    // Control events (edit / delete / reaction) aren't "new content": they must
    // not bump the conversation's recency or mark it unread — otherwise your own
    // delete resurfaces the chat with a phantom unread badge.
    let is_control = matches!(
        &message.content,
        MessageContent::Edit { .. }
            | MessageContent::Delete { .. }
            | MessageContent::Reaction { .. }
    );
    let sent_at = message.sent_at;
    let sent_at_ms = message.sent_at_ms;
    let msg_id = message.id;

    // 1. Live-update the open conversation.
    if viewing {
        // Edits / deletes / reactions modify *existing* messages (and
        // reactions are projected onto their target), so they can't just
        // be appended as a new line — re-read the conversation to get the
        // correctly-projected state (cheap, ~80ms). `begin` debounces:
        // it refuses if a read is already in flight. Plain new content
        // (text, attachment, system, pin, join, …) is appended in place,
        // which is instant and needs no round-trip.
        if is_control {
            // A live edit/delete/reaction reprojects in place — don't yank the
            // reader to the bottom, and keep the loaded scrollback depth.
            app.preserve_msg_scroll = true;
            request_reload_messages(app);
        } else if msg_id != 0 && !app.messages.iter().any(|m| m.id == msg_id) {
            app.messages.push(message);
            app.rebuild_msg_meta();
            // If the reader is scrolled up in history, a new arrival lands below
            // the fold — count it for the floating "▼ N new · End" jump cue.
            if app.messages_scroll > 0 && !from_me {
                app.new_since_scroll += 1;
            } else if app.messages_scroll == 0 && app.settings_cache.auto_mark_read {
                // Reading at the bottom: the message is seen the moment it
                // renders, but only a server-side `mark` moves the read
                // pointer — without it the next inbox resync re-marks the
                // conversation you're literally reading as unread (and the
                // phone badge keeps counting). Fire-and-forget on the
                // background lane; no in-flight ticket.
                if let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id)
                    && let Ok(channel) = read_channel_from_conv(conv)
                {
                    let _ = app.bg_worker_tx.send(WorkerRequest::MarkReadSilent {
                        channel,
                        message_id: msg_id,
                    });
                }
            }
        }
    }

    // 2. Incremental inbox bump (no full re-fetch).
    let known = if let Some(c) = app.conversations.iter_mut().find(|c| c.id == conv_id) {
        if !is_control && sent_at_ms > c.active_at_ms {
            c.active_at_ms = sent_at_ms;
            c.active_at = sent_at;
        }
        // Mark unread unless it's our own message, we're viewing it, or it's a
        // control event (edit/delete/reaction — not new content).
        if !from_me && !viewing && !is_control {
            c.unread = true;
        }
        true
    } else {
        false
    };
    if known {
        // A bump only touches recency/unread — fields the lowered projection
        // doesn't carry — so rebuilding it here (N conversations × 4 strings,
        // each zeroize-wiped on drop, per pushed message) would be pure waste.
        // Preserve the cursor: a busy team pushes constantly, and the recency
        // re-sort must not yank the user's tree selection to the top.
        app.rebuild_filter_preserving_cursor();
    } else {
        // First message of a conversation we don't have yet.
        request_load_inbox_silent(app);
    }
}

// ── Load older messages (pagination) ────────────────────────────────

pub fn request_load_older_messages(app: &mut App) {
    let Some(cursor) = app.messages_next.clone() else {
        app.messages_loading_older = false;
        app.set_action(ActionState::Done("No more history".into()));
        return;
    };
    let Some((_, channel)) = open_channel(app) else {
        // No open conversation, it left the inbox, or its type is
        // unsupported — clear the in-progress flag so the viewport
        // doesn't get stuck showing "loading older…".
        app.messages_loading_older = false;
        return;
    };
    let peek = !app.settings_cache.auto_mark_read;
    app.submit(
        InFlight::LoadOlderMessages,
        "Loading older messages…",
        WorkerRequest::ReadMessages {
            channel,
            num: MESSAGES_PER_PAGE,
            peek,
            next_cursor: Some(cursor),
        },
    );
}

pub fn handle_load_older_messages_response(
    app: &mut App,
    result: Result<(Vec<Message>, Option<String>), KeybaseError>,
) {
    // Drop the orphan if the conversation was closed mid-flight.
    if app.open_conv_id.is_none() {
        app.messages_loading_older = false;
        return;
    }
    match result {
        Ok((mut older, next)) => {
            older.reverse();
            older = project_messages(older);
            let n = older.len();
            older.extend(std::mem::take(&mut app.messages));
            app.messages = older;
            app.messages_next = next;
            app.messages_loading_older = false;
            app.rebuild_msg_meta();
            app.set_action(ActionState::Done(format!("Loaded {n} older messages")));
            app.push_cmd(
                "keybase chat api read (older)",
                true,
                format!("{n} messages"),
            );
            try_jump_to_search_target(app);
        }
        Err(e) => {
            app.messages_loading_older = false;
            app.pending_search_jump = None;
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api read (older)", false, e.to_string());
        }
    }
}

// ── Escape semantics on the conversation screen ─────────────────────

/// Esc in the compose pane — the vim chain, one layer per press:
/// cancel an in-progress **edit**, else cancel a **reply** target (the typed
/// text survives, matching Slack), else leave insert for **Select mode**
/// (the draft stays in the compose buffer — Esc never destroys typed text;
/// closing later stashes it via [`close_conversation`]). An empty
/// conversation has nothing to select, so Esc closes it directly.
/// The single Esc path — the input handler routes here.
pub fn escape_conversation(app: &mut App) {
    if app.edit_target_id.is_some() {
        cancel_edit(app);
    } else if app.reply_to_id.is_some() {
        app.reply_to_id = None;
    } else if !app.messages.is_empty() {
        enter_select_mode(app);
    } else {
        close_conversation(app);
    }
}

// ── Message selection (Compose ↔ Select modes) ──────────────────────

pub fn enter_select_mode(app: &mut App) {
    if app.messages.is_empty() {
        app.set_action(ActionState::Error("No messages to select".into()));
        return;
    }
    app.compose_open = false;
    app.select_from_compose = false;
    app.selected_msg_idx = Some(app.messages.len() - 1);
    app.msg_marks.clear();
    app.select_anchor = None;
}

pub fn leave_select_mode(app: &mut App) {
    app.selected_msg_idx = None;
    app.select_from_compose = false;
    app.compose_open = true;
    app.msg_marks.clear();
    app.select_anchor = None;
}

/// Toggles the mark on the cursor message (manual one-by-one multi-select).
pub fn msg_toggle_mark(app: &mut App) {
    let Some(id) = app
        .selected_msg_idx
        .and_then(|i| app.messages.get(i))
        .map(|m| m.id)
    else {
        return;
    };
    app.select_anchor = None;
    if !app.msg_marks.remove(&id) {
        app.msg_marks.insert(id);
    }
}

/// Extends a contiguous shaded selection by `delta` (Shift+↑/↓), editor-style:
/// the anchor stays put while the cursor moves and the whole range is marked.
pub fn select_extend(app: &mut App, delta: isize) {
    let Some(cur) = app.selected_msg_idx else {
        return;
    };
    let max = app.messages.len().saturating_sub(1);
    let anchor = *app.select_anchor.get_or_insert(cur);
    let new = (cur as isize + delta).clamp(0, max as isize) as usize;
    app.selected_msg_idx = Some(new);
    let (lo, hi) = (anchor.min(new), anchor.max(new));
    app.msg_marks = (lo..=hi)
        .filter_map(|i| app.messages.get(i))
        .map(|m| m.id)
        .collect();
}

/// Copies the selected messages (marked, or the cursor message) to the
/// clipboard. `full` includes the author + timestamp above each body;
/// otherwise just the bodies, one per line. Each message is separated by a
/// blank line in full mode, a single newline in content mode.
pub fn do_copy_messages(app: &mut App, full: bool) {
    let mut idxs: Vec<usize> = if app.msg_marks.is_empty() {
        app.selected_msg_idx.into_iter().collect()
    } else {
        let mut v: Vec<usize> = app
            .msg_marks
            .iter()
            .filter_map(|id| app.msg_index.get(id).copied())
            .collect();
        v.sort_unstable();
        v
    };
    idxs.retain(|&i| i < app.messages.len());
    if idxs.is_empty() {
        app.set_action(ActionState::Error("No messages selected".into()));
        return;
    }
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut blocks: Vec<String> = Vec::new();
    for &i in &idxs {
        let m = &app.messages[i];
        let Some(body) = message_copy_body(m) else {
            continue; // skip system rows / non-text content
        };
        if full {
            let when = crate::domain::message_time(m.sent_at, now_s);
            let head = if when.is_empty() {
                m.sender.clone()
            } else {
                format!("{}  {when}", m.sender)
            };
            blocks.push(format!("{head}\n{body}"));
        } else {
            blocks.push(body);
        }
    }
    if blocks.is_empty() {
        app.set_action(ActionState::Error("Nothing to copy".into()));
        return;
    }
    let sep = if full { "\n\n" } else { "\n" };
    let text = blocks.join(sep);
    let n = blocks.len();
    let secs = app.settings_cache.clipboard_clear_secs;
    match app.clipboard.write_with_clear(&text, secs) {
        Ok(()) => {
            let what = if full { "full" } else { "content" };
            app.set_action(ActionState::Done(format!(
                "Copied {n} message{} ({what})",
                if n == 1 { "" } else { "s" }
            )));
            app.push_cmd("clipboard write", true, format!("{n} message(s), {what}"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("clipboard write", false, e.to_string());
        }
    }
}

/// The copyable text body of a message, or `None` for system / non-text rows.
fn message_copy_body(m: &crate::domain::Message) -> Option<String> {
    use crate::domain::MessageContent;
    match &m.content {
        MessageContent::Text(b) => Some(b.clone()),
        MessageContent::Edit { body, .. } => Some(body.clone()),
        MessageContent::Attachment(a) => Some(format!("[attachment: {}]", a.filename)),
        _ => None,
    }
}

/// The `http(s)` links in the selected message, in order (empty if none / no
/// selection). Shared by the open-link and copy-link actions.
fn selected_message_urls(app: &App) -> Vec<String> {
    use crate::domain::MessageContent;
    let Some(idx) = app.selected_msg_idx else {
        return Vec::new();
    };
    let body = match app.messages.get(idx).map(|m| &m.content) {
        Some(MessageContent::Text(b)) => b.clone(),
        Some(MessageContent::Edit { body, .. }) => body.clone(),
        Some(MessageContent::Attachment(a)) => a.title.clone(),
        _ => String::new(),
    };
    crate::domain::extract_urls(&body)
}

/// Opens the first `http(s)` link in the selected message with the OS browser
/// (`o` in select mode). Reports when the message has no link.
pub fn do_open_url(app: &mut App) {
    let urls = selected_message_urls(app);
    let Some(url) = urls.first().cloned() else {
        app.set_action(ActionState::Error("No link in this message".into()));
        return;
    };
    let extra = if urls.len() > 1 {
        format!("  (+{} more)", urls.len() - 1)
    } else {
        String::new()
    };
    match app.opener.open(&url) {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Opened {url}{extra}")));
            app.push_cmd("open url", true, url);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.clone()));
            app.push_cmd("open url", false, e);
        }
    }
}

/// Accepts the `@`-mention autocomplete: replaces the in-progress `@prefix`
/// at the cursor with `@username ` (trailing space).
pub fn accept_mention(app: &mut App, username: &str) {
    let Some((start, _)) = crate::domain::active_mention(app.compose.text(), app.compose.cursor())
    else {
        return;
    };
    while app.compose.cursor() > start {
        app.compose.backspace();
    }
    app.compose.insert_str(&format!("@{username} "));
    app.mention_selected = 0;
}

/// Copies the first `http(s)` link in the selected message to the clipboard
/// (`L` in select mode). Reports when the message has no link.
pub fn do_copy_url(app: &mut App) {
    let Some(url) = selected_message_urls(app).into_iter().next() else {
        app.set_action(ActionState::Error("No link in this message".into()));
        return;
    };
    let secs = app.settings_cache.clipboard_clear_secs;
    match app.clipboard.write_with_clear(&url, secs) {
        Ok(()) => {
            app.set_action(ActionState::Done("Link copied".into()));
            app.push_cmd("clipboard write", true, url);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("clipboard write", false, e.to_string());
        }
    }
}

pub fn select_move_up(app: &mut App) {
    app.select_anchor = None; // a plain move re-anchors the next shift-range
    if let Some(i) = app.selected_msg_idx
        && i > 0
    {
        app.selected_msg_idx = Some(i - 1);
    }
}

pub fn select_move_down(app: &mut App) {
    app.select_anchor = None;
    let max = app.messages.len().saturating_sub(1);
    if let Some(i) = app.selected_msg_idx
        && i < max
    {
        app.selected_msg_idx = Some(i + 1);
    }
}

// ── Edit ─────────────────────────────────────────────────────────────

pub fn open_edit_for_selected(app: &mut App) {
    let Some(idx) = app.selected_msg_idx else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.messages.get(idx) else {
        return;
    };
    if msg.sender != app.identity.username {
        app.set_action(ActionState::Error("Can only edit your own messages".into()));
        return;
    }
    // For an Edit envelope, `msg.id` is the id of the edit *operation*;
    // the message to re-edit is its inner `target_id`. Targeting msg.id
    // would edit the wrong (envelope) message.
    let (body, target_id) = match &msg.content {
        crate::domain::MessageContent::Text(s) => (s.clone(), msg.id),
        crate::domain::MessageContent::Edit { target_id, body } => (body.clone(), *target_id),
        _ => {
            app.set_action(ActionState::Error(
                "Only text messages can be edited".into(),
            ));
            return;
        }
    };
    app.edit_target_id = Some(target_id);
    app.compose_clear();
    app.compose.set(body);
    app.compose_open = true;
    app.selected_msg_idx = None;
    app.msg_marks.clear(); // editing is single-message; drop any shading
    app.select_anchor = None;
}

pub fn request_save_edit(app: &mut App) {
    let Some(target_id) = app.edit_target_id else {
        app.set_action(ActionState::Error("No edit in progress".into()));
        return;
    };
    let body = app.compose.text().trim().to_string();
    if body.is_empty() {
        app.set_action(ActionState::Error("Edit body is empty".into()));
        return;
    }
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    app.submit(
        InFlight::EditMessage { target_id },
        "Saving edit…",
        WorkerRequest::EditMessage {
            channel,
            message_id: target_id,
            body,
        },
    );
}

pub fn handle_save_edit_response(app: &mut App, result: Result<(), KeybaseError>, target_id: u64) {
    match result {
        Ok(()) => {
            app.compose_clear();
            app.edit_target_id = None;
            app.set_action(ActionState::Done("Edit saved".into()));
            app.push_cmd("keybase chat api edit", true, format!("msg #{target_id}"));
            app.preserve_msg_scroll = true; // stay where the reader was
            request_reload_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api edit", false, e.to_string());
        }
    }
}

pub fn cancel_edit(app: &mut App) {
    app.edit_target_id = None;
    app.compose_clear();
}

// ── Threaded replies ─────────────────────────────────────────────────

pub fn start_reply_for_selected(app: &mut App) {
    let Some(idx) = app.selected_msg_idx else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.messages.get(idx) else {
        return;
    };
    app.edit_target_id = None;
    app.reply_to_id = Some(msg.id);
    app.compose.clear();
    app.compose_open = true;
    app.selected_msg_idx = None;
    app.msg_marks.clear(); // replying is single-message; drop any shading
    app.select_anchor = None;
}

// ── Delete ───────────────────────────────────────────────────────────

// ── Multi-select batch actions (delete / react over `msg_marks`) ─────

/// The message ids the current Select-mode action targets: every **marked**
/// message (by id — stable across the reload), or the cursor message when
/// nothing is marked. `own_only` keeps just the local user's messages (delete).
fn selection_target_ids(app: &App, own_only: bool) -> Vec<u64> {
    let me = &app.identity.username;
    let keep = |m: &&crate::domain::Message| !own_only || &m.sender == me;
    let mut ids: Vec<u64> = if !app.msg_marks.is_empty() {
        let mut v: Vec<u64> = app
            .msg_marks
            .iter()
            .filter_map(|id| app.msg_index.get(id).and_then(|&i| app.messages.get(i)))
            .filter(keep)
            .map(|m| m.id)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    } else {
        app.selected_msg_idx
            .and_then(|i| app.messages.get(i))
            .filter(keep)
            .map(|m| m.id)
            .into_iter()
            .collect()
    };
    ids.retain(|&id| id != 0);
    ids
}

/// How many of the user's own messages a delete would remove — for the confirm.
pub fn delete_selection_count(app: &App) -> usize {
    selection_target_ids(app, true).len()
}

/// Cleans up after a batch (delete/react) completes or fails: clears the marks
/// and pending batch, keeps the reader in Select mode where they were. On
/// success it reloads so the change is reflected (the read handler re-anchors
/// the cursor); on failure it does **not** reload, so the error toast the
/// caller set stays visible instead of being overwritten by the read's toast.
fn finish_selection_batch(app: &mut App, reload: bool) {
    app.pending_batch = None;
    app.msg_marks.clear();
    app.select_anchor = None;
    if reload {
        app.preserve_msg_scroll = true; // stay put; stay in Select mode
        request_reload_messages(app);
    }
}

pub fn open_delete_for_selected(app: &mut App) {
    let n = delete_selection_count(app);
    if n == 0 {
        app.set_action(ActionState::Error(
            "Select your own message(s) to delete".into(),
        ));
        return;
    }
    app.delete_msg_yes = false;
    app.screen = Screen::ConfirmDeleteMessage;
}

pub fn close_delete_confirm(app: &mut App) {
    // If the delete was launched from Compose via Alt+D, returning must
    // land back in Compose — not in Select mode (which would happen if
    // selected_msg_idx stayed set).
    if app.select_from_compose {
        app.selected_msg_idx = None;
        app.compose_open = true;
        app.select_from_compose = false;
    }
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Starts a (possibly multi-message) delete over the selection. Fires one
/// delete at a time — the serial worker can't take a burst — chaining through
/// [`handle_delete_response`].
pub fn request_delete_selected_message(app: &mut App) {
    let mut ids = selection_target_ids(app, true);
    let total = ids.len();
    if total == 0 {
        app.set_action(ActionState::Error("No deletable message selected".into()));
        return;
    }
    let first = ids.remove(0);
    app.pending_batch = Some(PendingBatch::Delete {
        remaining: ids,
        done: 0,
        total,
    });
    fire_next_delete(app, first, 0, total);
}

fn fire_next_delete(app: &mut App, id: u64, done: usize, total: usize) {
    let Some((_, channel)) = open_channel(app) else {
        app.pending_batch = None;
        return;
    };
    let label = if total > 1 {
        format!("Deleting… {}/{}", done + 1, total)
    } else {
        "Deleting…".to_string()
    };
    app.submit(
        InFlight::DeleteMessage { message_id: id },
        &label,
        WorkerRequest::DeleteMessage {
            channel,
            message_id: id,
        },
    );
}

pub fn handle_delete_response(app: &mut App, result: Result<(), KeybaseError>, message_id: u64) {
    match result {
        Ok(()) => {
            app.push_cmd(
                "keybase chat api delete",
                true,
                format!("msg #{message_id}"),
            );
            // Advance the batch: the next id, or finish.
            let (next, done, total) = match &mut app.pending_batch {
                Some(PendingBatch::Delete {
                    remaining,
                    done,
                    total,
                }) => {
                    *done += 1;
                    (remaining.pop(), *done, *total)
                }
                _ => (None, 1, 1),
            };
            if let Some(id) = next {
                fire_next_delete(app, id, done, total);
                return;
            }
            app.set_action(ActionState::Done(format!(
                "Deleted {total} message{}",
                if total == 1 { "" } else { "s" }
            )));
            finish_selection_batch(app, true);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api delete", false, e.to_string());
            finish_selection_batch(app, false);
        }
    }
}

// ── React ────────────────────────────────────────────────────────────

pub fn open_react_for_selected(app: &mut App) {
    if app.selected_msg_idx.is_none() {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    }
    app.react.clear();
    app.react_selected = 0;
    // Fresh query + possibly new frecency since the last open → refilter.
    app.rebuild_emoji_filter();
    app.screen = crate::tui::screens::Screen::React;
    // The standard set is seeded at construction; lazily fetch the team's
    // custom emojis the first time the picker opens (merged on top), cached
    // for the rest of the session.
    if !app.emojis_loaded {
        request_emojis(app);
    }
}

/// Fetches the emoji catalogue for the reaction picker, once. Runs on the
/// background lane (no `in_flight` ticket) so it never blocks input — the
/// picker is usable as a custom-shortcode entry while it loads. Guarded by
/// `emojis_loading`/`emojis_loaded` so it fires at most once.
pub fn request_emojis(app: &mut App) {
    if app.emojis_loaded || app.emojis_loading {
        return;
    }
    app.emojis_loading = true;
    app.emojis_started = Some(std::time::Instant::now());
    let _ = app.bg_worker_tx.send(WorkerRequest::ListEmojis);
}

pub fn handle_emojis_response(
    app: &mut App,
    result: Result<Vec<crate::domain::Emoji>, KeybaseError>,
) {
    app.emojis_loading = false;
    match result {
        Ok(custom) => {
            // Keybase returns only the team's custom emojis; merge them with
            // the bundled standard set (standard first, custom appended,
            // de-duplicated by alias) so the picker has the everyday glyphs.
            let n = custom.len();
            let mut merged = crate::domain::emoji::standard();
            let mut seen: std::collections::HashSet<String> =
                merged.iter().map(|e| e.alias.clone()).collect();
            for e in custom {
                if seen.insert(e.alias.clone()) {
                    merged.push(e);
                }
            }
            app.emojis = merged;
            app.rebuild_emoji_index();
            // The picker may be open (the fetch is async) — refilter so the
            // merged custom emojis appear without a keystroke.
            app.rebuild_emoji_filter();
            // Reaction chips resolve glyphs through the catalogue.
            app.invalidate_msg_render_cache();
            app.emojis_loaded = true;
            app.push_cmd(
                "keybase chat api emojilist",
                true,
                format!("{n} custom emojis"),
            );
        }
        Err(e) => {
            // Non-fatal: the picker still works as a custom-shortcode entry,
            // and we can retry on the next open (loaded stays false).
            app.push_cmd("keybase chat api emojilist", false, e.to_string());
        }
    }
}

pub fn close_react(app: &mut App) {
    app.react.clear();
    // Same as delete: cancelling a react launched from Compose returns
    // to Compose, not Select mode.
    if app.select_from_compose {
        app.selected_msg_idx = None;
        app.compose_open = true;
        app.select_from_compose = false;
    }
    app.screen = crate::tui::screens::Screen::Inbox;
}

pub fn request_send_reaction(app: &mut App) {
    // Re-sync the cached filter with the query before committing — belt and
    // braces for any path that set the query without a picker keystroke.
    app.rebuild_emoji_filter();
    // Prefer the highlighted emoji from the picker; fall back to the typed
    // text as a literal custom `:shortcode:` when nothing matches the query.
    let filtered = app.filtered_emoji_indices();
    let body = match filtered
        .get(app.react_selected)
        .or_else(|| filtered.first())
    {
        Some(&ei) => {
            let e = &app.emojis[ei];
            // Stock emojis send their raw glyph (works for the whole Unicode
            // set, no shortcode lookup); custom ones must send `:alias:`.
            if e.display.starts_with(':') {
                format!(":{}:", e.alias)
            } else {
                e.display.clone()
            }
        }
        None => app.react.text().trim().to_string(),
    };
    if body.is_empty() {
        app.set_action(ActionState::Error("Reaction is empty".into()));
        return;
    }
    let mut ids = selection_target_ids(app, false);
    let total = ids.len();
    if total == 0 {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    }
    // Close the picker; the batch runs over the conversation.
    app.react.clear();
    app.screen = Screen::Inbox;
    // Frecency bump once for the chosen emoji.
    let alias = body.trim_matches(':').to_string();
    if !alias.is_empty() {
        *app.emoji_uses.entry(alias).or_insert(0) += 1;
    }
    let first = ids.remove(0);
    app.pending_batch = Some(PendingBatch::React {
        body: body.clone(),
        remaining: ids,
        done: 0,
        total,
    });
    fire_next_react(app, first, body, 0, total);
}

fn fire_next_react(app: &mut App, id: u64, body: String, done: usize, total: usize) {
    let Some((_, channel)) = open_channel(app) else {
        app.pending_batch = None;
        return;
    };
    let label = if total > 1 {
        format!("Reacting… {}/{}", done + 1, total)
    } else {
        "Sending reaction…".to_string()
    };
    app.submit(
        InFlight::SendReaction { body: body.clone() },
        &label,
        WorkerRequest::React {
            channel,
            message_id: id,
            body,
        },
    );
}

pub fn handle_react_response(app: &mut App, result: Result<(), KeybaseError>, body: String) {
    match result {
        Ok(()) => {
            app.push_cmd("keybase chat api reaction", true, body.clone());
            let (next, done, total) = match &mut app.pending_batch {
                Some(PendingBatch::React {
                    remaining,
                    done,
                    total,
                    ..
                }) => {
                    *done += 1;
                    (remaining.pop(), *done, *total)
                }
                _ => (None, 1, 1),
            };
            if let Some(id) = next {
                fire_next_react(app, id, body, done, total);
                return;
            }
            app.set_action(ActionState::Done(format!(
                "Reacted to {total} message{}",
                if total == 1 { "" } else { "s" }
            )));
            finish_selection_batch(app, true);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api reaction", false, e.to_string());
            finish_selection_batch(app, false);
        }
    }
}

// ── Pin / unpin ──────────────────────────────────────────────────────

pub fn request_pin_selected_message(app: &mut App) {
    let Some(idx) = app.selected_msg_idx else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg_id) = app.messages.get(idx).map(|m| m.id) else {
        return;
    };
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    app.submit(
        InFlight::PinMessage { message_id: msg_id },
        "Pinning…",
        WorkerRequest::PinMessage {
            channel,
            message_id: msg_id,
        },
    );
}

pub fn handle_pin_response(app: &mut App, result: Result<(), KeybaseError>, message_id: u64) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Pinned msg #{message_id}")));
            app.push_cmd("keybase chat api pin", true, format!("msg #{message_id}"));
            // Stay in Select mode (coherent with delete/react) and reload so
            // `rebuild_pinned` refreshes the 📌 indicator from the new history.
            app.msg_marks.clear();
            app.select_anchor = None;
            app.preserve_msg_scroll = true;
            request_reload_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api pin", false, e.to_string());
        }
    }
}

pub fn request_unpin_conversation(app: &mut App) {
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    app.submit(
        InFlight::UnpinConversation,
        "Unpinning…",
        WorkerRequest::UnpinMessage { channel },
    );
}

pub fn handle_unpin_response(app: &mut App, result: Result<(), KeybaseError>) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done("Pin cleared".into()));
            app.push_cmd("keybase chat api unpin", true, "ok");
            // Reload so the 📌 banner clears from fresh history.
            request_reload_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api unpin", false, e.to_string());
        }
    }
}

// ── Send message ─────────────────────────────────────────────────────

pub fn request_send_message(app: &mut App) {
    let body = app.compose.text().trim().to_string();
    if body.is_empty() {
        app.set_action(ActionState::Error("Message is empty".into()));
        return;
    }
    let Some((conv_id, channel)) = open_channel(app) else {
        return;
    };
    let reply_to = app.reply_to_id;
    let body_bytes_len = body.len();
    // The compose buffer is deliberately NOT cleared here. If the
    // send fails (e.g. transient network blip), the user's draft
    // survives — they hit Enter again. The Ok arm of
    // `handle_send_message_response` clears it on success, matching
    // the synchronous behaviour the sync code had before the worker
    // refactor and aligning with `request_save_edit` /
    // `request_send_reaction`.
    if !app.begin(InFlight::SendMessage {
        body_len: body_bytes_len,
        was_reply: reply_to.is_some(),
    }) {
        return;
    }
    // Optimistic echo: queue the message in the outbox (state Pending) and
    // show it immediately, so it appears the instant Enter is pressed
    // rather than after the send round-trip. Clear the compose now — the
    // body is held safely in the outbox, so a failure keeps it (visible
    // and resendable) without leaving a stale draft behind.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // The draft is being sent — drop any saved copy for this conversation.
    app.drafts.remove(&conv_id);
    app.outbox.push(PendingSend {
        conv_id,
        body: body.clone(),
        reply_to,
        sent_at_ms: now_ms,
        state: SendState::Pending,
    });
    app.compose_clear();
    app.messages_scroll = 0;
    app.set_action(ActionState::Running("Sending…".into()));
    let _ = app.worker_tx.send(WorkerRequest::SendMessage {
        channel,
        body,
        reply_to,
    });
}

pub fn handle_send_message_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    body_len: usize,
    was_reply: bool,
) {
    match result {
        Ok(()) => {
            // The send landed. Flip the in-flight pending to Delivered (it
            // stays on screen, now styled as sent) and re-read to reconcile
            // it with the server's real message; the re-read prunes the
            // Delivered bubble once the real message is in `messages`, so
            // there's no gap. The compose was already cleared at send time.
            if let Some(p) = app
                .outbox
                .iter_mut()
                .find(|p| p.state == SendState::Pending)
            {
                p.state = SendState::Delivered;
            }
            let done = if was_reply {
                "Reply sent".to_string()
            } else {
                "Message sent".to_string()
            };
            app.set_action(ActionState::Done(done));
            app.push_cmd("keybase chat api send", true, format!("{body_len} chars"));
            app.messages_scroll = 0;
            request_load_messages(app);
        }
        Err(e) => {
            // The send failed — mark the in-flight pending Failed so it
            // stays on screen with a resend affordance; its body lives in
            // the outbox, so `Alt+R` can retry it.
            if let Some(p) = app
                .outbox
                .iter_mut()
                .find(|p| p.state == SendState::Pending)
            {
                p.state = SendState::Failed;
            }
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api send", false, e.to_string());
        }
    }
}

/// Resends the oldest failed message in the open conversation. Re-arms it
/// (Failed → Pending) and reuses the normal send path, so the response is
/// handled by [`handle_send_message_response`] like any other send.
pub fn request_resend_message(app: &mut App) {
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(idx) = app
        .outbox
        .iter()
        .position(|p| p.state == SendState::Failed && p.conv_id == conv_id)
    else {
        app.set_action(ActionState::Error("No failed message to resend".into()));
        return;
    };
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    let body = app.outbox[idx].body.clone();
    let reply_to = app.outbox[idx].reply_to;
    let body_len = body.len();
    if !app.begin(InFlight::SendMessage {
        body_len,
        was_reply: reply_to.is_some(),
    }) {
        return;
    }
    app.outbox[idx].state = SendState::Pending;
    app.set_action(ActionState::Running("Resending…".into()));
    let _ = app.worker_tx.send(WorkerRequest::SendMessage {
        channel,
        body,
        reply_to,
    });
}
