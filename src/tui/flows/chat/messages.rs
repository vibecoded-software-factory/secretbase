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
    // The alternate-buffer register for Ctrl+O: remember what was open
    // before this switch (a re-open of the same conversation doesn't count).
    if let Some(cur) = app.open_conv_id.clone()
        && cur != id
    {
        app.prev_conv_id = Some(cur);
    }
    // Record how far the outgoing conversation was read before switching, so its
    // next open can place the `new messages` divider above anything since.
    record_conv_seen(app);
    stash_draft(app);
    app.open_conv_id = Some(id.clone());
    // Seed the unread boundary from what we last saw of *this* conversation this
    // session (None on a first-ever open → no divider).
    app.pagination.unread_boundary = app.conv_last_seen.get(&id).copied();
    app.mentioned.remove(&id); // the mention is about to be seen
    // Optimistically clear the local unread flag when auto-mark-read is on, so
    // the conversation leaves the unread affordances (the attention island, the
    // switcher's Unread section, the tree dot + group count) the *moment* it
    // opens. The server `read` follows and the next inbox list confirms;
    // without this a just-read conversation lingers unread until the periodic
    // resync — which reads as "no matter how much I read it, it comes back".
    if app.settings_cache.auto_mark_read
        && let Some(c) = app.conversations.iter_mut().find(|c| c.id == id)
        && c.unread
    {
        c.unread = false;
        app.rebuild_filter_preserving_cursor();
    }
    app.thread.messages.clear();
    // Reset the per-history projections (pin / headline / id index) so the
    // loading view can't show the previous conversation's pin or topic.
    app.rebuild_msg_meta();
    app.pagination.scroll = 0;
    app.pagination.new_since = 0;
    app.compose_open = true;
    app.edit_target_id = None;
    app.reply_to_id = None;
    app.select.cursor = None;
    app.pending_search_jump = None;
    app.conv_search.query.clear();
    app.conv_search.results.clear();
    app.conv_search.selected = 0;
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
    app.pagination.unread_boundary = None;
    app.pane_zoomed = false;
    app.thread.messages.clear();
    app.rebuild_msg_meta();
    app.pagination.scroll = 0;
    app.pagination.next = None;
    app.pagination.loading_older = false;
    app.compose_clear();
    app.screen = crate::tui::screens::Screen::Inbox;
    app.focus = crate::tui::screens::Focus::Tree;
}

