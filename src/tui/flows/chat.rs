//! Chat / inbox flows.
//!
//! Every operation that talks to the `keybase` CLI is split into two
//! functions:
//!
//! 1. `request_*` — validates state, captures parameters from the
//!    app, sets [`InFlight`], and pushes a [`WorkerRequest`] to the
//!    worker thread. The render thread continues immediately.
//! 2. `handle_*_response` — applies the result returned by the
//!    worker. Called from
//!    [`crate::tui::flows::apply_response`] when the matching
//!    response arrives.
//!
//! Pure UI-side helpers (filter cycling, search query edits, popup
//! lifecycle) stay synchronous since they never call the port.

use crate::domain::{InboxHit, Message};
use crate::ports::KeybaseError;
use crate::ports::keybase::{ListConversationsOk, ReadChannel};
use crate::tui::action::ActionState;
use crate::tui::app::{App, ConvAction};
use crate::tui::app::{PendingSend, SendState};
use crate::tui::worker::{InFlight, WorkerRequest};

/// Number of messages fetched per page. Sized so most chats fit
/// into a single page on first open while keeping the per-page
/// decode cost low when the user paginates back through history.
pub const MESSAGES_PER_PAGE: u32 = 50;

// ── Inbox load ────────────────────────────────────────────────────────

/// Queues a refresh of the inbox list (`keybase chat api list`).
pub fn request_load_inbox(app: &mut App) {
    if !app.begin(InFlight::LoadInbox) {
        return;
    }
    app.set_action(ActionState::Running("Refreshing inbox…".into()));
    let _ = app.worker_tx.send(WorkerRequest::ListConversations);
}

/// Variant of [`request_load_inbox`] used by the auto-refresh hook.
///
/// "Silent" means **silent on success, loud on failure**:
///
/// * Success: the feedback strip stays Idle — no "Loaded N
///   conversations" banner so an idle user reading the inbox is
///   not distracted by a periodic flash.
/// * Failure: the error still surfaces on the feedback strip.
///   Hiding network blips would let a stale inbox quietly accumulate
///   while the user wondered why no new messages were arriving.
///
/// Either way the call lands on the command-log panel so the user
/// can reconstruct the refresh history if they want detail.
pub fn request_load_inbox_silent(app: &mut App) {
    // Runs on the BACKGROUND lane (its own worker + `keybase` process)
    // so it never occupies the user's `in_flight` slot — otherwise
    // `busy_blocks` would silently swallow every keypress during the
    // refresh. Guarded by `bg_inflight`, never by `begin`/`in_flight`.
    if app.bg_inflight {
        return;
    }
    app.bg_inflight = true;
    app.bg_started = Some(std::time::Instant::now());
    // Deliberately do not set Running — see the contract above.
    let _ = app
        .bg_worker_tx
        .send(WorkerRequest::ListConversationsSilent);
}

