//! Keybase chat message types.
//!
//! Keybase emits a heterogeneous stream of message types under
//! `content.type`. Rather than carrying the raw JSON we project each
//! known type into a typed variant of [`MessageContent`]; everything
//! else lands in [`MessageContent::Unknown`] with the original type
//! name preserved so the view can render a meaningful placeholder.
//!
//! Parsing happens in the adapter layer
//! ([`crate::adapters::keybase_cli`]) — this module only defines the
//! shapes.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Typed body of a single Keybase message.
///
/// Variants are listed in (roughly) decreasing order of how often they
/// show up in real-world inboxes.
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub enum MessageContent {
    /// A plain-text message — the user's literal body.
    Text(String),

    /// An edit of an earlier message.
    Edit {
        /// Id of the message being edited.
        target_id: u64,
        /// New body.
        body: String,
    },

    /// A deletion marker — references the ids of one or more deleted
    /// messages.
    Delete { target_ids: Vec<u64> },

    /// A reaction (emoji) on an existing message.
    Reaction {
        target_id: u64,
        /// Reaction body — typically a `:shortcode:` or raw emoji.
        body: String,
    },

    /// A file attachment uploaded to the conversation.
    Attachment(AttachmentInfo),

    /// A keybase system message — team membership changes, channel
    /// creation, gitpush, retention policy changes, etc.
    System(SystemInfo),

    /// A channel-metadata change (currently: conversation title).
    Metadata {
        /// New title set on the channel.
        title: String,
    },

    /// Long-form channel description (the "topic" / "headline").
    Headline { headline: String },

    /// A user pinned a message to the conversation.
    Pin { target_id: u64 },

    /// A user joined the channel.
    Join { joiner: String },

    /// A user left the channel.
    Leave { leaver: String },

    /// Stellar payment sent in chat. `text` is the pre-rendered
    /// `paymentText` field Keybase emits.
    SendPayment { text: String },

    /// Stellar payment request — pre-rendered `requestText`.
    RequestPayment { text: String },

    /// Catch-all for message types we don't render specially. Carries
    /// the original `content.type` string so the view can show a
    /// meaningful placeholder.
    Unknown { type_name: String },

    /// Default — no content parsed (initial state before parsing).
    #[default]
    Empty,
}

/// File attachment payload.
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct AttachmentInfo {
    /// User-supplied caption (often the body of the original message).
    pub title: String,
    /// Original filename as uploaded.
    pub filename: String,
    /// Size in bytes (uncompressed).
    #[zeroize(skip)]
    pub size: u64,
    /// MIME type as reported by the client.
    pub mime_type: String,
    /// Whether the upload finished successfully. Keybase emits a
    /// placeholder `attachment` row before the upload completes — the
    /// flag flips true once it does.
    #[zeroize(skip)]
    pub uploaded: bool,
}

/// System-message payload — captures the [`SystemKind`] and a
/// pre-rendered human-readable description.
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct SystemInfo {
    #[zeroize(skip)]
    pub kind: SystemKind,
    /// Description ready for the view to render (e.g.
    /// "alice added bob as writer").
    pub description: String,
}

/// Stable identifier for the system-message variant.
///
/// The numeric values mirror the `systemType` enum in the keybase
/// service for the most common cases. Anything not enumerated here
/// shows up as [`SystemKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemKind {
    AddedToTeam,
    InviteAddedToTeam,
    ComplexTeam,
    CreateTeam,
    GitPush,
    ChangeAvatar,
    ChangeRetention,
    BulkAddToConv,
    SbsResolve,
    NewChannel,
    #[default]
    Other,
}

impl SystemKind {
    /// Builds a [`SystemKind`] from the integer `systemType` field
    /// emitted by `keybase chat api`.
    pub fn from_int(n: i64) -> Self {
        match n {
            0 => SystemKind::AddedToTeam,
            1 => SystemKind::InviteAddedToTeam,
            2 => SystemKind::ComplexTeam,
            3 => SystemKind::CreateTeam,
            4 => SystemKind::GitPush,
            5 => SystemKind::ChangeAvatar,
            6 => SystemKind::ChangeRetention,
            7 => SystemKind::BulkAddToConv,
            8 => SystemKind::SbsResolve,
            9 => SystemKind::NewChannel,
            _ => SystemKind::Other,
        }
    }

    /// Short verb used in the renderer (e.g. `"team event"`).
    pub fn label(self) -> &'static str {
        match self {
            SystemKind::AddedToTeam | SystemKind::InviteAddedToTeam => "member",
            SystemKind::ComplexTeam => "team",
            SystemKind::CreateTeam => "team",
            SystemKind::GitPush => "git",
            SystemKind::ChangeAvatar => "avatar",
            SystemKind::ChangeRetention => "retention",
            SystemKind::BulkAddToConv => "members",
            SystemKind::SbsResolve => "invite",
            SystemKind::NewChannel => "channel",
            SystemKind::Other => "system",
        }
    }
}

/// Maps the integer role used inside `system.addedtoteam` to a
/// human-readable string. The mapping mirrors the team-role enum in
/// the keybase service.
pub fn team_role_name(n: i64) -> &'static str {
    match n {
        0 => "none",
        1 => "reader",
        2 => "writer",
        3 => "admin",
        4 => "owner",
        5 => "bot",
        6 => "restricted-bot",
        _ => "member",
    }
}

