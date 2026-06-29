//! Inbox filters.
//!
//! The **status** axis ([`StatusFilter`]: All / Unread) narrows the
//! conversation list by read-state. The source grouping (Direct messages
//! vs. each team) lives in the conversation tree itself
//! ([`crate::tui::app::App::tree_rows`]), not here.
//!
//! [`StatusFilter::includes`] does not check active-membership; the caller
//! ([`crate::tui::app::App::rebuild_filter`]) applies that once.

use crate::domain::conversation::Conversation;

/// Status axis: read-state filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    /// Every conversation in the current source.
    All,
    /// Only those with `unread = true`.
    Unread,
}

/// Status filters in sidebar (top-to-bottom) order.
pub const STATUS_FILTERS: [StatusFilter; 2] = [StatusFilter::All, StatusFilter::Unread];

impl StatusFilter {
    /// Whether `conv` passes the status axis (no active-membership check).
    pub fn includes(self, conv: &Conversation) -> bool {
        match self {
            StatusFilter::All => true,
            StatusFilter::Unread => conv.unread,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{Channel, MemberStatus, MembersType, TopicType};

    fn conv(name: &str, members: MembersType, unread: bool) -> Conversation {
        Conversation {
            id: name.into(),
            channel: Channel {
                name: name.into(),
                members_type: members,
                topic_type: TopicType::Chat,
                topic_name: None,
                public: false,
            },
            is_default_conv: true,
            unread,
            active_at: 0,
            active_at_ms: 0,
            member_status: MemberStatus::Active,
            creator_info: None,
        }
    }

    #[test]
    fn status_axis() {
        let unread = conv("a", MembersType::Team, true);
        let read = conv("a", MembersType::Team, false);
        assert!(StatusFilter::All.includes(&read));
        assert!(StatusFilter::Unread.includes(&unread));
        assert!(!StatusFilter::Unread.includes(&read));
    }
}