/// Records the highest loaded message id of the open conversation as **seen**
/// (session-local), so the next open can anchor the `new messages` divider above
/// whatever arrived since. No-op when nothing is open / loaded.
fn record_conv_seen(app: &mut App) {
    if let Some(id) = app.open_conv_id.clone()
        && let Some(last) = app.thread.messages.last()
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

/// `Ctrl+N` — opens the **next unread** conversation (recency order,
/// wrapping past the current one), the triage primitive: one key per
/// unread conversation, from anywhere.
pub fn open_next_unread(app: &mut App) {
    // Priority hotlist, weechat-style: conversations with an unseen
    // @mention of you first, then unread DMs, then unread team channels —
    // most recent first within each tier. Repeated Ctrl+N drains the
    // queue in the order the status-strip badges imply.
    let mut unread: Vec<(u8, std::cmp::Reverse<u64>, String)> = app
        .conversations
        .iter()
        .filter(|c| c.member_status == crate::domain::MemberStatus::Active && app.conv_is_unread(c))
        .map(|c| {
            let tier = if app.mentioned.contains(&c.id) {
                0
            } else if c.channel.members_type.is_team() {
                2
            } else {
                1
            };
            (tier, std::cmp::Reverse(c.active_at_ms), c.id.clone())
        })
        .collect();
    if unread.is_empty() {
        app.set_action(ActionState::Done("No unread conversations".into()));
        return;
    }
    unread.sort();
    let pick = match app
        .open_conv_id
        .as_ref()
        .and_then(|cur| unread.iter().position(|(_, _, id)| id == cur))
    {
        // The open one is itself unread → advance past it, wrapping.
        Some(i) => unread[(i + 1) % unread.len()].2.clone(),
        None => unread[0].2.clone(),
    };
    open_conversation_by_id(app, pick);
}

/// `Ctrl+O` — toggles back to the previously open conversation (vim's
/// `Ctrl+^` alternate buffer). Works from a closed chat too (re-opens it).
pub fn open_previous_conversation(app: &mut App) {
    let Some(prev) = app.prev_conv_id.clone() else {
        app.set_action(ActionState::Done("No previous conversation".into()));
        return;
    };
    if app.open_conv_id.as_deref() == Some(prev.as_str()) {
        return;
    }
    if !app.conversations.iter().any(|c| c.id == prev) {
        app.set_action(ActionState::Error(
            "Previous conversation left the inbox".into(),
        ));
        return;
    }
    open_conversation_by_id(app, prev);
}

/// `Alt+E` in the compose — jump straight to editing your **most recent own
/// text message** (the Slack/IRC up-arrow-to-edit fast path): seats the
/// Select cursor on it and opens the edit.
pub fn edit_last_own_message(app: &mut App) {
    let me = app.identity.username.clone();
    if me.is_empty() {
        return;
    }
    let Some(idx) = app.thread.messages.iter().rposition(|m| {
        m.sender == me
            && matches!(
                m.content,
                crate::domain::MessageContent::Text(_) | crate::domain::MessageContent::Edit { .. }
            )
    }) else {
        app.set_action(ActionState::Error("No own message to edit here".into()));
        return;
    };
    app.compose_open = false;
    app.select.cursor = Some(idx);
    open_edit_for_selected(app);
}

// ── Quick switcher (Ctrl+K) ──────────────────────────────────────────

pub fn open_quick_switcher(app: &mut App) {
    app.switcher.from = app.screen;
    app.switcher.query.clear();
    app.switcher.selected = 0;
    app.screen = crate::tui::screens::Screen::QuickSwitcher;
}

pub fn close_quick_switcher(app: &mut App) {
    app.switcher.query.clear();
    app.screen = app.switcher.from;
}

/// Jumps to the highlighted conversation in the switcher.
pub fn quick_switcher_open_selected(app: &mut App) {
    let selectable = app.switcher_selectable();
    let Some(&i) = selectable.get(app.switcher.selected) else {
        close_quick_switcher(app);
        return;
    };
    let id = app.conversations[i].id.clone();
    app.switcher.query.clear();
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
    let loaded = u32::try_from(app.thread.messages.len()).unwrap_or(RELOAD_DEPTH_MAX);
    request_load_messages_num(app, loaded.clamp(MESSAGES_PER_PAGE, RELOAD_DEPTH_MAX));
}

/// Minimum **visible** (post-projection) messages a load should leave on
/// screen before the handlers stop chaining older pages automatically.
const BACKFILL_VISIBLE_FLOOR: usize = MESSAGES_PER_PAGE as usize;
/// Cap on auto-chained older pages per load — bounds the raw fetch at
/// `BACKFILL_MAX_PAGES × MESSAGES_PER_PAGE` slots even in a conversation
/// that is almost entirely folded envelopes.
const BACKFILL_MAX_PAGES: u8 = 8;

/// Chains one more older-page fetch when the projected history is still
/// too thin to scroll/select over (see [`App::backfill_pages`]). Called by
/// both read handlers after they merge + reproject.
fn maybe_backfill(app: &mut App) {
    if app.thread.messages.len() >= BACKFILL_VISIBLE_FLOOR
        || app.pagination.next.is_none()
        || app.pagination.backfill_pages == 0
        || app.pagination.loading_older
    {
        return;
    }
    app.pagination.backfill_pages -= 1;
    app.pagination.loading_older = true;
    request_load_older_messages(app);
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
    app.pagination.next = None;
    app.pagination.loading_older = false;
    app.pagination.backfill_pages = BACKFILL_MAX_PAGES;
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
        app.pagination.loading_older = false;
        return;
    }
    match result {
        Ok((mut msgs, next)) => {
            // Which message the Select-mode cursor sat on, by id — captured
            // before the list is replaced so it can be re-found afterwards.
            let prev_cursor_id = app
                .select
                .cursor
                .and_then(|i| app.thread.messages.get(i))
                .map(|m| m.id);
            msgs.reverse();
            app.thread.messages = project_messages(msgs, app.settings_cache.smart_joins);
            app.rebuild_conv_members(); // add the people who've spoken
            let n = app.thread.messages.len();
            app.pagination.next = next;
            app.pagination.loading_older = false;
            app.rebuild_msg_meta();
            // A control-op re-read (delete/edit/react) keeps the reader where
            // they were; a fresh open/refresh snaps to the latest message.
            if !std::mem::take(&mut app.pagination.preserve_scroll) {
                app.pagination.scroll = 0;
            }
            // Re-anchor Select mode across the reprojection. Marks are ids:
            // prune any whose message is gone (batch- or remote-deleted).
            // The cursor re-seeks its message by id; only when that message
            // no longer exists does it fall back to clamping the old index
            // (or clearing, if the conversation is now empty).
            app.select
                .marks
                .retain(|id| app.thread.msg_index.contains_key(id));
            if let Some(i) = app.select.cursor {
                app.select.cursor = if app.thread.messages.is_empty() {
                    None
                } else {
                    Some(
                        prev_cursor_id
                            .and_then(|id| app.thread.msg_index.get(&id).copied())
                            .unwrap_or_else(|| i.min(app.thread.messages.len() - 1)),
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
            maybe_backfill(app);
            maybe_fetch_pin_body(app);
        }
        Err(e) => {
            app.pagination.loading_older = false;
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
    // Unfurls decorate the message that carried the URL: append them live
    // (they render as a card), but like controls they are not "new
    // content" — no unread badge, no recency bump for a link preview.
    // Giphy cards don't even append: projection drops them (the GIF itself
    // renders inline on the URL message), so a pushed one must not slip in.
    let is_decoration = matches!(&message.content, MessageContent::Unfurl { .. });
    if matches!(&message.content, MessageContent::Unfurl { label } if label == "GIPHY") {
        return;
    }
    let sent_at = message.sent_at;
    let sent_at_ms = message.sent_at_ms;
    let msg_id = message.id;
    let mentions_me = !app.identity.username.is_empty()
        && message
            .mentions
            .iter()
            .any(|m| m.eq_ignore_ascii_case(&app.identity.username));

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
            // A read already in flight will reflect this op (it was issued
            // after the op completed server-side) — re-requesting would just
            // hit the busy guard and spam the log.
            if !matches!(
                app.in_flight,
                Some(crate::tui::worker::InFlight::LoadMessages)
                    | Some(crate::tui::worker::InFlight::LoadOlderMessages)
            ) {
                // A live edit/delete/reaction reprojects in place — don't
                // yank the reader to the bottom, and keep the loaded depth.
                app.pagination.preserve_scroll = true;
                request_reload_messages(app);
            }
        } else if msg_id != 0 && !app.thread.messages.iter().any(|m| m.id == msg_id) {
            app.thread.messages.push(message);
            app.rebuild_msg_meta();
            // If the reader is scrolled up in history, a new arrival lands below
            // the fold — count it for the floating "▼ N new · End" jump cue.
            if app.pagination.scroll > 0 && !from_me {
                app.pagination.new_since += 1;
            } else if app.pagination.scroll == 0 && app.settings_cache.auto_mark_read {
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

    // An @mention of you in a conversation you're not viewing gets the red
    // `@` badge in the tree (cleared when you open it). Session-local — the
    // inbox `list` carries no mention state.
    if !from_me && !viewing && !is_control && !is_decoration && mentions_me {
        app.mentioned.insert(conv_id.clone());
    }

    // 2. Incremental inbox bump (no full re-fetch).
    let known = if let Some(c) = app.conversations.iter_mut().find(|c| c.id == conv_id) {
        if !is_control && !is_decoration && sent_at_ms > c.active_at_ms {
            c.active_at_ms = sent_at_ms;
            c.active_at = sent_at;
        }
        // Mark unread unless it's our own message, we're viewing it, or it's
        // a control event (edit/delete/reaction) or a decoration (unfurl) —
        // neither is new content.
        if !from_me && !viewing && !is_control && !is_decoration {
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
    let Some(cursor) = app.pagination.next.clone() else {
        app.pagination.loading_older = false;
        app.set_action(ActionState::Done("No more history".into()));
        return;
    };
    let Some((_, channel)) = open_channel(app) else {
        // No open conversation, it left the inbox, or its type is
        // unsupported — clear the in-progress flag so the viewport
        // doesn't get stuck showing "loading older…".
        app.pagination.loading_older = false;
        return;
    };
    let peek = !app.settings_cache.auto_mark_read;
    if !app.submit(
        InFlight::LoadOlderMessages,
        "Loading older messages…",
        WorkerRequest::ReadMessages {
            channel,
            num: MESSAGES_PER_PAGE,
            peek,
            next_cursor: Some(cursor),
        },
    ) {
        // Refused (another request in flight): clear the in-progress flag or
        // the wheel's auto-pagination stays dead for the whole session.
        app.pagination.loading_older = false;
    }
}

pub fn handle_load_older_messages_response(
    app: &mut App,
    result: Result<(Vec<Message>, Option<String>), KeybaseError>,
) {
    // Drop the orphan if the conversation was closed mid-flight.
    if app.open_conv_id.is_none() {
        app.pagination.loading_older = false;
        return;
    }
    match result {
        Ok((mut older, next)) => {
            older.reverse();
            older = project_messages(older, app.settings_cache.smart_joins);
            let n = older.len();
            older.extend(std::mem::take(&mut app.thread.messages));
            app.thread.messages = older;
            app.pagination.next = next;
            app.pagination.loading_older = false;
            // The Select-mode cursor and the `v` anchor are **indices** (the
            // marks are ids) — prepending n rows shifts every index, so both
            // must shift with them to stay on the same messages.
            if let Some(i) = app.select.cursor {
                app.select.cursor = Some(i + n);
            }
            if let Some(a) = app.select.anchor {
                app.select.anchor = Some(a + n);
            }
            app.rebuild_msg_meta_after_prepend();
            app.set_action(ActionState::Done(format!("Loaded {n} older messages")));
            app.push_cmd(
                "keybase chat api read (older)",
                true,
                format!("{n} messages"),
            );
            try_jump_to_search_target(app);
            maybe_backfill(app);
            maybe_fetch_pin_body(app);
        }
        Err(e) => {
            app.pagination.loading_older = false;
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
    } else if !app.thread.messages.is_empty() {
        enter_select_mode(app);
    } else {
        close_conversation(app);
    }
}

// ── Message selection (Compose ↔ Select modes) ──────────────────────

pub fn enter_select_mode(app: &mut App) {
    if app.thread.messages.is_empty() {
        app.set_action(ActionState::Error("No messages to select".into()));
        return;
    }
    app.compose_open = false;
    app.select.from_compose = false;
    app.select.cursor = Some(app.thread.messages.len() - 1);
    app.select.marks.clear();
    app.select.anchor = None;
}

pub fn leave_select_mode(app: &mut App) {
    app.select.cursor = None;
    app.select.from_compose = false;
    app.compose_open = true;
    app.select.marks.clear();
    app.select.anchor = None;
}

/// Toggles the mark on the cursor message (manual one-by-one multi-select).
pub fn msg_toggle_mark(app: &mut App) {
    let Some(id) = app
        .select
        .cursor
        .and_then(|i| app.thread.messages.get(i))
        .map(|m| m.id)
    else {
        return;
    };
    app.select.anchor = None;
    if !app.select.marks.remove(&id) {
        app.select.marks.insert(id);
    }
}

/// Extends a contiguous shaded selection by `delta` (Shift+↑/↓), editor-style:
/// the anchor stays put while the cursor moves and the whole range is marked.
pub fn select_extend(app: &mut App, delta: isize) {
    let Some(cur) = app.select.cursor else {
        return;
    };
    let max = app.thread.messages.len().saturating_sub(1);
    let anchor = *app.select.anchor.get_or_insert(cur);
    let new = (cur as isize + delta).clamp(0, max as isize) as usize;
    app.select.cursor = Some(new);
    let (lo, hi) = (anchor.min(new), anchor.max(new));
    app.select.marks = (lo..=hi)
        .filter_map(|i| app.thread.messages.get(i))
        .map(|m| m.id)
        .collect();
}

/// Copies the selected messages (marked, or the cursor message) to the
/// clipboard. `full` includes the author + timestamp above each body;
/// otherwise just the bodies, one per line. Each message is separated by a
/// blank line in full mode, a single newline in content mode.
pub fn do_copy_messages(app: &mut App, full: bool) {
    let mut idxs: Vec<usize> = if app.select.marks.is_empty() {
        app.select.cursor.into_iter().collect()
    } else {
        let mut v: Vec<usize> = app
            .select
            .marks
            .iter()
            .filter_map(|id| app.thread.msg_index.get(id).copied())
            .collect();
        v.sort_unstable();
        v
    };
    idxs.retain(|&i| i < app.thread.messages.len());
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
        let m = &app.thread.messages[i];
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
    let Some(idx) = app.select.cursor else {
        return Vec::new();
    };
    let body = match app.thread.messages.get(idx).map(|m| &m.content) {
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
/// Inserts the chosen `:emoji:` autocomplete row into the compose at the
/// token: stock emojis insert their glyph (what the body renders anyway),
/// custom team emojis insert `:alias:` (the only form Keybase accepts).
pub fn accept_emoji_ac(app: &mut App, catalogue_idx: usize) {
    let Some((start, _)) =
        crate::domain::active_emoji_token(app.compose.text(), app.compose.cursor())
    else {
        return;
    };
    let Some(e) = app.emoji.all.get(catalogue_idx) else {
        return;
    };
    let insert = if e.display.starts_with(':') {
        format!(":{}: ", e.alias)
    } else {
        format!("{} ", e.display)
    };
    while app.compose.cursor() > start {
        app.compose.backspace();
    }
    app.compose.insert_str(&insert);
    app.emoji_ac_selected = 0;
}

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

/// Enter (or click-again) on the Select-mode cursor: a **reply** jumps to
/// the message it quotes — the search-jump machinery paginates older pages
/// if the parent is outside the loaded window — anything else returns to
/// Compose (Enter's historical exit, kept for non-reply messages).
pub fn select_activate(app: &mut App) {
    let target = app
        .select
        .cursor
        .and_then(|i| app.thread.messages.get(i))
        .and_then(|m| m.reply_to);
    match target {
        Some(t) => {
            app.pending_search_jump = Some(t);
            try_jump_to_search_target(app);
        }
        None => leave_select_mode(app),
    }
}

/// `Alt+N`: jump to the first message after the `new messages` divider
/// (the session's last-seen boundary). Enters Select mode on it — the
/// render then scrolls it into view. Honest fallbacks when there's no
/// boundary or nothing newer.
pub fn jump_to_new_messages(app: &mut App) {
    let Some(boundary) = app.pagination.unread_boundary else {
        app.set_action(ActionState::Done("No new-messages marker here".into()));
        return;
    };
    let Some(idx) = app.thread.messages.iter().position(|m| m.id > boundary) else {
        app.set_action(ActionState::Done(
            "Nothing newer than your last visit".into(),
        ));
        return;
    };
    app.compose_open = false;
    app.select.cursor = Some(idx);
    select_resync_anchor_marks(app);
    app.set_action(ActionState::Done("Jumped to new messages".into()));
}

/// `[` / `]` in Select mode: previous / next message that **@mentions me**
/// in the loaded history — the messages you actually owe a response to.
pub fn select_jump_mention(app: &mut App, dir: isize) {
    let me = app.identity.username.clone();
    if me.is_empty() {
        return;
    }
    let mentions_me = |m: &Message| m.mentions.iter().any(|u| u.eq_ignore_ascii_case(&me));
    let cur = app.select.cursor.unwrap_or(0);
    let found = if dir < 0 {
        app.thread.messages[..cur].iter().rposition(&mentions_me)
    } else {
        app.thread.messages[cur + 1..]
            .iter()
            .position(mentions_me)
            .map(|i| cur + 1 + i)
    };
    match found {
        Some(i) => {
            app.select.cursor = Some(i);
            select_resync_anchor_marks(app);
        }
        None => app.set_action(ActionState::Done("No more mentions of you".into())),
    }
}

/// `w` in Select mode: **who reacted** — lists each reaction with its
/// senders on the feedback strip (the chips only show counts; the GUI
/// shows names on hover, which a terminal doesn't have).
pub fn show_reactors(app: &mut App) {
    let Some(m) = app.select.cursor.and_then(|i| app.thread.messages.get(i)) else {
        return;
    };
    if m.reactions.is_empty() {
        app.set_action(ActionState::Done("No reactions on this message".into()));
        return;
    }
    let listing = m
        .reactions
        .iter()
        .map(|r| format!("{} {}", r.emoji, r.usernames.join(", ")))
        .collect::<Vec<_>>()
        .join(" · ");
    app.set_action(ActionState::Done(listing));
}

pub fn select_move_up(app: &mut App) {
    match app.select.cursor {
        // At the top of the loaded window — pull an older page instead of
        // going dead: Select mode was the one surface that couldn't reach
        // older history. The prepend handler shifts the cursor index so it
        // stays on the same message; the next `k` then moves into the
        // newly-loaded page.
        Some(0) if app.pagination.next.is_some() && !app.pagination.loading_older => {
            app.pagination.loading_older = true;
            request_load_older_messages(app);
        }
        Some(0) => {}
        Some(i) => {
            app.select.cursor = Some(i - 1);
            select_resync_anchor_marks(app);
        }
        None => {}
    }
}

pub fn select_move_down(app: &mut App) {
    let max = app.thread.messages.len().saturating_sub(1);
    if let Some(i) = app.select.cursor
        && i < max
    {
        app.select.cursor = Some(i + 1);
        select_resync_anchor_marks(app);
    }
}

/// Re-marks the anchored range after a Select-mode cursor move: while a `v`
/// anchor is set, **every motion extends the visual range** (vim's visual
/// mode) — plain `j`/`k`, paging, `g`/`G`, `{`/`}`. Without an anchor this
/// is a no-op, so Space-marked scatter selections are untouched by motion.
pub(crate) fn select_resync_anchor_marks(app: &mut App) {
    let (Some(anchor), Some(cur)) = (app.select.anchor, app.select.cursor) else {
        return;
    };
    let (lo, hi) = (anchor.min(cur), anchor.max(cur));
    app.select.marks = (lo..=hi)
        .filter_map(|i| app.thread.messages.get(i))
        .map(|m| m.id)
        .collect();
}

/// `v` — toggle the visual anchor at the cursor. On: the cursor row is
/// marked and every motion extends the range. Off: anchor + marks clear
/// (vim's `v` exit). The Alt+Shift range chords remain as aliases that
/// implicitly anchor.
pub fn select_toggle_anchor(app: &mut App) {
    if app.select.anchor.take().is_some() {
        app.select.marks.clear();
    } else if app.select.cursor.is_some() {
        app.select.anchor = app.select.cursor;
        select_resync_anchor_marks(app);
    }
}

/// `{` / `}` — jump to the head of the previous / next **speaker run**
/// (consecutive messages from one sender), vim's paragraph motion mapped
/// onto the chat stream. Extends the range while anchored.
pub fn select_jump_run(app: &mut App, dir: isize) {
    let Some(cur) = app.select.cursor else {
        return;
    };
    let n = app.thread.messages.len();
    if n == 0 {
        return;
    }
    let sender_of = |i: usize| app.thread.messages[i].sender.clone();
    let run_head = |mut i: usize| {
        while i > 0 && app.thread.messages[i - 1].sender == app.thread.messages[i].sender {
            i -= 1;
        }
        i
    };
    let new = if dir < 0 {
        let h = run_head(cur);
        if h < cur {
            h
        } else if h == 0 {
            0
        } else {
            run_head(h - 1)
        }
    } else {
        let mut i = cur;
        let s = sender_of(cur);
        while i + 1 < n && app.thread.messages[i + 1].sender == s {
            i += 1;
        }
        (i + 1).min(n - 1)
    };
    app.select.cursor = Some(new);
    select_resync_anchor_marks(app);
}

// ── Edit ─────────────────────────────────────────────────────────────

pub fn open_edit_for_selected(app: &mut App) {
    let Some(idx) = app.select.cursor else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.thread.messages.get(idx) else {
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
    app.select.cursor = None;
    app.select.marks.clear(); // editing is single-message; drop any shading
    app.select.anchor = None;
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
            app.pagination.preserve_scroll = true; // stay where the reader was
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
    let Some(idx) = app.select.cursor else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.thread.messages.get(idx) else {
        return;
    };
    app.edit_target_id = None;
    app.reply_to_id = Some(msg.id);
    app.compose.clear();
    app.compose_open = true;
    app.select.cursor = None;
    app.select.marks.clear(); // replying is single-message; drop any shading
    app.select.anchor = None;
}

// ── Delete ───────────────────────────────────────────────────────────

// ── Multi-select batch actions (delete / react over `select.marks`) ─────

/// The message ids the current Select-mode action targets: every **marked**
/// message (by id — stable across the reload), or the cursor message when
/// nothing is marked. `own_only` keeps just the local user's messages (delete).
fn selection_target_ids(app: &App, own_only: bool) -> Vec<u64> {
    let me = &app.identity.username;
    let keep = |m: &&crate::domain::Message| !own_only || &m.sender == me;
    let mut ids: Vec<u64> = if !app.select.marks.is_empty() {
        let mut v: Vec<u64> = app
            .select
            .marks
            .iter()
            .filter_map(|id| {
                app.thread
                    .msg_index
                    .get(id)
                    .and_then(|&i| app.thread.messages.get(i))
            })
            .filter(keep)
            .map(|m| m.id)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    } else {
        app.select
            .cursor
            .and_then(|i| app.thread.messages.get(i))
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
    app.select.batch = None;
    app.select.marks.clear();
    app.select.anchor = None;
    if reload {
        app.pagination.preserve_scroll = true; // stay put; stay in Select mode
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
    // select.cursor stayed set).
    if app.select.from_compose {
        app.select.cursor = None;
        app.compose_open = true;
        app.select.from_compose = false;
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
    app.select.batch = Some(PendingBatch::Delete {
        remaining: ids,
        done: 0,
        total,
    });
    fire_next_delete(app, first, 0, total);
}

fn fire_next_delete(app: &mut App, id: u64, done: usize, total: usize) {
    let Some((_, channel)) = open_channel(app) else {
        app.select.batch = None;
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
            let (next, done, total) = match &mut app.select.batch {
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
    if app.select.cursor.is_none() {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    }
    app.react_to_compose = false;
    app.react.clear();
    app.react_selected = 0;
    // Fresh query + possibly new frecency since the last open → refilter.
    app.rebuild_emoji_filter();
    app.screen = crate::tui::screens::Screen::React;
    // The standard set is seeded at construction; lazily fetch the team's
    // custom emojis the first time the picker opens (merged on top), cached
    // for the rest of the session.
    if !app.emoji.loaded {
        request_emojis(app);
    }
}

/// `copy` menu action — author + timestamp + body of the selected message(s).
fn copy_selected_message(app: &mut App) {
    do_copy_messages(app, true);
}

/// One per-message action menu row: its label + the handler it runs.
pub type MessageAction = (&'static str, fn(&mut App));

/// The per-message action menu rows, in order: label + the handler it runs
/// (each targets the message under the Select cursor). Both the menu view and
/// its activation read this array, so the labels and actions can't drift.
pub const MESSAGE_ACTIONS: [MessageAction; 5] = [
    ("react", open_react_for_selected),
    ("reply", start_reply_for_selected),
    ("edit", open_edit_for_selected),
    ("delete", open_delete_for_selected),
    ("copy", copy_selected_message),
];

/// Opens the per-message action menu (`Screen::MessageActions`) for the message
/// at `idx` — right-click. Seats the Select cursor on it first so the actions
/// target the right message.
pub fn open_message_actions(app: &mut App, idx: usize) {
    if app.open_conv_id.is_none() || idx >= app.thread.messages.len() {
        return;
    }
    app.select.cursor = Some(idx);
    app.msg_actions_selected = 0;
    app.screen = crate::tui::screens::Screen::MessageActions;
}

/// Closes the menu back to the open conversation without running an action.
pub fn close_message_actions(app: &mut App) {
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Runs the highlighted menu action: closes the menu first, then dispatches
/// (the action may open its own overlay — react / delete-confirm — or mutate
/// compose — reply / edit).
pub fn run_message_action(app: &mut App) {
    let idx = app.msg_actions_selected.min(MESSAGE_ACTIONS.len() - 1);
    let action = MESSAGE_ACTIONS[idx].1;
    close_message_actions(app);
    action(app);
}

/// Opens the emoji picker in **insert** mode: the chosen emoji lands in the
/// compose draft at the cursor (the compose bar's `emoji` chip / `Alt+I`),
/// instead of reacting to a message. Same picker, same catalogue.
pub fn open_emoji_for_compose(app: &mut App) {
    if app.open_conv_id.is_none() {
        return;
    }
    app.react_to_compose = true;
    app.react.clear();
    app.react_selected = 0;
    app.rebuild_emoji_filter();
    app.screen = crate::tui::screens::Screen::React;
    if !app.emoji.loaded {
        request_emojis(app);
    }
}

/// Fetches the emoji catalogue for the reaction picker, once. Runs on the
/// background lane (no `in_flight` ticket) so it never blocks input — the
/// picker is usable as a custom-shortcode entry while it loads. Guarded by
/// `emojis_loading`/`emojis_loaded` so it fires at most once.
pub fn request_emojis(app: &mut App) {
    if app.emoji.loaded || app.emoji.loading {
        return;
    }
    app.emoji.loading = true;
    app.emojis_started = Some(std::time::Instant::now());
    let _ = app.bg_worker_tx.send(WorkerRequest::ListEmojis);
}

pub fn handle_emojis_response(
    app: &mut App,
    result: Result<Vec<crate::domain::Emoji>, KeybaseError>,
) {
    app.emoji.loading = false;
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
            app.emoji.set(merged);
            // The picker may be open (the fetch is async) — refilter so the
            // merged custom emojis appear without a keystroke.
            app.rebuild_emoji_filter();
            // Reaction chips resolve glyphs through the catalogue.
            app.invalidate_msg_render_cache();
            app.emoji.loaded = true;
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
    app.react_to_compose = false;
    app.react.clear();
    // Same as delete: cancelling a react launched from Compose returns
    // to Compose, not Select mode.
    if app.select.from_compose {
        app.select.cursor = None;
        app.compose_open = true;
        app.select.from_compose = false;
    }
    app.screen = crate::tui::screens::Screen::Inbox;
}

pub fn request_send_reaction(app: &mut App) {
    // Re-sync the cached filter with the query before committing — belt and
    // braces for any path that set the query without a picker keystroke.
    app.rebuild_emoji_filter();
    // Prefer the highlighted emoji from the picker; fall back to the typed
    // text as a literal custom `:shortcode:` when nothing matches the query.
    let filtered = app.emoji.filtered();
    let body = match filtered
        .get(app.react_selected)
        .or_else(|| filtered.first())
    {
        Some(&ei) => {
            let e = &app.emoji.all[ei];
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
    // Insert mode: the emoji goes into the draft, nothing is posted.
    if app.react_to_compose {
        app.compose.insert_str(&body);
        close_react(app);
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
        app.emoji.bump_use(&alias);
    }
    let first = ids.remove(0);
    app.select.batch = Some(PendingBatch::React {
        body: body.clone(),
        remaining: ids,
        done: 0,
        total,
    });
    fire_next_react(app, first, body, 0, total);
}

fn fire_next_react(app: &mut App, id: u64, body: String, done: usize, total: usize) {
    let Some((_, channel)) = open_channel(app) else {
        app.select.batch = None;
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
            let (next, done, total) = match &mut app.select.batch {
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
    let Some(idx) = app.select.cursor else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg_id) = app.thread.messages.get(idx).map(|m| m.id) else {
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
            // Remember the target locally: the JSON API strips the pin
            // payload from reads, so this session-local record is the only
            // way the 📌 header can point at the *specific* message.
            if let Some(conv) = app.open_conv_id.clone() {
                app.set_local_pin(conv, Some(message_id));
            }
            // Stay in Select mode (coherent with delete/react) and reload so
            // `rebuild_msg_meta` refreshes the 📌 indicator from the new history.
            app.select.marks.clear();
            app.select.anchor = None;
            app.pagination.preserve_scroll = true;
            request_reload_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api pin", false, e.to_string());
        }
    }
}

/// Hides the pin banner **locally** (the GUI-parity ✕): records the pin
/// envelope id so this pin stays hidden across reloads and restarts, posts
/// nothing, unpins for no one. A newer pin revives the banner. `Alt+U`
/// (a real unpin) remains the loud sibling.
pub fn dismiss_pin_banner(app: &mut App) {
    let (Some(conv), Some(env)) = (app.open_conv_id.clone(), app.pins.envelope_id) else {
        app.set_action(ActionState::Error("No pin banner to hide".into()));
        return;
    };
    app.set_pin_dismissed(conv, env);
    app.rebuild_msg_meta();
    app.set_action(ActionState::Done("Pin hidden (local only)".into()));
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
            if let Some(conv) = app.open_conv_id.clone() {
                app.set_local_pin(conv, None);
            }
            // Reload so the 📌 banner clears from fresh history.
            request_reload_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api unpin", false, e.to_string());
        }
    }
}

/// Fires a background `{"method":"get"}` for the pinned message when its
/// target is known (the persisted local pin record) but **older than the
/// loaded window** — so the 📌 header can show the real body instead of
/// just `#id`. Fire-and-forget on the background lane (routed by variant,
/// no `InFlight` ticket); one attempt per `(conv, target)` so a failing
/// fetch can't loop on every reload.
pub(crate) fn maybe_fetch_pin_body(app: &mut App) {
    if !app.pins.present {
        return;
    }
    let Some(pid) = app.pins.msg_id else {
        return;
    };
    if app.thread.msg_index.contains_key(&pid) {
        return; // in the loaded window — the header reads it directly
    }
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    if app.pins.bodies.get(&conv_id).is_some_and(|m| m.id == pid) {
        return; // already fetched
    }
    if !app.pins.fetch_attempted.insert((conv_id.clone(), pid)) {
        return;
    }
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    let Ok(channel) = read_channel_from_conv(conv) else {
        return; // unsupported members_type — silent, it's a background nicety
    };
    let _ = app.bg_worker_tx.send(WorkerRequest::GetPinnedMessage {
        conv_id,
        channel,
        message_id: pid,
    });
}

pub fn handle_get_pinned_message_response(
    app: &mut App,
    conv_id: String,
    message_id: u64,
    result: Result<Option<Message>, KeybaseError>,
) {
    match result {
        Ok(Some(m)) if m.id == message_id => {
            app.push_cmd(
                "keybase chat api get (pin)",
                true,
                format!("msg #{message_id}"),
            );
            app.pins.bodies.insert(conv_id, m);
        }
        // Deleted / not returned: leave the honest `#id` fallback. The
        // attempted-marker stays, so this target isn't re-fetched.
        Ok(_) => app.push_cmd("keybase chat api get (pin)", false, "message not returned"),
        Err(e) => app.push_cmd("keybase chat api get (pin)", false, e.to_string()),
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
    app.pagination.scroll = 0;
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
            app.pagination.scroll = 0;
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
    // Oldest failed first — repeated Alt+R walks the queue in order, and
    // the toast says *which* message went out and how many still wait, so
    // multiple failures are never a lottery.
    let failed: Vec<usize> = app
        .outbox
        .iter()
        .enumerate()
        .filter(|(_, p)| p.state == SendState::Failed && p.conv_id == conv_id)
        .map(|(i, _)| i)
        .collect();
    let Some(&idx) = failed.first() else {
        app.set_action(ActionState::Error("No failed message to resend".into()));
        return;
    };
    let remaining = failed.len() - 1;
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
    let snippet: String = body.lines().next().unwrap_or("").chars().take(24).collect();
    app.set_action(ActionState::Running(if remaining > 0 {
        format!("Resending “{snippet}…” ({remaining} more failed)")
    } else {
        format!("Resending “{snippet}…”")
    }));
    let _ = app.worker_tx.send(WorkerRequest::SendMessage {
        channel,
        body,
        reply_to,
    });
}