/// One message envelope — sender metadata + timestamps + typed body.
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct Message {
    /// 1-based message id within the conversation.
    pub id: u64,

    /// Sender username.
    pub sender: String,

    /// Sender device name.
    pub device: String,

    /// Unix-seconds of send time.
    #[zeroize(skip)]
    pub sent_at: u64,

    /// Unix-millis of send time.
    #[zeroize(skip)]
    pub sent_at_ms: u64,

    /// Typed body. [`MessageContent::Empty`] is the post-default
    /// state and should not appear after the adapter has parsed a
    /// message.
    pub content: MessageContent,

    /// Reactions attached to this message, projected from the
    /// service's `reactions.reactions` map. Empty when nobody has
    /// reacted yet.
    pub reactions: Vec<Reaction>,

    /// Message id this is a threaded reply to (`content.text.replyTo`),
    /// `None` for non-reply messages.
    #[zeroize(skip)]
    pub reply_to: Option<u64>,

    /// Whether this message was later edited — set by [`fold_edits`] when an
    /// `edit` message targeting it is folded in (its body replaced). Drives
    /// the dim `(edited)` indicator in the view.
    #[zeroize(skip)]
    pub edited: bool,
}

/// Folds keybase `edit` messages into their targets: the target message's body
/// is replaced with the latest edit's body and marked [`Message::edited`], and
/// the standalone `edit` envelopes are dropped — so the chat shows the edited
/// text in place (Discord-style) rather than a separate "edited" entry. An edit
/// whose target isn't in `messages` is left untouched.
pub fn fold_edits(messages: &mut Vec<Message>) {
    use std::collections::HashMap;
    // Latest edit body per target id (the highest edit message id wins).
    let mut edits: HashMap<u64, (u64, String)> = HashMap::new();
    for m in messages.iter() {
        if let MessageContent::Edit { target_id, body } = &m.content {
            let e = edits.entry(*target_id).or_insert((0, String::new()));
            if m.id >= e.0 {
                *e = (m.id, body.clone());
            }
        }
    }
    if edits.is_empty() {
        return;
    }
    // Apply edits to the targets that are actually present, tracking which
    // ones we folded so we only drop those edit envelopes (orphan edits whose
    // target isn't loaded stay as-is).
    let mut folded: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for m in messages.iter_mut() {
        if let Some((_, body)) = edits.get(&m.id) {
            m.content = MessageContent::Text(body.clone());
            m.edited = true;
            folded.insert(m.id);
        }
    }
    messages.retain(|m| match &m.content {
        MessageContent::Edit { target_id, .. } => !folded.contains(target_id),
        _ => true,
    });
}

/// Lightweight reaction summary as exposed to the view layer (kept
/// separate from raw [`MessageContent::Reaction`] envelopes so the
/// view doesn't depend on the JSON shape).
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct Reaction {
    pub emoji: String,
    pub usernames: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(id: u64, body: &str) -> Message {
        let mut m = Message::default();
        m.id = id;
        m.content = MessageContent::Text(body.into());
        m
    }
    fn edit(id: u64, target: u64, body: &str) -> Message {
        let mut m = Message::default();
        m.id = id;
        m.content = MessageContent::Edit {
            target_id: target,
            body: body.into(),
        };
        m
    }

    #[test]
    fn fold_edits_replaces_body_marks_edited_and_drops_envelope() {
        let mut msgs = vec![
            text(1, "hello"),
            text(2, "world"),
            edit(3, 1, "hello (fixed)"),
            edit(5, 1, "hello (fixed again)"), // latest edit wins
        ];
        fold_edits(&mut msgs);
        // The edit envelopes are gone; only the two originals remain.
        assert_eq!(msgs.len(), 2);
        let m1 = msgs.iter().find(|m| m.id == 1).unwrap();
        assert!(m1.edited);
        assert!(matches!(&m1.content, MessageContent::Text(b) if b == "hello (fixed again)"));
        let m2 = msgs.iter().find(|m| m.id == 2).unwrap();
        assert!(!m2.edited);
    }

    #[test]
    fn fold_edits_keeps_edit_when_target_missing() {
        let mut msgs = vec![text(2, "world"), edit(3, 99, "orphan")];
        fold_edits(&mut msgs);
        // The orphan edit (target not loaded) is left untouched.
        assert_eq!(msgs.len(), 2);
        assert!(
            msgs.iter()
                .any(|m| matches!(m.content, MessageContent::Edit { .. }))
        );
    }

    #[test]
    fn system_kind_round_trip() {
        for (n, k) in [
            (0, SystemKind::AddedToTeam),
            (3, SystemKind::CreateTeam),
            (4, SystemKind::GitPush),
            (9, SystemKind::NewChannel),
            (999, SystemKind::Other),
        ] {
            assert_eq!(SystemKind::from_int(n), k);
        }
    }

    #[test]
    fn role_name_known_values() {
        assert_eq!(team_role_name(1), "reader");
        assert_eq!(team_role_name(2), "writer");
        assert_eq!(team_role_name(3), "admin");
        assert_eq!(team_role_name(4), "owner");
        assert_eq!(team_role_name(99), "member");
    }
}