pub fn handle_load_inbox_response(
    app: &mut App,
    result: Result<ListConversationsOk, KeybaseError>,
    silent: bool,
) {
    match result {
        Ok(load) => {
            // Remember which conversation the cursor was on so we can
            // restore it by id (the list re-sorts by recency, so the
            // positional index would point at a different conversation
            // after a refresh).
            let prev_selected_id = app.selected_conversation().map(|c| c.id.clone());
            let n = load.conversations.len();
            let skipped_count = load.skipped.len();
            app.conversations = load.conversations;
            app.rebuild_lowered();
            app.rebuild_filter();
            // Restore selection by id; fall back to clamping to a valid
            // row if that conversation is gone or there was none.
            let restored = prev_selected_id.as_deref().and_then(|id| {
                app.filtered_cache
                    .iter()
                    .position(|&i| app.conversations[i].id == id)
            });
            match restored {
                Some(pos) => app.list_selected = pos,
                None => {
                    if app.list_selected >= app.filtered_cache.len() {
                        app.list_selected = 0;
                        app.list_scroll = 0;
                    }
                }
            }
            app.last_inbox_load = std::time::Instant::now();
            let summary = if skipped_count == 0 {
                format!("{n} conversations")
            } else {
                format!("{n} conversations ({skipped_count} skipped)")
            };
            if !silent {
                app.set_action(ActionState::Done(format!("Loaded {summary}")));
            } else if matches!(app.action_state, ActionState::Running(_)) {
                app.set_action(ActionState::Idle);
            }
            // Top-level row is always "ok" because the inbox loaded;
            // per-row warnings go in their own log entries so the
            // user sees the exact decode error.
            app.push_cmd("keybase chat api list", true, summary);
            for diag in load.skipped {
                app.push_cmd("conversation parse warning", false, diag);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api list", false, e.to_string());
        }
    }
}

// ── Mark read ────────────────────────────────────────────────────────

/// Queues a mark-as-read for the currently selected conversation.
pub fn request_mark_read(app: &mut App) {
    // Drain everything we need from `conv` first — including the
    // channel-mapping result — so the immutable borrow on `app`
    // ends before we touch `app` mutably below.
    let (channel_result, this_conv_id) = {
        let Some(conv) = app.selected_conversation() else {
            app.set_action(ActionState::Error("No conversation selected".into()));
            return;
        };
        (read_channel_from_conv(conv), conv.id.clone())
    };
    let upto = match app.open_conv_id.as_deref() {
        Some(id) if id == this_conv_id => app.messages.last().map(|m| m.id).unwrap_or(0),
        _ => 0,
    };
    let conv_idx = app.filtered_cache[app.list_selected];
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::MarkRead { conv_idx }) {
        return;
    }
    app.set_action(ActionState::Running("Marking as read…".into()));
    let _ = app.worker_tx.send(WorkerRequest::MarkRead {
        channel,
        message_id: upto,
    });
}

pub fn handle_mark_read_response(app: &mut App, result: Result<(), KeybaseError>, conv_idx: usize) {
    match result {
        Ok(()) => {
            if let Some(c) = app.conversations.get_mut(conv_idx) {
                c.unread = false;
            }
            // Refresh the lowered cache + per-filter counts so the
            // sidebar "Unread N" and the Unread filter membership stay
            // consistent with the flipped flag, then rebuild the view.
            app.rebuild_lowered();
            app.rebuild_filter();
            app.set_action(ActionState::Done("Marked as read".into()));
            app.push_cmd("keybase chat api mark", true, "ok");
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api mark", false, e.to_string());
        }
    }
}

// ── Channel translation ──────────────────────────────────────────────

/// Error message returned by [`read_channel_from_conv`] when the
/// conversation carries an unrecognised `members_type`.
///
/// `Unknown` is produced by `#[serde(other)]` on
/// [`crate::domain::MembersType`] whenever Keybase emits a value the
/// domain enum hasn't been taught yet (e.g. a new conversation kind
/// added in a future release). Previously we silently fell back to
/// `"impteamnative"`, which would route writes to a DM with the
/// literal name string as participant — potentially creating bogus
/// conversations. The safer behaviour is to refuse the operation
/// and ask the user to update.
pub const UNKNOWN_MEMBERS_TYPE_ERR: &str = "Unsupported conversation type — update secretbase";

/// Translates a [`Conversation`] into a [`ReadChannel`] for the
/// adapter, returning `Err` if the conversation's `members_type` is
/// unrecognised. Callers must surface the error to the user instead
/// of silently routing to the wrong channel kind.
pub fn read_channel_from_conv(
    conv: &crate::domain::Conversation,
) -> Result<ReadChannel, &'static str> {
    let members_type = match conv.channel.members_type {
        crate::domain::MembersType::ImpTeamNative => "impteamnative",
        crate::domain::MembersType::ImpTeamUpgrade => "impteamupgrade",
        crate::domain::MembersType::Team => "team",
        crate::domain::MembersType::Kbfs => "kbfs",
        crate::domain::MembersType::Unknown => return Err(UNKNOWN_MEMBERS_TYPE_ERR),
    }
    .to_string();
    Ok(ReadChannel {
        name: conv.channel.name.clone(),
        members_type,
        topic_name: conv.channel.topic_name.clone(),
    })
}

