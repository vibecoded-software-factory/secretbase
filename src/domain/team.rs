//! Keybase team membership types.

use serde::Deserialize;

/// Role the local user holds in a team, in increasing order of
/// privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamRole {
    None,
    Reader,
    Writer,
    Admin,
    Owner,
    Bot,
    #[serde(rename = "restrictedbot")]
    RestrictedBot,
    #[serde(other)]
    Unknown,
}

impl TeamRole {
    /// Human-readable single-word label.
    pub fn label(self) -> &'static str {
        match self {
            TeamRole::None => "none",
            TeamRole::Reader => "reader",
            TeamRole::Writer => "writer",
            TeamRole::Admin => "admin",
            TeamRole::Owner => "owner",
            TeamRole::Bot => "bot",
            TeamRole::RestrictedBot => "restricted-bot",
            TeamRole::Unknown => "?",
        }
    }
}

/// A team the local user belongs to — one entry of the
/// `list-self-memberships` response.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamMembership {
    /// Fully-qualified team name (e.g. `phoenix` or `phoenix.bots`).
    #[serde(rename = "fq_name")]
    pub name: String,

    /// Whether the team is a sub-team of a parent team.
    #[serde(default)]
    pub is_implicit_team: bool,

    /// Number of active members.
    #[serde(default)]
    pub member_count: u32,

    /// Role of the local user within the team.
    #[serde(default = "default_role")]
    pub role: TeamRole,
}

fn default_role() -> TeamRole {
    TeamRole::Unknown
}
