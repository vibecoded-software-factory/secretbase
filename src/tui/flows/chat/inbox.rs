//! Inbox-side flows: the conversation list (load / resync / mark-read),
//! the filter and popup lifecycles, local mute/favorite, per-conversation
//! status actions and label copying.

use crate::ports::KeybaseError;
use crate::ports::keybase::{ListConversationsOk, ReadChannel};
use crate::tui::action::ActionState;
use crate::tui::app::{App, ConvAction};
use crate::tui::worker::{InFlight, WorkerRequest};

#[allow(unused_imports)]
use super::*;

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
    app.submit(InFlight::LoadInbox, label, WorkerRequest::ListConversations);
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
            // The conversation being read can't be "unread" for its reader:
            // the server's read pointer may lag a beat behind the silent
            // mark-read the push path fires, and a resync landing in that
            // window would re-badge the open chat. Trust the local truth.
            if app.settings_cache.auto_mark_read
                && let Some(open) = app.open_conv_id.clone()
                && let Some(c) = app.conversations.iter_mut().find(|c| c.id == open)
            {
                c.unread = false;
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
        // The tree selection first; else the open chat — so the palette's
        // "Mark as read" works with the cursor on a group header too.
        let conv = app.selected_conversation().or_else(|| {
            app.open_conv_id
                .as_ref()
                .and_then(|id| app.conversations.iter().find(|c| &c.id == id))
        });
        let Some(conv) = conv else {
            app.set_action(ActionState::Error("No conversation selected".into()));
            return;
        };
        (read_channel_from_conv(conv), conv.id.clone())
    };
    let upto = match app.open_conv_id.as_deref() {
        Some(id) if id == this_conv_id => app.thread.messages.last().map(|m| m.id).unwrap_or(0),
        _ => 0,
    };
    let Some(channel) = resolve_channel_or_fail(app, channel_result) else {
        return;
    };
    // Carry the conversation *id*, not an index: a background inbox refresh
    // can reorder/replace `conversations` before the response lands, so an
    // index would then flip `unread` on the wrong row (or be out of bounds).
    app.submit(
        InFlight::MarkRead {
            conv_id: this_conv_id,
        },
        "Marking as read…",
        WorkerRequest::MarkRead {
            channel,
            message_id: upto,
        },
    );
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
    if !app.submit(
        InFlight::SetConvStatus {
            done_label: "Restored".to_string(),
        },
        "Restoring…",
        WorkerRequest::SetConvStatus {
            channel,
            status: "unfiled".to_string(),
        },
    ) {
        return;
    }
    // Close the popup now; `handle_set_conv_status_response` refreshes the inbox.
    app.unhide_input.clear();
    app.screen = crate::tui::screens::Screen::Inbox;
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
    app.submit(
        InFlight::SetConvStatus {
            done_label: done.to_string(),
        },
        running,
        WorkerRequest::SetConvStatus {
            channel,
            status: status.to_string(),
        },
    );
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

// ── Per-conversation action menu (right-click a tree row) ─────────────

/// One per-conversation action menu row: label + the handler it runs (each
/// targets the conversation under the tree cursor).
pub type ConvMenuAction = (&'static str, fn(&mut App));

fn act_open(a: &mut App) {
    tree_activate(a);
}
fn act_channels(a: &mut App) {
    super::open_channel_browser(a);
}
fn act_ignore(a: &mut App) {
    open_conv_action(a, ConvAction::Ignore);
}
fn act_block(a: &mut App) {
    open_conv_action(a, ConvAction::Block);
}
fn act_report(a: &mut App) {
    open_conv_action(a, ConvAction::Report);
}

/// The per-conversation action menu rows, in order. Both the menu view and its
/// activation read this array, so labels and actions can't drift.
pub const CONV_ACTIONS: [ConvMenuAction; 9] = [
    ("open", act_open),
    ("mark read", request_mark_read),
    ("mute / unmute", toggle_muted_conversation),
    ("favorite", toggle_favorite_conversation),
    ("copy name", do_copy_conversation_label),
    ("channels", act_channels),
    ("ignore", act_ignore),
    ("block", act_block),
    ("report", act_report),
];

/// Opens the per-conversation action menu (`Screen::ConvActions`) for the tree
/// `row` — right-click. Seats the tree cursor on it so the actions target it.
pub fn open_conv_actions(app: &mut App, row: usize) {
    app.tree_selected = row;
    app.conv_actions_selected = 0;
    app.screen = crate::tui::screens::Screen::ConvActions;
}

/// Closes the menu back to the inbox without running an action.
pub fn close_conv_actions(app: &mut App) {
    app.screen = crate::tui::screens::Screen::Inbox;
}

/// Runs the highlighted menu action: closes the menu first, then dispatches
/// (the action may open its own overlay — ignore/block/report confirm, the
/// channel browser — or mutate local state — mute / favourite).
pub fn run_conv_action(app: &mut App) {
    let idx = app.conv_actions_selected.min(CONV_ACTIONS.len() - 1);
    let action = CONV_ACTIONS[idx].1;
    close_conv_actions(app);
    action(app);
}

/// Copies the marked command-log lines (or the cursor line if none are
/// marked) to the clipboard. `full` copies the whole line (`✓ cmd → detail
/// (dur)`); otherwise just the `detail`. The selection is kept so the user
/// can copy it both ways.
pub fn do_copy_cmd_log(app: &mut App, full: bool) {
    let len = app.cmdlog.entries.len();
    if len == 0 {
        app.set_action(ActionState::Error("Command log is empty".into()));
        return;
    }
    let idxs: Vec<usize> = if app.cmdlog.marks.is_empty() {
        vec![app.cmdlog.cursor.min(len - 1)]
    } else {
        let mut v: Vec<usize> = app
            .cmdlog
            .marks
            .iter()
            .copied()
            .filter(|&i| i < len)
            .collect();
        v.sort_unstable();
        v
    };
    let text = idxs
        .iter()
        .map(|&i| cmd_log_line_text(&app.cmdlog.entries[i], full))
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