/// Surface an `Err` from [`read_channel_from_conv`] to the user via
/// the feedback strip + command log, returning `None` so the caller
/// can short-circuit with `let Some(channel) = … else { return; };`.
///
/// Takes the [`Result`] (not the conversation) so the call site can
/// build it first — the conversation reference is borrowed from
/// `app`, so the borrow checker forbids holding it while we mutate
/// `app` for the error surfaces.
fn resolve_channel_or_fail(
    app: &mut App,
    result: Result<ReadChannel, &'static str>,
) -> Option<ReadChannel> {
    match result {
        Ok(channel) => Some(channel),
        Err(msg) => {
            app.set_action(ActionState::Error(msg.to_string()));
            app.push_cmd("channel mapping", false, msg);
            None
        }
    }
}

// ── Filter / search query helpers (pure, sync) ───────────────────────

/// Cycles `app.active_filter` forward by one and refreshes the cache.
pub fn cycle_filter_next(app: &mut App) {
    use crate::domain::CONVERSATION_FILTERS;
    let cur = CONVERSATION_FILTERS
        .iter()
        .position(|f| *f == app.active_filter)
        .unwrap_or(0);
    let next = (cur + 1) % CONVERSATION_FILTERS.len();
    app.active_filter = CONVERSATION_FILTERS[next];
    app.list_selected = 0;
    app.list_scroll = 0;
    app.rebuild_filter();
}

pub fn cycle_filter_prev(app: &mut App) {
    use crate::domain::CONVERSATION_FILTERS;
    let cur = CONVERSATION_FILTERS
        .iter()
        .position(|f| *f == app.active_filter)
        .unwrap_or(0);
    let prev = (cur + CONVERSATION_FILTERS.len() - 1) % CONVERSATION_FILTERS.len();
    app.active_filter = CONVERSATION_FILTERS[prev];
    app.list_selected = 0;
    app.list_scroll = 0;
    app.rebuild_filter();
}

pub fn search_push(app: &mut App, c: char) {
    app.search.insert(c);
    app.list_selected = 0;
    app.list_scroll = 0;
    app.rebuild_filter();
}

pub fn search_pop(app: &mut App) {
    app.search.backspace();
    app.list_selected = 0;
    app.list_scroll = 0;
    app.rebuild_filter();
}

pub fn search_clear(app: &mut App) {
    if !app.search.is_empty() {
        app.search.clear();
        app.list_selected = 0;
        app.list_scroll = 0;
        app.rebuild_filter();
    }
}

// ── Open / close conversation detail screen ──────────────────────────

pub fn open_selected_conversation(app: &mut App) {
    let Some(conv) = app.selected_conversation() else {
        app.set_action(ActionState::Error("No conversation selected".into()));
        return;
    };
    app.open_conv_id = Some(conv.id.clone());
    app.messages.clear();
    app.messages_scroll = 0;
    app.compose_open = true;
    app.compose_clear();
    app.screen = crate::tui::screens::Screen::Conversation;
    request_load_messages(app);
}

pub fn close_conversation(app: &mut App) {
    app.open_conv_id = None;
    app.messages.clear();
    app.messages_scroll = 0;
    app.messages_next = None;
    app.messages_loading_older = false;
    app.screen = crate::tui::screens::Screen::Inbox;
}

// ── Load messages (first page) ───────────────────────────────────────

