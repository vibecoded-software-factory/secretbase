//! Application flows — free functions that mutate [`crate::tui::App`].
//!
//! Each module owns one logical area of the UI (auth, chat, clipboard,
//! …). Every flow that talks to the `keybase` CLI follows the
//! request/response split:
//!
//! * `request_*` runs synchronously, validates state, captures the
//!   parameters the worker needs, sets [`InFlight`], and ships a
//!   [`WorkerRequest`] through `App::worker_tx`.
//! * `handle_*_response` runs when the matching [`WorkerResponse`]
//!   comes back. The unified dispatcher [`apply_response`] picks the
//!   right handler by matching on `App::in_flight`.

pub mod auth;
pub mod chat;
pub mod copy;
pub mod teams;

use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::worker::{InFlight, WorkerResponse};

/// Applies a push event from the `keybase chat api-listen` stream.
///
/// This is a *push*, not a request/response — it carries no `InFlight`
/// ticket and never touches the user's slot, so it can land at any time
/// (even mid-request). It updates the inbox and the open conversation
/// incrementally; a brand-new conversation triggers a silent resync.
pub fn apply_chat_event(app: &mut App, event: crate::domain::ChatEvent) {
    use crate::domain::ChatEvent;
    match event {
        ChatEvent::Message { conv_id, message } => {
            chat::handle_incoming_message(app, conv_id, message);
        }
        ChatEvent::NewConversation => {
            // The listener's conv summary is thinner than an inbox row,
            // so pull the new conversation in with a silent resync.
            chat::request_load_inbox_silent(app);
        }
    }
}

