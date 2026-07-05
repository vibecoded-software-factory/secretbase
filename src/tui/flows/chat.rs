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

use crate::domain::Message;
use crate::ports::keybase::ReadChannel;
use crate::tui::action::ActionState;
use crate::tui::app::App;

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
fn project_messages(msgs: Vec<Message>, smart_joins: bool) -> Vec<Message> {
    use crate::domain::MessageContent;
    let mut out: Vec<Message> = msgs
        .into_iter()
        .filter(|m| !matches!(m.content, MessageContent::Reaction { .. }))
        // A giphy unfurl card duplicates what the reader already sees: the
        // GIF renders inline above it (web previews on) or the giphy URL is
        // right there in the text (off) — either way, noise. Generic site
        // cards survive: their title is real information.
        .filter(|m| !matches!(&m.content, MessageContent::Unfurl { label } if label == "GIPHY"))
        .collect();
    crate::domain::fold_edits(&mut out);
    crate::domain::fold_deletes(&mut out);
    // weechat's smart filter: hide join/leave chatter except from people
    // who actually spoke in the loaded window — membership noise drowns
    // conversation in busy team channels. Hidden, not deleted: toggling
    // the setting reprojects them back on the next load.
    if smart_joins {
        let speakers: std::collections::HashSet<String> = out
            .iter()
            .filter(|m| {
                matches!(
                    m.content,
                    MessageContent::Text(_)
                        | MessageContent::Edit { .. }
                        | MessageContent::Attachment(_)
                )
            })
            .map(|m| m.sender.to_ascii_lowercase())
            .collect();
        out.retain(|m| match &m.content {
            MessageContent::Join { joiner } => speakers.contains(&joiner.to_ascii_lowercase()),
            MessageContent::Leave { leaver } => speakers.contains(&leaver.to_ascii_lowercase()),
            _ => true,
        });
    }
    out
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

// The per-area flow modules. Everything public is re-exported flat so call
// sites keep addressing `flows::chat::*` — the split is file organisation,
// not an API change.
mod attachments;
mod channels;
mod inbox;
mod messages;
mod search;

pub use attachments::*;
pub use channels::*;
pub use inbox::*;
pub use messages::*;
pub use search::*;

#[cfg(test)]
mod tests;