pub fn request_load_messages(app: &mut App) {
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
    if !app.begin(InFlight::LoadMessages) {
        return;
    }
    app.set_action(ActionState::Running("Loading messages…".into()));
    let _ = app.worker_tx.send(WorkerRequest::ReadMessages {
        channel,
        num: MESSAGES_PER_PAGE,
        peek,
        next_cursor: None,
    });
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
            msgs.reverse();
            let n = msgs.len();
            app.messages = msgs;
            app.messages_next = next;
            app.messages_loading_older = false;
            app.rebuild_pinned();
            app.messages_scroll = 0;
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
                    app.rebuild_lowered();
                    app.rebuild_filter();
                }
            }
            app.set_action(ActionState::Done(format!("Loaded {n} messages")));
            app.push_cmd("keybase chat api read", true, format!("{n} messages"));
        }
        Err(e) => {
            app.messages_loading_older = false;
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
    let from_me = !app.identity.username.is_empty() && message.sender == app.identity.username;
    let viewing = app.open_conv_id.as_deref() == Some(conv_id.as_str());
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
        use crate::domain::MessageContent;
        let modifies_existing = matches!(
            &message.content,
            MessageContent::Edit { .. }
                | MessageContent::Delete { .. }
                | MessageContent::Reaction { .. }
        );
        if modifies_existing {
            request_load_messages(app);
        } else if msg_id != 0 && !app.messages.iter().any(|m| m.id == msg_id) {
            app.messages.push(message);
            app.rebuild_pinned();
        }
    }

    // 2. Incremental inbox bump (no full re-fetch).
    let known = if let Some(c) = app.conversations.iter_mut().find(|c| c.id == conv_id) {
        if sent_at_ms > c.active_at_ms {
            c.active_at_ms = sent_at_ms;
            c.active_at = sent_at;
        }
        // Mark unread unless it's our own message or we're viewing it.
        if !from_me && !viewing {
            c.unread = true;
        }
        true
    } else {
        false
    };
    if known {
        app.rebuild_lowered();
        app.rebuild_filter();
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
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.messages_loading_older = false;
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.messages_loading_older = false;
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        // Channel mapping failed — clear the in-progress flag so the
        // viewport doesn't get stuck showing "loading older…".
        app.messages_loading_older = false;
        return;
    };
    let peek = !app.settings_cache.auto_mark_read;
    if !app.begin(InFlight::LoadOlderMessages) {
        return;
    }
    app.set_action(ActionState::Running("Loading older messages…".into()));
    let _ = app.worker_tx.send(WorkerRequest::ReadMessages {
        channel,
        num: MESSAGES_PER_PAGE,
        peek,
        next_cursor: Some(cursor),
    });
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
            let n = older.len();
            older.extend(std::mem::take(&mut app.messages));
            app.messages = older;
            app.messages_next = next;
            app.messages_loading_older = false;
            app.rebuild_pinned();
            app.set_action(ActionState::Done(format!("Loaded {n} older messages")));
            app.push_cmd(
                "keybase chat api read (older)",
                true,
                format!("{n} messages"),
            );
        }
        Err(e) => {
            app.messages_loading_older = false;
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api read (older)", false, e.to_string());
        }
    }
}

// ── Escape semantics on the conversation screen ─────────────────────

pub fn escape_conversation(app: &mut App) {
    if app.compose.is_empty() {
        close_conversation(app);
    } else {
        app.compose_clear();
    }
}

// ── New conversation popup ──────────────────────────────────────────

pub fn open_new_conversation(app: &mut App) {
    app.new_conv.clear();
    app.screen = crate::tui::screens::Screen::NewConversation;
}

pub fn close_new_conversation(app: &mut App) {
    app.new_conv.clear();
    app.screen = crate::tui::screens::Screen::Inbox;
}

