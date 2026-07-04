//! Push events from `keybase chat api-listen`.
//!
//! The listener subprocess emits one JSON object per chat notification;
//! the adapter parses each into a [`ChatEvent`] which the TUI applies
//! incrementally (bump a conversation, append a message, …) instead of
//! re-fetching the whole inbox / conversation.

use crate::domain::Message;

/// A parsed chat notification. Only the variants the TUI acts on are
/// modelled; everything else is dropped at the parser.
//
// `Message` is much larger than `NewConversation`, but these events are
// low-frequency (one per incoming chat message) and short-lived (parsed,
// sent over a channel, applied, dropped), so boxing the payload isn't
// worth the indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// An incoming (or echoed) message in some conversation. Edits,
    /// deletes and reactions arrive as messages too — their
    /// [`Message::content`] carries the specific kind.
    Message {
        /// `conversation_id` of the affected conversation — matches
        /// `Conversation::id` / `App::open_conv_id`.
        conv_id: String,
        /// The parsed message envelope.
        message: Message,
    },
    /// A new conversation was created or joined. Carries no payload —
    /// the TUI resolves it with a (silent) inbox resync, since the
    /// listener's conv summary doesn't carry everything the inbox needs.
    NewConversation,
    /// The `api-listen` stream ended (service restart, logout, crash).
    /// The listener supervisor respawns it with backoff; this event lets
    /// the UI log that real-time updates were interrupted meanwhile.
    StreamClosed,
}
