//! Sidebar filters — **two independent axes** that intersect: a *status*
//! axis (All / Unread) and a *type* axis (All / DMs / Teams). The inbox
//! shows conversations matching the active status **and** the active type.
//!
//! Neither axis checks active-membership; the caller
//! ([`crate::tui::app::App::rebuild_filter`]) applies that once across both.

use crate::domain::conversation::Conversation;

/// Status axis: read-state filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    /// Every active conversation.
    All,
    /// Conversations with `unread = true`.
    Unread,
}

/// Type axis: conversation-kind filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter {
    /// Every kind (DMs + teams).
    All,
    /// DMs only (impteam — one-to-one or small groups).
    Dms,
    /// Conversations inside an explicit team.
    Teams,
}

/// Status filters in sidebar (top-to-bottom) order.
pub const STATUS_FILTERS: [StatusFilter; 2] = [StatusFilter::All, StatusFilter::Unread];

/// Type filters in sidebar (top-to-bottom) order.
pub const TYPE_FILTERS: [TypeFilter; 3] = [TypeFilter::All, TypeFilter::Dms, TypeFilter::Teams];

impl StatusFilter {
    /// Sidebar label.
    pub fn label(self) -> &'static str {
        match self {
            StatusFilter::All => "All",
            StatusFilter::Unread => "Unread",
        }
    }

    /// Whether `conv` passes the status axis (no active-membership check).
    pub fn includes(self, conv: &Conversation) -> bool {
        match self {
            StatusFilter::All => true,
            StatusFilter::Unread => conv.unread,
        }
    }
}

impl TypeFilter {
    /// Sidebar label.
    pub fn label(self) -> &'static str {
        match self {
            TypeFilter::All => "All",
            TypeFilter::Dms => "DMs",
            TypeFilter::Teams => "Teams",
        }
    }

    /// Whether `conv` passes the type axis (no active-membership check).
    pub fn includes(self, conv: &Conversation) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Dms => conv.channel.members_type.is_dm(),
            TypeFilter::Teams => conv.channel.members_type.is_team(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{Channel, MemberStatus, MembersType, TopicType};

    fn conv(members: MembersType, unread: bool) -> Conversation {
        Conversation {
            id: "x".into(),
            channel: Channel {
                name: "x".into(),
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
        let unread = conv(MembersType::Team, true);
        let read = conv(MembersType::Team, false);
        assert!(StatusFilter::All.includes(&read));
        assert!(StatusFilter::All.includes(&unread));
        assert!(StatusFilter::Unread.includes(&unread));
        assert!(!StatusFilter::Unread.includes(&read));
    }

    #[test]
    fn type_axis() {
        let dm = conv(MembersType::ImpTeamNative, false);
        let team = conv(MembersType::Team, false);
        assert!(TypeFilter::All.includes(&dm));
        assert!(TypeFilter::All.includes(&team));
        assert!(TypeFilter::Dms.includes(&dm));
        assert!(!TypeFilter::Dms.includes(&team));
        assert!(TypeFilter::Teams.includes(&team));
        assert!(!TypeFilter::Teams.includes(&dm));
    }

    #[test]
    fn axes_intersect() {
        let unread_dm = conv(MembersType::ImpTeamNative, true);
        let read_dm = conv(MembersType::ImpTeamNative, false);
        let unread_team = conv(MembersType::Team, true);
        let pass =
            |c: &Conversation| StatusFilter::Unread.includes(c) && TypeFilter::Dms.includes(c);
        assert!(pass(&unread_dm));
        assert!(!pass(&read_dm));
        assert!(!pass(&unread_team));
    }
}