pub fn request_create_new_conversation(app: &mut App) {
    use crate::domain::is_valid_keybase_identity;

    let raw = app.new_conv.text().trim().to_string();
    if raw.is_empty() {
        app.set_action(ActionState::Error("Username list is empty".into()));
        return;
    }
    let mut names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // Client-side validation: surface obvious typos before the worker
    // round-trips a confusing server error. Accepts bare usernames and
    // social-proof identities (`alice@twitter`).
    let invalid: Vec<&str> = names
        .iter()
        .filter(|n| !is_valid_keybase_identity(n))
        .map(String::as_str)
        .collect();
    if !invalid.is_empty() {
        let bad = invalid.join(", ");
        app.set_action(ActionState::Error(format!("Invalid username(s): {bad}")));
        app.push_cmd("new conversation", false, format!("invalid: {bad}"));
        return;
    }

    if !app.identity.username.is_empty() && !names.contains(&app.identity.username) {
        names.insert(0, app.identity.username.clone());
    }
    let channel = ReadChannel {
        name: names.join(","),
        members_type: "impteamnative".into(),
        topic_name: None,
    };
    if !app.begin(InFlight::NewConversation) {
        return;
    }
    app.set_action(ActionState::Running("Creating conversation…".into()));
    let _ = app
        .worker_tx
        .send(WorkerRequest::NewConversation { channel });
}

