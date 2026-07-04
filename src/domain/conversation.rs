//! Keybase chat conversation types — direct mirror of the `keybase chat
//! api list` JSON shape.
//!
//! The Keybase service emits a stable JSON schema for every inbox
//! conversation. We deserialize it into these types so the rest of the
//! application can pattern-match on Rust enums instead of stringly-typed
//! JSON.

use serde::Deserialize;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// What kind of conversation this is.
///
/// `impteamnative` is Keybase's implicit team — created on-demand when
/// you message a user (or set of users) directly. `team` is an explicit
/// named team. The remaining variants are rare/legacy but emitted by
/// the service so we round-trip them faithfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MembersType {
    /// Implicit team — created on the fly from a comma-separated list
    /// of usernames (e.g. "alice,bob").
    #[serde(rename = "impteamnative")]
    ImpTeamNative,
    /// Implicit team upgraded to a real team.
    #[serde(rename = "impteamupgrade")]
    ImpTeamUpgrade,
    /// An explicit Keybase team.
    Team,
    /// Single-user "self conversation" (notes-to-self).
    #[serde(rename = "kbfs")]
    Kbfs,
    /// Fallback — unknown member type.
    #[serde(other)]
    Unknown,
}

impl MembersType {
    /// Human-readable label for sidebar / tags.
    pub fn label(self) -> &'static str {
        match self {
            MembersType::ImpTeamNative | MembersType::ImpTeamUpgrade => "DM",
            MembersType::Team => "TEAM",
            MembersType::Kbfs => "SELF",
            MembersType::Unknown => "?",
        }
    }

    /// Whether this conversation lives inside a Keybase team.
    pub fn is_team(self) -> bool {
        matches!(self, MembersType::Team)
    }
}

/// Whether the local user is still a member of the conversation.
///
/// Keybase keeps "left", "removed", "reset", "preview" conversations
/// in the inbox; the parser accepts them all, and the UI surfaces only
/// `Active` memberships today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberStatus {
    Active,
    Removed,
    Left,
    Preview,
    Reset,
    #[serde(other)]
    Unknown,
}

/// The "channel" sub-object emitted by `keybase chat api`.
///
/// For DMs, `name` is a comma-separated list of usernames including
/// the caller. For teams, `name` is the team name and `topic_name`
/// is the channel inside it.
#[derive(Debug, Clone, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Channel {
    /// `"alice,bob"` for impteam, `"phoenix"` for team.
    pub name: String,
    /// `impteamnative` / `team` / `kbfs` / …
    #[zeroize(skip)]
    #[serde(default = "default_members_type")]
    pub members_type: MembersType,
    /// Sub-channel within a team (e.g. `general`, `random`). Absent
    /// for DMs.
    #[serde(default)]
    pub topic_name: Option<String>,
}

fn default_members_type() -> MembersType {
    MembersType::Unknown
}

/// The "creator_info" sub-object — only present on conversations
/// the user did not initiate themselves.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatorInfo {
    /// Username of the creator.
    pub username: String,
}

/// One entry of the inbox list — corresponds to a single
/// conversation/channel pair.
///
/// `keybase chat api {"method":"list"}` returns
/// `result.conversations[]` shaped like this struct.
#[derive(Debug, Clone, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Conversation {
    /// 64-char hex conversation id — opaque, used for paging/marking.
    pub id: String,

    pub channel: Channel,

    /// Whether the conversation has unread messages.
    #[zeroize(skip)]
    #[serde(default)]
    pub unread: bool,

    /// Unix-seconds of last activity.
    #[zeroize(skip)]
    #[serde(default)]
    pub active_at: u64,

    /// Unix-millis of last activity (more precise than `active_at`).
    #[zeroize(skip)]
    #[serde(default)]
    pub active_at_ms: u64,

    /// Member-status of the local user.
    #[zeroize(skip)]
    #[serde(default = "default_member_status")]
    pub member_status: MemberStatus,

    /// Creator metadata. Absent on conversations the local user
    /// started.
    #[zeroize(skip)]
    #[serde(default)]
    pub creator_info: Option<CreatorInfo>,
}

