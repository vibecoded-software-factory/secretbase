//! Pure domain layer.
//!
//! This module contains the core entities of the Keybase TUI —
//! conversations, messages, teams, identity, filters, search ranking —
//! and nothing else. It has no knowledge of:
//!
//! * the Keybase CLI,
//! * the filesystem or any configuration format,
//! * the terminal, [`ratatui`](https://docs.rs/ratatui), or any rendering API.
//!
//! Because the layer is I/O-free it can be unit-tested without spawning
//! processes or mocking files. Every other layer in the crate
//! ([`crate::ports`], [`crate::adapters`], [`crate::tui`]) depends on
//! this one and never the other way around.

pub mod chat_event;
pub mod conversation;
pub mod duration;
pub mod emoji;
pub mod filter;
pub mod identity;
pub mod inbox_hit;
pub mod line_editor;
pub mod markdown;
pub mod mention;
pub mod message;
pub mod search;
pub mod team;
pub mod timefmt;
pub mod url;
pub mod validation;

pub use chat_event::ChatEvent;
pub use conversation::{
    Channel, Conversation, MemberStatus, MembersType, TopicType, conversation_label,
};
pub use duration::format_duration;
pub use emoji::Emoji;
pub use filter::{InboxSource, STATUS_FILTERS, StatusFilter};
pub use identity::IdentityInfo;
pub use inbox_hit::InboxHit;
pub use line_editor::LineEditor;
pub use markdown::{Run, parse_inline};
pub use mention::active_mention;
pub use message::{
    AttachmentInfo, Message, MessageContent, Reaction, SystemInfo, SystemKind, fold_edits,
    team_role_name,
};
pub use search::{LoweredConversation, fuzzy_score, fuzzy_score_lowered};
pub use team::{TeamMembership, TeamRole};
pub use timefmt::{message_time, relative_short};
pub use url::extract_urls;
pub use validation::{is_valid_keybase_identity, is_valid_keybase_username};
