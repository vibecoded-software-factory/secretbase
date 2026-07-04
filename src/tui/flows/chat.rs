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
use crate::tui::app::{PendingBatch, PendingSend, SendState};
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};

/// Number of messages fetched per page. Sized so most chats fit
/// into a single page on first open while keeping the per-page
/// decode cost low when the user paginates back through history.
pub const MESSAGES_PER_PAGE: u32 = 50;

/// Projects a freshly-read message list for display:
/// * drops standalone `reaction` events (Keybase aggregates each onto its
///   target's `reactions` field, which the view renders collapsed beneath it);
/// * **folds edits into their targets** (Discord-style `(edited)`);
/// * **applies deletes** — drops the `delete` events and removes the deleted
///   originals, so a deleted message disappears rather than leaving a stray
///   `(deleted msg #N)` line at deletion time ([`crate::domain::fold_deletes`]).
fn project_messages(msgs: Vec<Message>) -> Vec<Message> {
    let mut out: Vec<Message> = msgs
        .into_iter()
        .filter(|m| !matches!(m.content, crate::domain::MessageContent::Reaction { .. }))
        .collect();
    crate::domain::fold_edits(&mut out);
    crate::domain::fold_deletes(&mut out);
    out
}

// ── Inbox load ────────────────────────────────────────────────────────

/// Queues a refresh of the inbox list (`keybase chat api list`).
pub fn request_load_inbox(app: &mut App) {
    send_list_inbox(app, "Refreshing inbox…");
}

/// Boot-time inbox load: the same `list` call, but labelled "Loading chats…"
/// (shown on the splash, which doubles as the loading screen). On success
/// [`handle_load_inbox_response`] transitions Splash → Inbox, so the inbox is
/// only entered once its conversations are already loaded.
pub fn request_boot_load_inbox(app: &mut App) {
    send_list_inbox(app, "Loading chats…");
}

/// Shared body of the foreground inbox `list` request, parameterised by the
/// spinner label.
fn send_list_inbox(app: &mut App, label: &str) {
    if !app.begin(InFlight::LoadInbox) {
        return;
    }
    app.set_action(ActionState::Running(label.to_string()));
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
            let n = load.conversations.len();
            let skipped_count = load.skipped.len();
            app.conversations = load.conversations;
            // A note-to-self DM can never be genuinely unread — but Keybase's
            // `list` marks it unread whenever your own edits/deletes/reactions
            // advance the latest message id past your read pointer. Clear it so
            // it doesn't show a phantom unread badge.
            let me = app.identity.username.clone();
            if !me.is_empty() {
                for c in app.conversations.iter_mut() {
                    if c.is_self_dm(&me) {
                        c.unread = false;
                    }
                }
            }
            app.inbox_error = None;
            app.rebuild_lowered();
            app.rebuild_filter_preserving_cursor();
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
            // Keep the error for the empty-tree "couldn't load, retry" panel —
            // the toast expires but the inbox stays empty, so the panel is the
            // only lasting signal. A silent bg refresh that fails but already
            // has conversations loaded shouldn't clobber the good list, so only
            // record it when we have nothing to show.
            if app.conversations.is_empty() {
                app.inbox_error = Some(e.to_string());
            }
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api list", false, e.to_string());
        }
    }
    // Boot path: the splash doubles as the loading screen — enter the inbox
    // only now that the conversations are loaded (or surface the error there,
    // so a failed boot load isn't stranded on the splash forever). A normal
    // refresh is already on the inbox, so this is a no-op then.
    if app.screen == crate::tui::screens::Screen::Splash {
        app.screen = crate::tui::screens::Screen::Inbox;
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
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    // Carry the conversation *id*, not an index: a background inbox refresh
    // can reorder/replace `conversations` before the response lands, so an
    // index would then flip `unread` on the wrong row (or be out of bounds).
    if !app.begin(InFlight::MarkRead {
        conv_id: this_conv_id,
    }) {
        return;
    }
    app.set_action(ActionState::Running("Marking as read…".into()));
    let _ = app.worker_tx.send(WorkerRequest::MarkRead {
        channel,
        message_id: upto,
    });
}

