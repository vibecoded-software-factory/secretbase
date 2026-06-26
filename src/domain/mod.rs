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

pub mod conversation;
pub mod duration;
pub mod filter;
pub mod identity;
pub mod inbox_hit;
pub mod line_editor;
pub mod message;
pub mod search;
pub mod team;
pub mod validation;

pub use conversation::{
    Channel, Conversation, MemberStatus, MembersType, TopicType, conversation_label,
};
pub use duration::format_duration;
pub use filter::{CONVERSATION_FILTERS, ConversationFilter, ConversationFilters};
pub use identity::IdentityInfo;
pub use inbox_hit::InboxHit;
pub use line_editor::LineEditor;
pub use message::{
    AttachmentInfo, Message, MessageContent, Reaction, SystemInfo, SystemKind, team_role_name,
};
pub use search::{LoweredConversation, fuzzy_score, fuzzy_score_lowered};
pub use team::{TeamMembership, TeamRole};
pub use validation::{is_valid_keybase_identity, is_valid_keybase_username};
