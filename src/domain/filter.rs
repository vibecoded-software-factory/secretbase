//! Sidebar filters that narrow the conversation list.

use crate::domain::conversation::{Conversation, MemberStatus};

/// Filter applied to the conversation list before search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationFilter {
    /// Active conversations the user is still a member of.
    All,
    /// Conversations with `unread = true`.
    Unread,
    /// DMs only (impteam — one-to-one or small groups).
    Dms,
    /// Conversations inside an explicit team.
    Teams,
    /// Member-status != Active (left, removed, reset, preview).
    Archived,
}

/// Ordered set of all filters — drives sidebar rendering and arrow-key
/// cycling.
pub type ConversationFilters = [ConversationFilter; 5];

/// Filters in display order. The sidebar renders them top-to-bottom in
/// this exact sequence.
pub const CONVERSATION_FILTERS: ConversationFilters = [
    ConversationFilter::All,
    ConversationFilter::Unread,
    ConversationFilter::Dms,
    ConversationFilter::Teams,
    ConversationFilter::Archived,
];

impl ConversationFilter {
    /// Sidebar label.
    pub fn label(self) -> &'static str {
        match self {
            ConversationFilter::All => "All",
            ConversationFilter::Unread => "Unread",
            ConversationFilter::Dms => "DMs",
            ConversationFilter::Teams => "Teams",
            ConversationFilter::Archived => "Archived",
        }
    }

    /// Whether `conv` should appear in the list with this filter active.
    pub fn matches(self, conv: &Conversation) -> bool {
        let active = conv.member_status == MemberStatus::Active;
        match self {
            ConversationFilter::All => active,
            ConversationFilter::Unread => active && conv.unread,
            ConversationFilter::Dms => active && conv.channel.members_type.is_dm(),
            ConversationFilter::Teams => active && conv.channel.members_type.is_team(),
            ConversationFilter::Archived => !active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{Channel, MembersType, TopicType};

    fn conv(members: MembersType, status: MemberStatus, unread: bool) -> Conversation {
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
            member_status: status,
            creator_info: None,
        }
    }

    #[test]
    fn all_filter_matches_active_only() {
        let c1 = conv(MembersType::Team, MemberStatus::Active, false);
        let c2 = conv(MembersType::Team, MemberStatus::Left, false);
        assert!(ConversationFilter::All.matches(&c1));
        assert!(!ConversationFilter::All.matches(&c2));
    }

    #[test]
    fn unread_filter_requires_active_and_unread() {
        let c1 = conv(MembersType::Team, MemberStatus::Active, true);
        let c2 = conv(MembersType::Team, MemberStatus::Active, false);
        let c3 = conv(MembersType::Team, MemberStatus::Left, true);
        assert!(ConversationFilter::Unread.matches(&c1));
        assert!(!ConversationFilter::Unread.matches(&c2));
        assert!(!ConversationFilter::Unread.matches(&c3));
    }

    #[test]
    fn dms_filter_excludes_teams() {
        let c1 = conv(MembersType::ImpTeamNative, MemberStatus::Active, false);
        let c2 = conv(MembersType::Team, MemberStatus::Active, false);
        assert!(ConversationFilter::Dms.matches(&c1));
        assert!(!ConversationFilter::Dms.matches(&c2));
    }

    #[test]
    fn teams_filter_excludes_dms() {
        let c1 = conv(MembersType::Team, MemberStatus::Active, false);
        let c2 = conv(MembersType::ImpTeamNative, MemberStatus::Active, false);
        assert!(ConversationFilter::Teams.matches(&c1));
        assert!(!ConversationFilter::Teams.matches(&c2));
    }

    #[test]
    fn archived_filter_matches_inactive_member_status() {
        let c1 = conv(MembersType::Team, MemberStatus::Left, false);
        let c2 = conv(MembersType::Team, MemberStatus::Active, false);
        assert!(ConversationFilter::Archived.matches(&c1));
        assert!(!ConversationFilter::Archived.matches(&c2));
    }
}