impl Conversation {
    /// Whether this is a **note-to-self** DM: a non-team conversation whose
    /// only participant is `me`. Such a conversation can never be genuinely
    /// unread — Keybase's `list` nonetheless flags it unread whenever your own
    /// edits/deletes/reactions push the latest message id past your read
    /// pointer, which shows up as a phantom badge.
    pub fn is_self_dm(&self, me: &str) -> bool {
        if self.channel.members_type.is_team() || me.is_empty() {
            return false;
        }
        let mut any = false;
        for u in self.channel.name.split(',') {
            let u = u.trim();
            if u.is_empty() {
                continue;
            }
            any = true;
            if u != me {
                return false;
            }
        }
        any
    }
}

fn default_member_status() -> MemberStatus {
    MemberStatus::Active
}

/// User-visible label for a conversation.
///
/// For DMs, returns the comma-separated participant list with the
/// local user's name stripped (when supplied). For teams, returns
/// `team#channel`.
pub fn conversation_label(conv: &Conversation, me: Option<&str>) -> String {
    match conv.channel.members_type {
        MembersType::Team => match conv.channel.topic_name.as_deref() {
            Some(topic) => format!("{}#{}", conv.channel.name, topic),
            None => conv.channel.name.clone(),
        },
        _ => {
            let Some(me) = me else {
                return conv.channel.name.clone();
            };
            let parts: Vec<&str> = conv
                .channel
                .name
                .split(',')
                .filter(|p| !p.is_empty() && *p != me)
                .collect();
            if parts.is_empty() {
                // self-conversation.
                me.to_string()
            } else {
                parts.join(",")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_conv(json: &str) -> Conversation {
        serde_json::from_str::<Conversation>(json).expect("valid conv json")
    }

    #[test]
    fn members_type_label_team() {
        assert_eq!(MembersType::Team.label(), "TEAM");
    }

    #[test]
    fn is_self_dm_detects_note_to_self() {
        let dm = |name: &str, mt: &str| {
            parse_conv(&format!(
                r#"{{"id":"x","channel":{{"name":"{name}","members_type":"{mt}","topic_type":"chat"}},
                    "unread":true,"active_at":1,"active_at_ms":1,"member_status":"active"}}"#
            ))
        };
        // Only yourself → note-to-self.
        assert!(dm("me", "impteamnative").is_self_dm("me"));
        // A real DM with someone else → not.
        assert!(!dm("me,alice", "impteamnative").is_self_dm("me"));
        assert!(!dm("alice", "impteamnative").is_self_dm("me"));
        // A team channel is never a self-DM.
        assert!(!dm("me", "team").is_self_dm("me"));
        // Empty self guards against false positives.
        assert!(!dm("me", "impteamnative").is_self_dm(""));
    }

    #[test]
    fn parses_impteam_conversation() {
        let json = r#"{
            "id":"00aa","channel":{"name":"alice,bob","members_type":"impteamnative","topic_type":"chat"},
            "is_default_conv":true,"unread":false,"active_at":1,"active_at_ms":1000,
            "member_status":"active","creator_info":{"ctime":1,"username":"bob"}
        }"#;
        let c = parse_conv(json);
        assert_eq!(c.channel.members_type, MembersType::ImpTeamNative);
    }

    #[test]
    fn parses_team_conversation_with_topic() {
        let json = r#"{
            "id":"00bb","channel":{"name":"phoenix","members_type":"team","topic_type":"chat","topic_name":"general"},
            "unread":true,"active_at":2,"active_at_ms":2000,"member_status":"active"
        }"#;
        let c = parse_conv(json);
        assert_eq!(c.channel.members_type, MembersType::Team);
        assert_eq!(c.channel.topic_name.as_deref(), Some("general"));
        assert!(c.unread);
    }

    #[test]
    fn label_strips_self_from_dm() {
        let json = r#"{
            "id":"00cc","channel":{"name":"alice,bob,charlie","members_type":"impteamnative","topic_type":"chat"}
        }"#;
        let c = parse_conv(json);
        assert_eq!(conversation_label(&c, Some("alice")), "bob,charlie");
    }

    #[test]
    fn label_uses_team_channel_format() {
        let json = r#"{
            "id":"00dd","channel":{"name":"phoenix","members_type":"team","topic_type":"chat","topic_name":"random"}
        }"#;
        let c = parse_conv(json);
        assert_eq!(conversation_label(&c, Some("alice")), "phoenix#random");
    }
}