pub fn handle_new_conversation_response(app: &mut App, result: Result<String, KeybaseError>) {
    match result {
        Ok(id) => {
            app.set_action(ActionState::Done("Conversation created".into()));
            app.push_cmd(
                "keybase chat api newconv",
                true,
                if id.is_empty() {
                    "ok".into()
                } else {
                    // Char-boundary-safe truncation: a non-ASCII id with
                    // a multi-byte codepoint straddling byte 16 would
                    // panic a byte slice on the render thread.
                    format!("id={}", id.chars().take(16).collect::<String>())
                },
            );
            close_new_conversation(app);
            // Re-fetch the inbox so the new conv shows up; stay on the
            // inbox. (Do NOT set open_conv_id here — the conv isn't in
            // `conversations` until the refresh lands, and leaving it set
            // on the inbox screen desyncs open-conversation state.)
            request_load_inbox(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api newconv", false, e.to_string());
        }
    }
}

// ── Server-side search popup ────────────────────────────────────────

pub fn open_search_global(app: &mut App) {
    app.search_global_input.clear();
    app.search_global_results.clear();
    app.search_global_selected = 0;
    app.screen = crate::tui::screens::Screen::SearchGlobal;
}

pub fn close_search_global(app: &mut App) {
    app.search_global_input.clear();
    app.search_global_results.clear();
    app.search_global_selected = 0;
    app.screen = crate::tui::screens::Screen::Inbox;
}

pub fn request_search_inbox_remote(app: &mut App) {
    let q = app.search_global_input.text().trim().to_string();
    if q.is_empty() {
        app.set_action(ActionState::Error("Query is empty".into()));
        return;
    }
    if !app.begin(InFlight::SearchInboxRemote) {
        return;
    }
    app.set_action(ActionState::Running("Searching…".into()));
    let _ = app.worker_tx.send(WorkerRequest::SearchInboxHits {
        query: q,
        max_hits: 30,
    });
}

pub fn handle_search_inbox_response(app: &mut App, result: Result<Vec<InboxHit>, KeybaseError>) {
    match result {
        Ok(hits) => {
            let n = hits.len();
            app.search_global_results = hits;
            app.search_global_selected = 0;
            app.set_action(ActionState::Done(format!("{n} matches")));
            app.push_cmd("keybase chat api searchinbox", true, format!("{n} hits"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api searchinbox", false, e.to_string());
        }
    }
}

pub fn open_selected_search_result(app: &mut App) {
    let Some(hit_ref) = app.search_global_results.get(app.search_global_selected) else {
        return;
    };
    let conv_id = hit_ref.conv_id.clone();
    if !app.conversations.iter().any(|c| c.id == conv_id) {
        app.set_action(ActionState::Error(
            "Conversation not in cached inbox — refresh first".into(),
        ));
        return;
    }
    close_search_global(app);
    app.open_conv_id = Some(conv_id);
    app.messages.clear();
    app.messages_scroll = 0;
    app.compose_open = true;
    app.compose_clear();
    app.screen = crate::tui::screens::Screen::Conversation;
    request_load_messages(app);
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
}

pub fn leave_select_mode(app: &mut App) {
    app.selected_msg_idx = None;
    app.select_from_compose = false;
    app.compose_open = true;
}

pub fn select_move_up(app: &mut App) {
    if let Some(i) = app.selected_msg_idx
        && i > 0
    {
        app.selected_msg_idx = Some(i - 1);
    }
}

pub fn select_move_down(app: &mut App) {
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
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::EditMessage { target_id }) {
        return;
    }
    app.set_action(ActionState::Running("Saving edit…".into()));
    let _ = app.worker_tx.send(WorkerRequest::EditMessage {
        channel,
        message_id: target_id,
        body,
    });
}

pub fn handle_save_edit_response(app: &mut App, result: Result<(), KeybaseError>, target_id: u64) {
    match result {
        Ok(()) => {
            app.compose_clear();
            app.edit_target_id = None;
            app.set_action(ActionState::Done("Edit saved".into()));
            app.push_cmd("keybase chat api edit", true, format!("msg #{target_id}"));
            request_load_messages(app);
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

// ── Attachment download ──────────────────────────────────────────────

/// Reduces an attachment filename emitted by the chat server to a
/// single safe basename that can be concatenated with `~/Downloads/`
/// without escaping the directory.
///
/// The `filename` field on an attachment is set by whoever uploaded
/// it. A malicious sender could pick something like
/// `"../../.ssh/authorized_keys"` to land the file outside the
/// expected destination once the user accepts the download. We
/// defend by:
///
/// * Stripping every path component (`/`, `\`) and keeping only the
///   final segment.
/// * Filtering control characters and NULs (which some filesystems
///   accept but would surprise the user / confuse logs).
/// * Falling back to a synthetic name when the sanitised value is
///   empty, `"."`, or `".."`.
///
/// The user can still edit the proposed path freely in the download
/// popup — this only secures the *default* the popup pre-fills with.
fn safe_attachment_basename(raw: &str, msg_id: u64) -> String {
    // `rsplit` over the path separators always yields at least one
    // item, so `next()` is infallible; the `unwrap_or` is for the
    // type-checker.
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return format!("attachment-{msg_id}.bin");
    }
    trimmed.to_string()
}

pub fn open_download_for_selected(app: &mut App) {
    use crate::domain::MessageContent;
    let Some(idx) = app.selected_msg_idx else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.messages.get(idx) else {
        return;
    };
    let MessageContent::Attachment(att) = &msg.content else {
        app.set_action(ActionState::Error(
            "Selected message is not an attachment".into(),
        ));
        return;
    };
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let fname = safe_attachment_basename(&att.filename, msg.id);
    app.download_msg_id = Some(msg.id);
    app.download.set(format!("{home}/Downloads/{fname}"));
    app.screen = crate::tui::screens::Screen::DownloadAttachment;
}

pub fn close_download(app: &mut App) {
    app.download_msg_id = None;
    app.download.clear();
    app.screen = crate::tui::screens::Screen::Conversation;
}

pub fn request_download_attachment(app: &mut App) {
    let Some(msg_id) = app.download_msg_id else {
        app.set_action(ActionState::Error("No attachment selected".into()));
        return;
    };
    let path = app.download.text().trim().to_string();
    if path.is_empty() {
        app.set_action(ActionState::Error("Output path is empty".into()));
        return;
    }
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::DownloadAttachment {
        message_id: msg_id,
        path: path.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running("Downloading…".into()));
    let _ = app.worker_tx.send(WorkerRequest::DownloadAttachment {
        channel,
        message_id: msg_id,
        output: path,
    });
}

pub fn handle_download_attachment_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    message_id: u64,
    path: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Saved to {path}")));
            app.push_cmd(
                "keybase chat api download",
                true,
                format!("msg #{message_id} → {path}"),
            );
            close_download(app);
            app.selected_msg_idx = None;
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api download", false, e.to_string());
        }
    }
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
}

// ── Delete ───────────────────────────────────────────────────────────

pub fn open_delete_for_selected(app: &mut App) {
    let Some(idx) = app.selected_msg_idx else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.messages.get(idx) else {
        return;
    };
    if msg.sender != app.identity.username {
        app.set_action(ActionState::Error(
            "Can only delete your own messages".into(),
        ));
        return;
    }
    app.delete_msg_yes = false;
    app.screen = crate::tui::screens::Screen::ConfirmDeleteMessage;
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
    app.screen = crate::tui::screens::Screen::Conversation;
}

pub fn request_delete_selected_message(app: &mut App) {
    let Some(idx) = app.selected_msg_idx else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg_id) = app.messages.get(idx).map(|m| m.id) else {
        return;
    };
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::DeleteMessage { message_id: msg_id }) {
        return;
    }
    app.set_action(ActionState::Running("Deleting…".into()));
    let _ = app.worker_tx.send(WorkerRequest::DeleteMessage {
        channel,
        message_id: msg_id,
    });
}