pub fn handle_mark_read_response(app: &mut App, result: Result<(), KeybaseError>, conv_id: String) {
    match result {
        Ok(()) => {
            if let Some(c) = app.conversations.iter_mut().find(|c| c.id == conv_id) {
                c.unread = false;
            }
            // Rebuild the view so the unread badge tracks the flipped flag —
            // keeping the cursor on the row the user just marked. (The lowered
            // projection only carries names/labels, so it doesn't need a
            // rebuild for an unread change.)
            app.rebuild_filter_preserving_cursor();
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

/// Resolves the currently-open conversation into the `(conv_id, channel)`
/// pair a worker call needs. Surfaces the standard errors on the feedback
/// strip and returns `None` when no conversation is open, it has left the
/// inbox, or its `members_type` is unsupported — so callers short-circuit
/// with `let Some((conv_id, channel)) = open_channel(app) else { return; };`.
///
/// Collapses the resolution boilerplate every `request_*` for the open
/// conversation used to repeat. (The inbox-cursor variants
/// [`request_mark_read`] / [`set_conv_status_request`] resolve from
/// [`App::selected_conversation`] instead, and [`request_load_messages`]
/// keeps its own copy because it also recovers the screen state when the
/// conversation has vanished.)
fn open_channel(app: &mut App) -> Option<(String, ReadChannel)> {
    let Some(conv_id) = app.open_conv_id.clone() else {
        app.set_action(ActionState::Error("No conversation open".into()));
        return None;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        app.set_action(ActionState::Error("Conversation no longer in inbox".into()));
        return None;
    };
    let channel_result = read_channel_from_conv(conv);
    let channel = resolve_channel_or_fail(app, channel_result)?;
    Some((conv_id, channel))
}

// ── Filter / search query helpers (pure, sync) ───────────────────────

/// Tree navigation: moves the cursor by `delta` over the visible tree rows.
pub fn tree_move(app: &mut App, delta: isize) {
    let len = app.tree_rows().len();
    if len == 0 {
        return;
    }
    let last = (len - 1) as isize;
    app.tree_selected = (app.tree_selected as isize + delta).clamp(0, last) as usize;
}

/// Activates the selected tree row: toggles a group's collapse, or opens a
/// conversation. Returns `true` if a conversation was opened.
pub fn tree_activate(app: &mut App) -> bool {
    match app.tree_rows().get(app.tree_selected) {
        Some(crate::tui::app::TreeRow::Group { key, .. }) => {
            let key = key.clone();
            app.toggle_collapsed(&key);
            let len = app.tree_rows().len();
            if app.tree_selected >= len {
                app.tree_selected = len.saturating_sub(1);
            }
            false
        }
        Some(crate::tui::app::TreeRow::Conv { idx }) => {
            let id = app.conversations[*idx].id.clone();
            open_conversation_by_id(app, id);
            true
        }
        None => false,
    }
}

/// Right / `l` in the tree: **only opens** — expand a collapsed group or open
/// the conversation. Never collapses (so `l` can't loop open↔closed); use
/// `h`/`←` ([`tree_back`]) to collapse / close.
pub fn tree_forward(app: &mut App) {
    match app.tree_rows().get(app.tree_selected) {
        Some(crate::tui::app::TreeRow::Group {
            key,
            collapsed: true,
            ..
        }) => {
            let key = key.clone();
            app.toggle_collapsed(&key); // expand only
            let len = app.tree_rows().len();
            if app.tree_selected >= len {
                app.tree_selected = len.saturating_sub(1);
            }
        }
        Some(crate::tui::app::TreeRow::Conv { idx }) => {
            let id = app.conversations[*idx].id.clone();
            open_conversation_by_id(app, id);
        }
        // Already-expanded group / nothing → no-op.
        _ => {}
    }
}

/// Left / `h` in the tree: collapse an expanded group, or — on a conversation
/// row — close the open chat (master-detail "back").
pub fn tree_back(app: &mut App) {
    match app.tree_rows().get(app.tree_selected) {
        // An expanded group collapses.
        Some(crate::tui::app::TreeRow::Group {
            key,
            collapsed: false,
            ..
        }) => {
            let key = key.clone();
            app.toggle_collapsed(&key);
        }
        // On a conversation row, close the open chat.
        Some(crate::tui::app::TreeRow::Conv { .. }) if app.open_conv_id.is_some() => {
            close_conversation(app);
        }
        _ => {}
    }
}

// ── Open / close conversation detail screen ──────────────────────────

/// Shared conversation-open path: stash the previously-open draft, switch to
/// `id`, restore its draft, and load its messages.
fn enter_conversation(app: &mut App, id: String) {
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
    if !app.begin(InFlight::LoadMessages) {
        return;
    }
    app.set_action(ActionState::Running("Loading messages…".into()));
    let _ = app.worker_tx.send(WorkerRequest::ReadMessages {
        channel,
        num,
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

/// Esc in the compose pane: cancel an in-progress edit first, else clear a
/// non-empty draft, else close the conversation (focus back to the tree).
/// The single Esc path — the input handler routes here.
pub fn escape_conversation(app: &mut App) {
    if app.edit_target_id.is_some() {
        cancel_edit(app);
    } else if app.compose.is_empty() {
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

// ── Unhide popup (restore a blocked/reported conversation by name) ───

/// Opens the **Unhide** popup. Blocked/reported conversations are excluded from
/// the inbox `list` (verified: the chat `list` JSON has no status filter), so
/// they can't be reached from the tree — this restores one by name.
pub fn open_unhide(app: &mut App) {
    app.unhide_input.clear();
    app.screen = crate::tui::screens::Screen::UnhideConversation;
}

pub fn close_unhide(app: &mut App) {
    app.unhide_input.clear();
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Restores a blocked/reported/ignored DM by name: builds the implicit-team
/// channel from the typed username(s) and issues `setstatus unfiled` directly
/// (no inbox row needed). The response handler refreshes, so it reappears.
pub fn request_unhide_conversation(app: &mut App) {
    use crate::domain::is_valid_keybase_identity;

    let raw = app.unhide_input.text().trim().to_string();
    if raw.is_empty() {
        app.set_action(ActionState::Error("Username is empty".into()));
        return;
    }
    let mut names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let invalid: Vec<&str> = names
        .iter()
        .filter(|n| !is_valid_keybase_identity(n))
        .map(String::as_str)
        .collect();
    if !invalid.is_empty() {
        let bad = invalid.join(", ");
        app.set_action(ActionState::Error(format!("Invalid username(s): {bad}")));
        app.push_cmd("unhide", false, format!("invalid: {bad}"));
        return;
    }
    // Implicit-team DMs are keyed by the full participant set (us included).
    if !app.identity.username.is_empty() && !names.contains(&app.identity.username) {
        names.insert(0, app.identity.username.clone());
    }
    let channel = ReadChannel {
        name: names.join(","),
        members_type: "impteamnative".into(),
        topic_name: None,
    };
    if !app.begin(InFlight::SetConvStatus {
        done_label: "Restored".to_string(),
    }) {
        return;
    }
    app.set_action(ActionState::Running("Restoring…".into()));
    let _ = app.worker_tx.send(WorkerRequest::SetConvStatus {
        channel,
        status: "unfiled".to_string(),
    });
    // Close the popup now; `handle_set_conv_status_response` refreshes the inbox.
    app.unhide_input.clear();
    app.screen = crate::tui::screens::Screen::Inbox;
}

// ── Channel browser (`c` on a team) ───────────────────────────────

/// The channel name of a team-channel conversation (its `topic_name`), used for
/// display and to build the `join`/`leave` channel.
fn channel_topic(c: &crate::domain::Conversation) -> String {
    c.channel.topic_name.clone().unwrap_or_default()
}

/// Whether a channel row is one you're a member of.
fn channel_joined(c: &crate::domain::Conversation) -> bool {
    c.member_status == crate::domain::MemberStatus::Active
}

/// Opens the channel browser for the team of the selected tree row (a team
/// group header, or a team channel). Errors when the cursor isn't on a team.
pub fn open_channel_browser(app: &mut App) {
    use crate::tui::app::TreeRow;
    let team = match app.tree_rows().get(app.tree_selected) {
        Some(TreeRow::Group {
            is_team: true, key, ..
        }) => Some(key.clone()),
        Some(TreeRow::Conv { idx }) => {
            let c = &app.conversations[*idx];
            c.channel
                .members_type
                .is_team()
                .then(|| c.channel.name.clone())
        }
        _ => None,
    };
    let Some(team) = team else {
        app.set_action(ActionState::Error(
            "Select a team (or a team channel) first".into(),
        ));
        return;
    };
    open_channel_browser_for_team(app, team);
}

/// Opens the channel browser for a named team (e.g. from the Teams screen).
pub fn open_channel_browser_for_team(app: &mut App, team: String) {
    app.channel_browser_team = Some(team);
    app.channels.clear();
    app.channel_selected = 0;
    app.default_channels.clear();
    clear_channel_input(app);
    app.screen = crate::tui::screens::Screen::ChannelBrowser;
    request_load_channels(app);
}

pub fn close_channel_browser(app: &mut App) {
    app.channel_browser_team = None;
    app.channels.clear();
    app.channel_selected = 0;
    app.default_channels.clear();
    clear_channel_input(app);
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Clears every inline browser mode (create / rename / delete-confirm).
fn clear_channel_input(app: &mut App) {
    app.channel_creating = false;
    app.channel_renaming = None;
    app.channel_confirm_delete = None;
    app.channel_new_name.clear();
}

/// Enters create mode in the browser (`Alt+N`): the new-channel-name input.
pub fn open_channel_create(app: &mut App) {
    if app.channel_browser_team.is_none() {
        return;
    }
    clear_channel_input(app);
    app.channel_creating = true;
}

/// Cancels create mode, back to the channel list.
pub fn cancel_channel_create(app: &mut App) {
    clear_channel_input(app);
}

// ── Rename channel (r) ───────────────────────────────────────────────

/// Enters rename mode on the selected channel (`r`), pre-filling its name.
pub fn open_channel_rename(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let old = channel_topic(c);
    clear_channel_input(app);
    app.channel_new_name.set(old.clone());
    app.channel_renaming = Some(old);
}

pub fn cancel_channel_rename(app: &mut App) {
    clear_channel_input(app);
}

pub fn request_rename_channel(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let Some(old) = app.channel_renaming.clone() else {
        return;
    };
    let new = app.channel_new_name.text().trim().to_lowercase();
    if new.is_empty() {
        app.set_action(ActionState::Error("New channel name is empty".into()));
        return;
    }
    if new == old {
        cancel_channel_rename(app);
        return;
    }
    if !app.begin(InFlight::RenameChannel { topic: new.clone() }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Renaming #{old} → #{new}…")));
    let _ = app
        .worker_tx
        .send(WorkerRequest::RenameChannel { team, old, new });
}

pub fn handle_rename_channel_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    topic: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Renamed to #{topic}")));
            app.push_cmd("keybase chat rename-channel", true, format!("#{topic}"));
            clear_channel_input(app);
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat rename-channel", false, e.to_string());
        }
    }
}

// ── Delete channel (d, inline confirm) ───────────────────────────────

/// Opens the inline delete confirm for the selected channel (`d`).
pub fn open_channel_delete_confirm(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let topic = channel_topic(c);
    clear_channel_input(app);
    app.channel_confirm_delete = Some(topic);
}

pub fn cancel_channel_delete(app: &mut App) {
    clear_channel_input(app);
}

/// Commits the pending channel delete (`y`). Destructive + irreversible.
pub fn confirm_channel_delete(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let Some(topic) = app.channel_confirm_delete.clone() else {
        return;
    };
    app.channel_confirm_delete = None;
    if !app.begin(InFlight::DeleteChannel {
        topic: topic.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Deleting #{topic}…")));
    let _ = app.worker_tx.send(WorkerRequest::DeleteChannel {
        team,
        channel: topic,
    });
}

pub fn handle_delete_channel_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    topic: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Deleted #{topic}")));
            app.push_cmd("keybase chat delete-channel", true, format!("#{topic}"));
            clear_channel_input(app);
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat delete-channel", false, e.to_string());
        }
    }
}

// ── Default channels (t: toggle) ─────────────────────────────────────

/// Fetches the team's default channels (get-only), chained after the list load
/// so the browser can badge them. Quiet — no error toast if it fails (badges
/// just stay empty).
pub fn request_get_default_channels(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    if !app.begin(InFlight::DefaultChannels { setting: false }) {
        return;
    }
    let _ = app.worker_tx.send(WorkerRequest::DefaultChannels {
        team,
        set: Vec::new(),
    });
}

/// `t`: toggles the selected channel in the team's default set (SET replaces the
/// whole set, so we recompute it). `#general` is always default; and the CLI
/// can't clear the set to empty (`--channel`-less = get), so removing the last
/// one is refused with an explanation.
pub fn toggle_default_channel(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let topic = channel_topic(c);
    if topic == "general" {
        app.set_action(ActionState::Error(
            "#general is always a default channel".into(),
        ));
        return;
    }
    let mut set = app.default_channels.clone();
    if let Some(pos) = set.iter().position(|x| *x == topic) {
        set.remove(pos);
    } else {
        set.push(topic);
    }
    if set.is_empty() {
        app.set_action(ActionState::Error(
            "Keybase can't clear the last default channel via the CLI".into(),
        ));
        return;
    }
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    if !app.begin(InFlight::DefaultChannels { setting: true }) {
        return;
    }
    app.set_action(ActionState::Running("Updating default channels…".into()));
    let _ = app
        .worker_tx
        .send(WorkerRequest::DefaultChannels { team, set });
}

pub fn handle_default_channels_response(
    app: &mut App,
    result: Result<Vec<String>, KeybaseError>,
    setting: bool,
) {
    match result {
        Ok(names) => {
            let n = names.len();
            app.default_channels = names;
            if setting {
                app.set_action(ActionState::Done("Default channels updated".into()));
                app.push_cmd("keybase chat default-channels", true, "updated");
            } else {
                // Quiet load-time get — just clear the spinner.
                app.set_action(ActionState::Idle);
                app.push_cmd(
                    "keybase chat default-channels",
                    true,
                    format!("{n} default(s)"),
                );
            }
        }
        Err(e) => {
            // A get failure is non-critical (badges stay empty); a set failure
            // is a real error the user should see.
            if setting {
                app.set_action(ActionState::Error(e.to_string()));
            } else {
                app.set_action(ActionState::Idle);
            }
            app.push_cmd("keybase chat default-channels", false, e.to_string());
        }
    }
}

// ── Members view (listmembers / addtochannel / removefromchannel) ────

/// Clears the Members view's inline modes (add input / remove confirm).
fn clear_member_input(app: &mut App) {
    app.member_adding = false;
    app.member_add_input.clear();
    app.member_confirm_remove = None;
}

/// Opens the Members view for `channel` (labelled `label`), returning to
/// `return_to` on close, and loads the member list.
fn open_members(app: &mut App, channel: ReadChannel, label: String, return_to: Screen) {
    app.members_channel = Some(channel);
    app.members_label = label;
    app.members.clear();
    app.members_selected = 0;
    app.members_return = return_to;
    clear_member_input(app);
    app.screen = Screen::Members;
    request_load_members(app);
}

/// `m` in the channel browser: members of the selected channel.
pub fn open_members_from_browser(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    let topic = channel_topic(c);
    let channel = ReadChannel {
        name: team.clone(),
        members_type: "team".into(),
        topic_name: Some(topic.clone()),
    };
    open_members(
        app,
        channel,
        format!("{team}#{topic}"),
        Screen::ChannelBrowser,
    );
}

/// `Alt+P` on an open team channel: its members. DMs/non-team convs have a
/// fixed membership, so it's refused there.
pub fn open_members_from_conversation(app: &mut App) {
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    if conv.channel.members_type != crate::domain::MembersType::Team {
        app.set_action(ActionState::Error(
            "Members is only available for team channels".into(),
        ));
        return;
    }
    let label = format!(
        "{}#{}",
        conv.channel.name,
        conv.channel.topic_name.as_deref().unwrap_or("general")
    );
    let channel = match read_channel_from_conv(conv) {
        Ok(ch) => ch,
        Err(_) => return,
    };
    open_members(app, channel, label, Screen::Inbox);
}

pub fn close_members(app: &mut App) {
    let return_to = app.members_return;
    app.members_channel = None;
    app.members.clear();
    app.members_selected = 0;
    clear_member_input(app);
    app.screen = return_to;
}

pub fn request_load_members(app: &mut App) {
    let Some(channel) = app.members_channel.clone() else {
        return;
    };
    if !app.begin(InFlight::LoadMembers) {
        return;
    }
    app.set_action(ActionState::Running("Loading members…".into()));
    let _ = app.worker_tx.send(WorkerRequest::LoadMembers { channel });
}

pub fn handle_load_members_response(
    app: &mut App,
    result: Result<Vec<crate::domain::ChatMember>, KeybaseError>,
) {
    match result {
        Ok(mut members) => {
            // Higher-privilege roles first, then alphabetical by username.
            members.sort_by(|a, b| {
                a.role
                    .sort_rank()
                    .cmp(&b.role.sort_rank())
                    .then_with(|| a.username.cmp(&b.username))
            });
            let n = members.len();
            app.members = members;
            app.members_selected = app
                .members_selected
                .min(app.members.len().saturating_sub(1));
            app.set_action(ActionState::Done(format!("{n} members")));
            app.push_cmd("keybase chat api listmembers", true, format!("{n} members"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api listmembers", false, e.to_string());
        }
    }
}

pub fn members_move(app: &mut App, delta: isize) {
    let len = app.members.len();
    if len == 0 {
        return;
    }
    app.members_selected =
        (app.members_selected as isize + delta).clamp(0, len as isize - 1) as usize;
}

// add member(s)

pub fn open_member_add(app: &mut App) {
    if app.members_channel.is_none() {
        return;
    }
    clear_member_input(app);
    app.member_adding = true;
}

pub fn cancel_member_add(app: &mut App) {
    clear_member_input(app);
}

pub fn request_add_members(app: &mut App) {
    use crate::domain::is_valid_keybase_identity;
    let Some(channel) = app.members_channel.clone() else {
        return;
    };
    // Accept comma / whitespace separated usernames.
    let usernames: Vec<String> = app
        .member_add_input
        .text()
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    if usernames.is_empty() {
        app.set_action(ActionState::Error("No usernames entered".into()));
        return;
    }
    if let Some(bad) = usernames.iter().find(|u| !is_valid_keybase_identity(u)) {
        app.set_action(ActionState::Error(format!("Invalid username: {bad}")));
        return;
    }
    let count = usernames.len();
    if !app.begin(InFlight::AddToChannel { count }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Adding {count} member(s)…")));
    let _ = app
        .worker_tx
        .send(WorkerRequest::AddToChannel { channel, usernames });
}

pub fn handle_add_members_response(app: &mut App, result: Result<(), KeybaseError>, count: usize) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Added {count} member(s)")));
            app.push_cmd(
                "keybase chat api addtochannel",
                true,
                format!("{count} added"),
            );
            clear_member_input(app);
            request_load_members(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api addtochannel", false, e.to_string());
        }
    }
}

// remove member (inline confirm)

pub fn open_member_remove_confirm(app: &mut App) {
    let Some(m) = app.members.get(app.members_selected) else {
        return;
    };
    let username = m.username.clone();
    clear_member_input(app);
    app.member_confirm_remove = Some(username);
}

pub fn cancel_member_remove(app: &mut App) {
    clear_member_input(app);
}

pub fn confirm_remove_member(app: &mut App) {
    let Some(channel) = app.members_channel.clone() else {
        return;
    };
    let Some(username) = app.member_confirm_remove.clone() else {
        return;
    };
    app.member_confirm_remove = None;
    if !app.begin(InFlight::RemoveFromChannel {
        username: username.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Removing {username}…")));
    let _ = app.worker_tx.send(WorkerRequest::RemoveFromChannel {
        channel,
        usernames: vec![username],
    });
}

pub fn handle_remove_member_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    username: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Removed {username}")));
            app.push_cmd("keybase chat api removefromchannel", true, username);
            clear_member_input(app);
            request_load_members(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api removefromchannel", false, e.to_string());
        }
    }
}

/// Creates a new channel on the browsed team (`newconv` with a team channel).
pub fn request_create_channel(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    let topic = app.channel_new_name.text().trim().to_lowercase();
    if topic.is_empty() {
        app.set_action(ActionState::Error("Channel name is empty".into()));
        return;
    }
    let channel = ReadChannel {
        name: team,
        members_type: "team".into(),
        topic_name: Some(topic.clone()),
    };
    if !app.begin(InFlight::CreateChannel {
        topic: topic.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Creating #{topic}…")));
    // Reuses the newconv worker request; routed to the channel handler by the
    // CreateChannel in-flight slot.
    let _ = app
        .worker_tx
        .send(WorkerRequest::NewConversation { channel });
}

pub fn handle_create_channel_response(
    app: &mut App,
    result: Result<String, KeybaseError>,
    topic: String,
) {
    match result {
        Ok(_) => {
            app.set_action(ActionState::Done(format!("Created #{topic}")));
            app.push_cmd(
                "keybase chat api newconv (channel)",
                true,
                format!("#{topic}"),
            );
            app.channel_creating = false;
            app.channel_new_name.clear();
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api newconv (channel)", false, e.to_string());
        }
    }
}

/// Loads every channel of `channel_browser_team` (`listconvsonname`).
pub fn request_load_channels(app: &mut App) {
    let Some(team) = app.channel_browser_team.clone() else {
        return;
    };
    if !app.begin(InFlight::LoadChannels) {
        return;
    }
    app.set_action(ActionState::Running(format!("Loading channels of {team}…")));
    let _ = app.worker_tx.send(WorkerRequest::LoadChannels { team });
}

pub fn handle_load_channels_response(
    app: &mut App,
    result: Result<ListConversationsOk, KeybaseError>,
) {
    match result {
        Ok(load) => {
            let n = load.conversations.len();
            app.channels = load.conversations;
            // Joined channels first, then alphabetical by channel name.
            app.channels.sort_by(|a, b| {
                channel_joined(b)
                    .cmp(&channel_joined(a))
                    .then_with(|| channel_topic(a).cmp(&channel_topic(b)))
            });
            app.channel_selected = app
                .channel_selected
                .min(app.channels.len().saturating_sub(1));
            app.set_action(ActionState::Done(format!("{n} channels")));
            app.push_cmd(
                "keybase chat api listconvsonname",
                true,
                format!("{n} channels"),
            );
            for diag in load.skipped {
                app.push_cmd("channel parse warning", false, diag);
            }
            // Chain a get of the team's default channels to badge them.
            request_get_default_channels(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api listconvsonname", false, e.to_string());
        }
    }
}

pub fn channel_browser_move(app: &mut App, delta: isize) {
    let len = app.channels.len();
    if len == 0 {
        return;
    }
    app.channel_selected =
        (app.channel_selected as isize + delta).clamp(0, len as isize - 1) as usize;
}

/// `Enter` in the browser: open a channel you're in, or join one you're not.
pub fn channel_browser_activate(app: &mut App) {
    let Some(c) = app.channels.get(app.channel_selected) else {
        return;
    };
    if channel_joined(c) {
        let id = c.id.clone();
        close_channel_browser(app);
        open_conversation_by_id(app, id);
    } else {
        request_join_selected_channel(app);
    }
}

/// Builds the `team#channel` [`ReadChannel`] for the selected browser row.
fn selected_channel_read(app: &App) -> Option<(String, ReadChannel)> {
    let team = app.channel_browser_team.clone()?;
    let c = app.channels.get(app.channel_selected)?;
    let topic = channel_topic(c);
    Some((
        topic.clone(),
        ReadChannel {
            name: team,
            members_type: "team".into(),
            topic_name: Some(topic),
        },
    ))
}

pub fn request_join_selected_channel(app: &mut App) {
    let Some((topic, channel)) = selected_channel_read(app) else {
        return;
    };
    if !app.begin(InFlight::JoinChannel {
        topic: topic.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Joining #{topic}…")));
    let _ = app.worker_tx.send(WorkerRequest::JoinChannel { channel });
}

pub fn handle_join_response(app: &mut App, result: Result<(), KeybaseError>, topic: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Joined #{topic}")));
            app.push_cmd("keybase chat api join", true, format!("#{topic}"));
            // Pull the new channel into the inbox so it's openable, and refresh
            // the browser so its membership flips.
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api join", false, e.to_string());
        }
    }
}

pub fn request_leave_selected_channel(app: &mut App) {
    let joined = app
        .channels
        .get(app.channel_selected)
        .is_some_and(channel_joined);
    if !joined {
        app.set_action(ActionState::Error("Not a member of this channel".into()));
        return;
    }
    let Some((topic, channel)) = selected_channel_read(app) else {
        return;
    };
    if !app.begin(InFlight::LeaveChannel {
        topic: topic.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Leaving #{topic}…")));
    let _ = app.worker_tx.send(WorkerRequest::LeaveChannel { channel });
}

pub fn handle_leave_response(app: &mut App, result: Result<(), KeybaseError>, topic: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Left #{topic}")));
            app.push_cmd("keybase chat api leave", true, format!("#{topic}"));
            request_load_inbox_silent(app);
            if app.screen == crate::tui::screens::Screen::ChannelBrowser {
                request_load_channels(app);
            }
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api leave", false, e.to_string());
        }
    }
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
    let target = hit_ref.message_id;
    if !app.conversations.iter().any(|c| c.id == conv_id) {
        app.set_action(ActionState::Error(
            "Conversation not in cached inbox — refresh first".into(),
        ));
        return;
    }
    close_search_global(app);
    enter_conversation(app, conv_id);
    // Land on the matched message once the history loads (try_jump_to_search
    // _target, called from the read handlers, paginates older if needed).
    app.pending_search_jump = Some(target);
}

/// After a (paginated) read, jump to + highlight the message the global
/// search targeted. If it's older than what's loaded, pull the next older
/// page and re-check when it arrives; give up once history is exhausted.
fn try_jump_to_search_target(app: &mut App) {
    let Some(target) = app.pending_search_jump else {
        return;
    };
    if let Some(idx) = app.messages.iter().position(|m| m.id == target) {
        // Selecting the message makes the renderer scroll it into view and
        // highlight it (same as Select mode).
        app.pending_search_jump = None;
        app.compose_open = false;
        app.select_from_compose = false;
        app.selected_msg_idx = Some(idx);
        app.set_action(ActionState::Done("Jumped to message".into()));
    } else if app.messages_next.is_some() && !app.messages_loading_older {
        app.messages_loading_older = true;
        request_load_older_messages(app);
    } else if !app.messages_loading_older {
        app.pending_search_jump = None;
        app.set_action(ActionState::Error("Message not found in history".into()));
    }
}

// ── In-conversation search (Ctrl+F → searchregexp) ───────────────────

/// Max matches requested from `searchregexp` for the in-conversation search.
const CONV_SEARCH_MAX_HITS: u32 = 50;

/// Opens the in-conversation search **modal** (`Screen::ConvSearch`, `Ctrl+F`) —
/// a centered `searchregexp` box + results, like the global-search modal. No-op
/// when no conversation is open.
pub fn open_conv_search(app: &mut App) {
    if app.open_conv_id.is_none() {
        return;
    }
    app.conv_search.clear();
    app.conv_search_results.clear();
    app.conv_search_selected = 0;
    app.screen = crate::tui::screens::Screen::ConvSearch;
}

/// Closes the search modal, clearing its query + results, back to the inbox/chat.
pub fn close_conv_search(app: &mut App) {
    app.conv_search.clear();
    app.conv_search_results.clear();
    app.conv_search_selected = 0;
    if app.screen == crate::tui::screens::Screen::ConvSearch {
        app.screen = crate::tui::screens::Screen::Inbox;
    }
}

/// Runs `searchregexp` over the open conversation for the current query.
pub fn request_conv_search(app: &mut App) {
    let q = app.conv_search.text().trim().to_string();
    if q.is_empty() {
        app.set_action(ActionState::Error("Search is empty".into()));
        return;
    }
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    if !app.begin(InFlight::ConvSearch) {
        return;
    }
    app.set_action(ActionState::Running("Searching conversation…".into()));
    let _ = app.worker_tx.send(WorkerRequest::SearchRegexp {
        channel,
        query: q,
        max_hits: CONV_SEARCH_MAX_HITS,
    });
}

pub fn handle_conv_search_response(app: &mut App, result: Result<Vec<InboxHit>, KeybaseError>) {
    match result {
        Ok(hits) => {
            let n = hits.len();
            app.conv_search_results = hits;
            app.conv_search_selected = 0;
            app.set_action(ActionState::Done(format!("{n} matches")));
            app.push_cmd("keybase chat api searchregexp", true, format!("{n} hits"));
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api searchregexp", false, e.to_string());
        }
    }
}

/// Jumps to the highlighted in-conversation match: closes the search and
/// reuses the message-jump path (finds it in the loaded history, paginating
/// older if needed).
pub fn conv_search_jump_selected(app: &mut App) {
    let Some(hit) = app.conv_search_results.get(app.conv_search_selected) else {
        return;
    };
    let target = hit.message_id;
    close_conv_search(app);
    // Land focus on the chat so the jumped-to message is in Select mode.
    app.focus = crate::tui::screens::Focus::Chat;
    app.pending_search_jump = Some(target);
    try_jump_to_search_target(app);
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
    let filename = safe_attachment_basename(&att.filename, msg.id);
    app.picker_action = crate::tui::app::PickerAction::Download {
        message_id: msg.id,
        filename,
    };
    // Pick the destination directory, starting at the user's Downloads.
    app.file_picker = Some(crate::tui::file_picker::FilePicker::new_dir(
        &default_download_dir(),
    ));
}

/// The OS default Downloads directory: `$XDG_DOWNLOAD_DIR`, then
/// `~/Downloads`, then `/` as a last resort.
fn default_download_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(d) = std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
    {
        return d;
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    PathBuf::from("/")
}

/// Downloads attachment `message_id` from the open conversation into
/// `dir`, saved under `filename`. Fired when the directory picker returns.
pub fn request_download_to(
    app: &mut App,
    message_id: u64,
    dir: std::path::PathBuf,
    filename: String,
) {
    let output = dir.join(&filename).to_string_lossy().to_string();
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    if !app.begin(InFlight::DownloadAttachment {
        message_id,
        path: output.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Downloading {filename}…")));
    let _ = app.worker_tx.send(WorkerRequest::DownloadAttachment {
        channel,
        message_id,
        output,
    });
}

/// Cache directory for downloaded inline-preview images
/// (`$XDG_CACHE_HOME/secretbase/images`, falling back to `~/.cache` / tmp).
pub fn image_cache_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("secretbase").join("images")
}

/// Stable cache path for an attachment image, `{conv}-{msg}.{ext}` — unique
/// per conversation so per-conversation message ids can't collide.
pub fn image_path_for(conv_id: &str, msg_id: u64, filename: &str) -> String {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("img");
    image_cache_dir()
        .join(format!("{conv_id}-{msg_id}.{ext}"))
        .to_string_lossy()
        .to_string()
}

/// Enqueues background downloads for every visible image that isn't already
/// cached, in flight, or known-failed. Called from the run loop after each
/// draw (the view fills `image_to_fetch` with `(message_id, cache path)`).
pub fn ensure_visible_images(app: &mut App) {
    if app.image_to_fetch.is_empty() {
        return;
    }
    let to_fetch = std::mem::take(&mut app.image_to_fetch);
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    let Ok(channel) = read_channel_from_conv(conv) else {
        return;
    };
    let _ = std::fs::create_dir_all(image_cache_dir());
    for (msg_id, output) in to_fetch {
        if app.image_ready.contains(&output)
            || app.image_pending.contains(&output)
            || app.image_failed.contains(&output)
        {
            continue;
        }
        // Reuse a file fetched in an earlier session rather than re-downloading.
        if std::path::Path::new(&output).exists() {
            app.image_ready.insert(output);
            app.image_dirty = true;
            continue;
        }
        app.image_pending.insert(output.clone());
        let _ = app.worker_tx.send(WorkerRequest::PreviewImage {
            channel: channel.clone(),
            message_id: msg_id,
            output,
        });
    }
}

/// Whether the single selected message is an image whose file is on disk —
/// returns its `(cache path, mime)` so `c` can copy the image itself.
fn selected_ready_image(app: &App) -> Option<(String, String)> {
    if !app.msg_marks.is_empty() {
        return None; // multi-select copies text, not a single image
    }
    let m = app.messages.get(app.selected_msg_idx?)?;
    let crate::domain::MessageContent::Attachment(att) = &m.content else {
        return None;
    };
    if !crate::tui::image::is_image(&att.mime_type, &att.filename) {
        return None;
    }
    let conv_id = app.open_conv_id.as_deref()?;
    let path = image_path_for(conv_id, m.id, &att.filename);
    if app.image_ready.contains(&path) || std::path::Path::new(&path).exists() {
        let mime = if att.mime_type.is_empty() {
            "image/png".to_string()
        } else {
            att.mime_type.clone()
        };
        Some((path, mime))
    } else {
        None
    }
}

/// `c` in select mode: copy the **image** to the clipboard when a downloaded
/// image is selected, otherwise copy the message body text.
pub fn do_copy_content(app: &mut App) {
    let Some((path, mime)) = selected_ready_image(app) else {
        do_copy_messages(app, false);
        return;
    };
    match crate::tui::image::copy_to_clipboard(&path, &mime) {
        Ok(()) => {
            app.push_cmd("copy image", true, &path);
            app.set_action(ActionState::Done("Image copied to clipboard".into()));
        }
        Err(e) => {
            app.set_action(ActionState::Error(format!("Copy failed: {e}")));
        }
    }
}

/// Applies a finished background image download: mark the cache path ready (or
/// failed) and flag a repaint.
pub fn handle_preview_image_response(
    app: &mut App,
    path: String,
    result: Result<(), KeybaseError>,
) {
    app.image_pending.remove(&path);
    match result {
        Ok(()) => {
            app.image_ready.insert(path);
        }
        Err(_) => {
            app.image_failed.insert(path);
        }
    }
    app.image_dirty = true;
}

/// Enqueues background GIF decodes for every visible GIF that's downloaded but
/// not yet decoded (and not already in flight). Called from the run loop after
/// each draw; the view fills `gif_to_decode` with cache paths. Runs on the
/// background lane so a multi-second decode never stalls the user's keybase
/// calls on the main worker.
pub fn ensure_pending_gif_decodes(app: &mut App) {
    if app.gif_to_decode.is_empty() {
        return;
    }
    let to_decode = std::mem::take(&mut app.gif_to_decode);
    for path in to_decode {
        if app.gif_pending.contains(&path) || app.gif_anims.contains_key(&path) {
            continue;
        }
        app.gif_pending.insert(path.clone());
        let _ = app.bg_worker_tx.send(WorkerRequest::DecodeGif { path });
    }
}

/// Stores a finished off-thread GIF decode: `Some(frames)` for an animated GIF,
/// `None` for a still / single-frame GIF (then rendered as a static image).
/// Clears the pending flag and flags a repaint so the skeleton gives way.
pub fn handle_decode_gif_response(
    app: &mut App,
    path: String,
    frames: Option<crate::tui::image::GifFrames>,
) {
    app.gif_pending.remove(&path);
    app.gif_anims.insert(path, frames);
    app.image_dirty = true;
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
    if !app.begin(InFlight::DeleteMessage { message_id: id }) {
        return;
    }
    app.set_action(ActionState::Running(if total > 1 {
        format!("Deleting… {}/{}", done + 1, total)
    } else {
        "Deleting…".into()
    }));
    let _ = app.worker_tx.send(WorkerRequest::DeleteMessage {
        channel,
        message_id: id,
    });
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
    if !app.begin(InFlight::SendReaction { body: body.clone() }) {
        return;
    }
    app.set_action(ActionState::Running(if total > 1 {
        format!("Reacting… {}/{}", done + 1, total)
    } else {
        "Sending reaction…".into()
    }));
    let _ = app.worker_tx.send(WorkerRequest::React {
        channel,
        message_id: id,
        body,
    });
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
            request_reload_messages(app);
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

// ── Upload attachment ────────────────────────────────────────────────

/// Uploads a local file to the open conversation as an attachment
/// (`keybase chat api {"method":"attach"}`). The path comes from the
/// embedded file picker.
pub fn request_upload_attachment(app: &mut App, path: std::path::PathBuf) {
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    let filename = path.to_string_lossy().to_string();
    let display = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| filename.clone());
    if !app.begin(InFlight::UploadAttachment {
        filename: display.clone(),
    }) {
        return;
    }
    app.set_action(ActionState::Running(format!("Uploading {display}…")));
    let _ = app.worker_tx.send(WorkerRequest::UploadAttachment {
        channel,
        filename,
        title: String::new(),
    });
}

pub fn handle_upload_response(app: &mut App, result: Result<(), KeybaseError>, filename: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Uploaded {filename}")));
            app.push_cmd("keybase chat api attach", true, filename);
            // Re-read so the new attachment message appears.
            app.messages_scroll = 0;
            request_load_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api attach", false, e.to_string());
        }
    }
}

// ── Local mute / favorite (no Keybase call) ──────────────────────────

/// Toggles the **local-only** mute on the selected conversation. Synchronous,
/// instant, no Keybase call — see [`App::toggle_muted`]. Keybase's own `muted`
/// status is intentionally untouched (the CLI can't read it back, so a synced
/// state would drift). Muting suppresses secretbase's unread indicators for the
/// conv; it does **not** silence Keybase notifications on your other devices.
pub fn toggle_muted_conversation(app: &mut App) {
    let Some(id) = app.selected_conversation().map(|c| c.id.clone()) else {
        app.set_action(ActionState::Error("No conversation selected".into()));
        return;
    };
    let on = app.toggle_muted(id);
    app.set_action(ActionState::Done(
        if on {
            "Muted (local)"
        } else {
            "Unmuted (local)"
        }
        .into(),
    ));
    // The unread badge changed — reproject the tree, keeping the cursor on
    // the row the user just muted.
    app.rebuild_filter_preserving_cursor();
}

/// Toggles the **local-only** star on the selected conversation. Synchronous,
/// instant, no Keybase call — see [`App::toggle_favorite`]. (Keybase's own
/// `favorite` status is intentionally untouched: the CLI can't read it back, so
/// a synced star would drift; this one is fully ours.)
pub fn toggle_favorite_conversation(app: &mut App) {
    let Some(id) = app.selected_conversation().map(|c| c.id.clone()) else {
        app.set_action(ActionState::Error("No conversation selected".into()));
        return;
    };
    let on = app.toggle_favorite(id);
    app.set_action(ActionState::Done(
        if on {
            "Favorited (local)"
        } else {
            "Unfavorited (local)"
        }
        .into(),
    ));
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
        Some(id) if app.focus == crate::tui::screens::Focus::Chat => {
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

/// Copies the marked command-log lines (or the cursor line if none are
/// marked) to the clipboard. `full` copies the whole line (`✓ cmd → detail
/// (dur)`); otherwise just the `detail`. The selection is kept so the user
/// can copy it both ways.
pub fn do_copy_cmd_log(app: &mut App, full: bool) {
    let len = app.cmd_log.len();
    if len == 0 {
        app.set_action(ActionState::Error("Command log is empty".into()));
        return;
    }
    let idxs: Vec<usize> = if app.cmdlog_marks.is_empty() {
        vec![app.cmdlog_cursor.min(len - 1)]
    } else {
        let mut v: Vec<usize> = app
            .cmdlog_marks
            .iter()
            .copied()
            .filter(|&i| i < len)
            .collect();
        v.sort_unstable();
        v
    };
    let text = idxs
        .iter()
        .map(|&i| cmd_log_line_text(&app.cmd_log[i], full))
        .collect::<Vec<_>>()
        .join("\n");
    let n = idxs.len();
    let secs = app.settings_cache.clipboard_clear_secs;
    match app.clipboard.write_with_clear(&text, secs) {
        Ok(()) => {
            let what = if full { "full" } else { "detail" };
            app.set_action(ActionState::Done(format!(
                "Copied {n} command-log line{} ({what})",
                if n == 1 { "" } else { "s" }
            )));
            app.push_cmd(
                "clipboard write",
                true,
                format!("{n} cmd-log line(s), {what}"),
            );
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("clipboard write", false, e.to_string());
        }
    }
}

/// Plain-text form of one command-log entry. `full` = the whole line; else
/// just the detail (the command's result text).
fn cmd_log_line_text(e: &crate::tui::action::CmdEntry, full: bool) -> String {
    if !full {
        return e.detail.clone();
    }
    let mark = if e.ok { "✓" } else { "✗" };
    let mut s = format!("{mark} {}  →  {}", e.cmd, e.detail);
    if let Some(d) = e.duration {
        s.push_str(&format!("  ({})", crate::domain::format_duration(d)));
    }
    s
}