/// Routes a worker response to the matching `handle_*_response`
/// based on the `InFlight` context.
///
/// Mismatches (response of a kind the in-flight slot wasn't expecting,
/// or a response while idle) are quietly dropped after logging a
/// `cmd_log` entry — they should not happen given the single-shot
/// dispatch model, but a stale response from a re-spawned worker
/// would otherwise hit a wrong-handler.
pub fn apply_response(app: &mut App, response: WorkerResponse) {
    // Background-lane responses are routed by variant BEFORE the
    // in_flight match: they belong to the idle auto-refresh, which uses
    // `bg_inflight` (never the user's `in_flight` slot), so they must
    // not consume or collide with a user request in flight.
    let response = match response {
        WorkerResponse::ListConversationsSilent(r) => {
            app.bg_inflight = false;
            // Stamp the background op's elapsed time for its cmd_log row.
            app.last_op_elapsed = app.bg_started.take().map(|t| t.elapsed());
            chat::handle_load_inbox_response(app, r, true);
            return;
        }
        // Background emoji-catalogue fetch for the reaction picker — also
        // routed by variant (no `in_flight` ticket).
        WorkerResponse::Emojis(r) => {
            // Stamp the fetch's elapsed time so its cmd_log row carries it,
            // like every other keybase op.
            app.last_op_elapsed = app.emojis_started.take().map(|t| t.elapsed());
            chat::handle_emojis_response(app, r);
            return;
        }
        other => other,
    };

    let Some(in_flight) = app.in_flight.take() else {
        app.push_cmd(
            "worker response",
            false,
            "received without an in-flight slot — dropped",
        );
        return;
    };

    // Stamp the operation's elapsed time so the handler's first
    // `push_cmd` carries it (request → response, as the user perceives
    // it). `push_cmd` consumes it, so later rows in the same handler
    // (parse warnings, …) stay unstamped.
    app.last_op_elapsed = app.request_started.take().map(|t| t.elapsed());

    match (in_flight, response) {
        // ── Status ────────────────────────────────────────────────
        (InFlight::BootStatus, WorkerResponse::Status(r)) => {
            auth::handle_status_response(app, r, true);
        }
        (InFlight::CheckStatus, WorkerResponse::Status(r)) => {
            auth::handle_status_response(app, r, false);
        }
        // ── Logout ────────────────────────────────────────────────
        (InFlight::Logout, WorkerResponse::Logout(r)) => {
            auth::handle_logout_response(app, r);
        }
        // ── Inbox load ────────────────────────────────────────────
        (InFlight::LoadInbox, WorkerResponse::ListConversations(r)) => {
            chat::handle_load_inbox_response(app, r, false);
        }
        // ── Messages ──────────────────────────────────────────────
        (InFlight::LoadMessages, WorkerResponse::ReadMessages(r)) => {
            chat::handle_load_messages_response(app, r);
        }
        (InFlight::LoadOlderMessages, WorkerResponse::ReadMessages(r)) => {
            chat::handle_load_older_messages_response(app, r);
        }
        // ── Mark read ─────────────────────────────────────────────
        (InFlight::MarkRead { conv_idx }, WorkerResponse::MarkRead(r)) => {
            chat::handle_mark_read_response(app, r, conv_idx);
        }
        // ── Search inbox ──────────────────────────────────────────
        (InFlight::SearchInboxRemote, WorkerResponse::SearchInboxHits(r)) => {
            chat::handle_search_inbox_response(app, r);
        }
        (InFlight::ConvSearch, WorkerResponse::SearchRegexp(r)) => {
            chat::handle_conv_search_response(app, r);
        }
        // ── Send / edit / delete / react ──────────────────────────
        (
            InFlight::SendMessage {
                body_len,
                was_reply,
            },
            WorkerResponse::SendMessage(r),
        ) => {
            chat::handle_send_message_response(app, r, body_len, was_reply);
        }
        (InFlight::EditMessage { target_id }, WorkerResponse::EditMessage(r)) => {
            chat::handle_save_edit_response(app, r, target_id);
        }
        (InFlight::DeleteMessage { message_id }, WorkerResponse::DeleteMessage(r)) => {
            chat::handle_delete_response(app, r, message_id);
        }
        (InFlight::SendReaction { body }, WorkerResponse::React(r)) => {
            chat::handle_react_response(app, r, body);
        }
        // ── New conversation ──────────────────────────────────────
        (InFlight::NewConversation, WorkerResponse::NewConversation(r)) => {
            chat::handle_new_conversation_response(app, r);
        }
        // ── Conversation status (mute/unmute/ignore/block/…) ──────
        (InFlight::SetConvStatus { done_label }, WorkerResponse::SetConvStatus(r)) => {
            chat::handle_set_conv_status_response(app, r, &done_label);
        }
        // ── Pin / unpin ───────────────────────────────────────────
        (InFlight::PinMessage { message_id }, WorkerResponse::PinMessage(r)) => {
            chat::handle_pin_response(app, r, message_id);
        }
        (InFlight::UnpinConversation, WorkerResponse::UnpinMessage(r)) => {
            chat::handle_unpin_response(app, r);
        }
        // ── Download attachment ───────────────────────────────────
        (
            InFlight::DownloadAttachment { message_id, path },
            WorkerResponse::DownloadAttachment(r),
        ) => {
            chat::handle_download_attachment_response(app, r, message_id, path);
        }
        (InFlight::UploadAttachment { filename }, WorkerResponse::UploadAttachment(r)) => {
            chat::handle_upload_response(app, r, filename);
        }
        // ── Teams ─────────────────────────────────────────────────
        (InFlight::LoadTeams, WorkerResponse::ListSelfMemberships(r)) => {
            teams::handle_load_teams_response(app, r);
        }
        // ── Mismatch (defensive) ──────────────────────────────────
        (slot, _) => {
            // Should never happen: in-flight slot and response kind
            // disagree. Reset the strip so the user is not stuck
            // looking at a spinner forever.
            app.set_action(ActionState::Error(
                "internal worker dispatch mismatch".into(),
            ));
            app.push_cmd(
                "worker response",
                false,
                format!("dispatch mismatch (in_flight: {slot:?})"),
            );
        }
    }
}