pub fn handle_delete_response(app: &mut App, result: Result<(), KeybaseError>, message_id: u64) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done("Message deleted".into()));
            app.push_cmd(
                "keybase chat api delete",
                true,
                format!("msg #{message_id}"),
            );
            app.screen = crate::tui::screens::Screen::Conversation;
            app.selected_msg_idx = None;
            app.select_from_compose = false;
            app.compose_open = true;
            request_load_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api delete", false, e.to_string());
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
    app.screen = crate::tui::screens::Screen::React;
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
    app.screen = crate::tui::screens::Screen::Conversation;
}

pub fn request_send_reaction(app: &mut App) {
    let body = app.react.text().trim().to_string();
    if body.is_empty() {
        app.set_action(ActionState::Error("Reaction is empty".into()));
        return;
    }
    let Some(idx) = app.selected_msg_idx else {
        return;
    };
    let Some(msg_id) = app.messages.get(idx).map(|m| m.id) else {
        return;
    };
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::SendReaction { body: body.clone() }) {
        return;
    }
    app.set_action(ActionState::Running("Sending reaction…".into()));
    let _ = app.worker_tx.send(WorkerRequest::React {
        channel,
        message_id: msg_id,
        body,
    });
}

pub fn handle_react_response(app: &mut App, result: Result<(), KeybaseError>, body: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done("Reaction sent".into()));
            app.push_cmd("keybase chat api reaction", true, body);
            app.screen = crate::tui::screens::Screen::Conversation;
            app.react.clear();
            app.selected_msg_idx = None;
            app.select_from_compose = false;
            app.compose_open = true;
            request_load_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api reaction", false, e.to_string());
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
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::PinMessage { message_id: msg_id }) {
        return;
    }
    app.set_action(ActionState::Running("Pinning…".into()));
    let _ = app.worker_tx.send(WorkerRequest::PinMessage {
        channel,
        message_id: msg_id,
    });
}

pub fn handle_pin_response(app: &mut App, result: Result<(), KeybaseError>, message_id: u64) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Pinned msg #{message_id}")));
            app.push_cmd("keybase chat api pin", true, format!("msg #{message_id}"));
            // Return to Compose and reload so `rebuild_pinned` refreshes
            // the 📌 indicator from the new history.
            app.selected_msg_idx = None;
            app.select_from_compose = false;
            app.compose_open = true;
            request_load_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api pin", false, e.to_string());
        }
    }
}

pub fn request_unpin_conversation(app: &mut App) {
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::UnpinConversation) {
        return;
    }
    app.set_action(ActionState::Running("Unpinning…".into()));
    let _ = app.worker_tx.send(WorkerRequest::UnpinMessage { channel });
}

pub fn handle_unpin_response(app: &mut App, result: Result<(), KeybaseError>) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done("Pin cleared".into()));
            app.push_cmd("keybase chat api unpin", true, "ok");
            // Reload so the 📌 banner clears from fresh history.
            request_load_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api unpin", false, e.to_string());
        }
    }
}

#[cfg(test)]
mod tests;

