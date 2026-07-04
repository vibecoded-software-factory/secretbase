//! Local-user identity snapshot — output of `keybase status --json`.
//!
//! Only the handful of fields the TUI actually displays are kept.
//! The full status object is large and many fields are
//! Linux/Windows-specific.

use serde::Deserialize;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Snapshot of the local Keybase session.
///
/// Unlike Bitwarden, the Keybase CLI authenticates against a
/// long-running local `keybased` service: there is no session key the
/// adapter has to carry around. The TUI simply asks the service
/// whether it is logged in.
#[derive(Debug, Clone, Default, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct IdentityInfo {
    /// Username of the active account (empty when logged out).
    #[serde(rename = "Username", default)]
    pub username: String,

    /// Default username — may differ from `username` when multiple
    /// accounts are provisioned but none is active.
    #[serde(rename = "DefaultUsername", default)]
    pub default_username: String,

    /// Whether the local service considers the user logged in.
    #[zeroize(skip)]
    #[serde(rename = "LoggedIn", default)]
    pub logged_in: bool,

    /// Device name (e.g. `"Debian desktop Linux Device"`).
    #[serde(default)]
    pub device_name: String,

    /// Device type (`desktop`, `mobile`, `paper`).
    #[serde(default)]
    pub device_type: String,
}
