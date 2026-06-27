//! Inbox filters. Two independent axes intersect:
//!
//! * a **status** axis ([`StatusFilter`]: All / Unread), and
//! * a **source** axis ([`InboxSource`]: Direct messages, or one team) —
//!   a Discord-style picker listing DMs and each team separately.
//!
//! Neither checks active-membership; the caller
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

/// Source axis: which space the inbox is showing — all DMs, or one team's
/// channels. Built dynamically from the loaded conversations (one entry per
/// team), so it can't be a fixed enum like [`StatusFilter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxSource {
    /// All direct messages (non-team conversations).
    Dms,
    /// One specific team — shows its channels. The `String` is the team
    /// name (`Channel::name` for team conversations).
    Team(String),
}

impl InboxSource {
    /// Sidebar label.
    pub fn label(&self) -> &str {
        match self {
            InboxSource::Dms => "Direct messages",
            InboxSource::Team(name) => name,
        }
    }

    /// Whether `conv` belongs to this source (no active-membership check).
    /// "Direct messages" is the catch-all for everything that isn't a team,
    /// so conversations with an unknown/future members-type stay visible.
    pub fn includes(&self, conv: &Conversation) -> bool {
        match self {
            InboxSource::Dms => !conv.channel.members_type.is_team(),
            InboxSource::Team(name) => {
                conv.channel.members_type.is_team() && &conv.channel.name == name
            }
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

    #[test]
    fn source_dms_matches_only_dms() {
        let dm = conv("alice,bob", MembersType::ImpTeamNative, false);
        let team = conv("acme", MembersType::Team, false);
        assert!(InboxSource::Dms.includes(&dm));
        assert!(!InboxSource::Dms.includes(&team));
    }

    #[test]
    fn source_team_matches_only_its_own_channels() {
        let acme = conv("acme", MembersType::Team, false);
        let other = conv("globex", MembersType::Team, false);
        let dm = conv("alice,bob", MembersType::ImpTeamNative, false);
        let src = InboxSource::Team("acme".into());
        assert!(src.includes(&acme));
        assert!(!src.includes(&other));
        assert!(!src.includes(&dm));
    }
}