// ── Send message ─────────────────────────────────────────────────────

pub fn request_send_message(app: &mut App) {
    let body = app.compose.text().trim().to_string();
    if body.is_empty() {
        app.set_action(ActionState::Error("Message is empty".into()));
        return;
    }
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
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
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
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

// ── Mute / unmute ────────────────────────────────────────────────────

pub fn request_mute_conversation(app: &mut App) {
    set_conv_status_request(app, "muted", "Muting…", "Muted");
}

pub fn request_unmute_conversation(app: &mut App) {
    set_conv_status_request(app, "unfiled", "Unmuting…", "Unmuted");
}

// ── Conversation actions (confirm popup → setstatus) ─────────────────

/// Opens the confirm popup for a [`ConvAction`] on the selected
/// conversation. No-op (with feedback) when nothing is selected.
pub fn open_conv_action(app: &mut App, action: ConvAction) {
    if app.selected_conversation().is_none() {
        app.set_action(ActionState::Error("No conversation selected".into()));
        return;
    }
    app.pending_conv_action = Some(action);
    app.conv_action_yes = false;
    app.screen = crate::tui::screens::Screen::ConfirmConvAction;
}

/// Cancels a pending conversation action and returns to the inbox.
pub fn cancel_conv_action(app: &mut App) {
    app.pending_conv_action = None;
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Commits the pending conversation action: issues its `setstatus`
/// call and returns to the inbox.
pub fn confirm_conv_action(app: &mut App) {
    let Some(action) = app.pending_conv_action.take() else {
        app.screen = crate::tui::screens::Screen::Inbox;
        return;
    };
    app.screen = crate::tui::screens::Screen::Inbox;
    set_conv_status_request(app, action.status(), action.running(), action.done());
}

/// Shared `setstatus` request builder for every status verb.
fn set_conv_status_request(app: &mut App, status: &str, running: &str, done: &str) {
    let Some(conv) = app.selected_conversation() else {
        app.set_action(ActionState::Error("No conversation selected".into()));
        return;
    };
    let channel_result = read_channel_from_conv(conv);
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    if !app.begin(InFlight::SetConvStatus {
        done_label: done.to_string(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(running.to_string()));
    let _ = app.worker_tx.send(WorkerRequest::SetConvStatus {
        channel,
        status: status.to_string(),
    });
}

pub fn handle_set_conv_status_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    done_label: &str,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(done_label.into()));
            app.push_cmd("keybase chat api setstatus", true, done_label.to_string());
            // The flipped status (ignored/blocked/…) changes inbox
            // membership and counts — refresh so the row leaves the view.
            request_load_inbox_silent(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api setstatus", false, e.to_string());
        }
    }
}

// ── Copy conversation label (clipboard, no worker) ──────────────────

/// Synchronous — the clipboard adapter runs on the render thread and
/// completes in milliseconds.
pub fn do_copy_conversation_label(app: &mut App) {
    use crate::domain::conversation_label;
    // On the conversation screen copy the OPEN conversation (Ctrl+Y); on
    // the inbox copy the cursor-selected one (Alt+C). Using the inbox
    // cursor on the conversation screen would copy the wrong label.
    let conv = match app.open_conv_id.as_deref() {
        Some(id) if app.screen == crate::tui::screens::Screen::Conversation => {
            app.conversations.iter().find(|c| c.id == id)
        }
        _ => app.selected_conversation(),
    };
    let Some(conv) = conv else {
        app.set_action(ActionState::Error("No conversation selected".into()));
        return;
    };
    let me = if app.identity.username.is_empty() {
        None
    } else {
        Some(app.identity.username.as_str())
    };
    let label = conversation_label(conv, me);
    let secs = app.settings_cache.clipboard_clear_secs;
    match app.clipboard.write_with_clear(&label, secs) {
        Ok(()) => {
            app.set_action(ActionState::Done("Label copied".into()));
            app.push_cmd("clipboard write", true, label);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("clipboard write", false, e.to_string());
        }
    }
}
