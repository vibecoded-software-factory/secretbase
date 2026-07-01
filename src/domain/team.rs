//! Keybase team membership types.

use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

/// Role the local user holds in a team, in increasing order of
/// privilege.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamRole {
    None,
    Reader,
    Writer,
    Admin,
    Owner,
    Bot,
    RestrictedBot,
    Unknown,
}

/// `keybase team api` is inconsistent about how it encodes a role: some
/// replies use the lowercase **string** name (`"owner"`), others the raw
/// `keybase1.TeamRole` **integer** ordinal (`4`). A serde-derived enum only
/// accepts one shape and rejects the other ("invalid type: integer `4`,
/// expected string or map"), which dropped the whole team row and emptied the
/// Teams view. Deserialize both by hand, mapping to [`TeamRole::Unknown`] for
/// anything unrecognized.
impl<'de> Deserialize<'de> for TeamRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RoleVisitor;

        impl Visitor<'_> for RoleVisitor {
            type Value = TeamRole;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a team role as a lowercase string or an integer ordinal")
            }

            fn visit_str<E: de::Error>(self, s: &str) -> Result<TeamRole, E> {
                Ok(match s.to_ascii_lowercase().as_str() {
                    "none" => TeamRole::None,
                    "reader" => TeamRole::Reader,
                    "writer" => TeamRole::Writer,
                    "admin" => TeamRole::Admin,
                    "owner" => TeamRole::Owner,
                    "bot" => TeamRole::Bot,
                    "restrictedbot" => TeamRole::RestrictedBot,
                    _ => TeamRole::Unknown,
                })
            }

            fn visit_u64<E: de::Error>(self, n: u64) -> Result<TeamRole, E> {
                // keybase1.TeamRole ordinals (verified against protocol source).
                Ok(match n {
                    0 => TeamRole::None,
                    1 => TeamRole::Reader,
                    2 => TeamRole::Writer,
                    3 => TeamRole::Admin,
                    4 => TeamRole::Owner,
                    5 => TeamRole::Bot,
                    6 => TeamRole::RestrictedBot,
                    _ => TeamRole::Unknown,
                })
            }

            fn visit_i64<E: de::Error>(self, n: i64) -> Result<TeamRole, E> {
                if n < 0 {
                    Ok(TeamRole::Unknown)
                } else {
                    self.visit_u64(n as u64)
                }
            }
        }

        deserializer.deserialize_any(RoleVisitor)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_from_lowercase_string() {
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!("owner")).unwrap(),
            TeamRole::Owner
        );
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!("restrictedbot")).unwrap(),
            TeamRole::RestrictedBot
        );
    }

    #[test]
    fn role_from_integer_ordinal() {
        // keybase1.TeamRole: 4 = owner, 2 = writer (the ints that broke Teams).
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!(4)).unwrap(),
            TeamRole::Owner
        );
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!(2)).unwrap(),
            TeamRole::Writer
        );
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!(0)).unwrap(),
            TeamRole::None
        );
    }

    #[test]
    fn role_unknown_falls_back() {
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!("wizard")).unwrap(),
            TeamRole::Unknown
        );
        assert_eq!(
            serde_json::from_value::<TeamRole>(serde_json::json!(99)).unwrap(),
            TeamRole::Unknown
        );
    }

    #[test]
    fn membership_row_parses_with_integer_role() {
        // The exact shape that emptied the Teams view: role as an integer.
        let row = serde_json::json!({
            "fq_name": "backfly",
            "member_count": 7,
            "role": 4,
        });
        let m: TeamMembership = serde_json::from_value(row).unwrap();
        assert_eq!(m.name, "backfly");
        assert_eq!(m.member_count, 7);
        assert_eq!(m.role, TeamRole::Owner);
    }
}
