//! Authentication / identity flows.
//!
//! Keybase does not expose a username/password login on the CLI — the
//! local `keybased` service handles provisioning and stores
//! credentials. The TUI therefore has only two operations, each split
//! into a `request_*` (fires the worker call) and a
//! `handle_*_response` (applies the result):
//!
//! 1. `request_status` — `keybase status --json`. Fired once at boot
//!    from `tui::run` and from the Login screen's retry (R/F5). On
//!    success it keeps the splash up as a **loading screen** until the
//!    inbox is fetched, then enters the inbox with everything already
//!    loaded; logged out → Login.
//! 2. `request_logout` — runs `keybase logout`, resets state on
//!    success.

use crate::domain::IdentityInfo;
use crate::ports::KeybaseError;
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};

/// Queues a `keybase status --json` check. Used both at boot and for the
/// Login screen's retry — the response handler routes from the splash
/// (logged in → load chats then enter the inbox; logged out → Login).
pub fn request_status(app: &mut App) {
    if !app.begin(InFlight::Status) {
        return;
    }
    app.set_action(ActionState::Running("Checking session…".into()));
    // Best-effort: a dead worker is reported once the response never
    // arrives (the run loop would surface the timeout via the
    // feedback strip — but in practice the worker is alive for the
    // entire process lifetime).
    let _ = app.worker_tx.send(WorkerRequest::Status);
}

/// Queues `keybase logout`.
pub fn request_logout(app: &mut App) {
    if !app.begin(InFlight::Logout) {
        return;
    }
    app.set_action(ActionState::Running("Logging out…".into()));
    let _ = app.worker_tx.send(WorkerRequest::Logout);
}

/// Applies the result of a Status request.
///
/// When logged in, the splash stays up as a **loading screen** — the
/// legend flips to "Loading chats…" while the inbox is fetched, and
/// `chat::handle_load_inbox_response` performs the Splash → Inbox
/// transition once the conversations are in hand. So the inbox is only
/// ever entered fully loaded, never mid-skeleton.
pub fn handle_status_response(app: &mut App, result: Result<IdentityInfo, KeybaseError>) {
    match result {
        Ok(info) => {
            let logged_in = info.logged_in;
            app.push_cmd(
                "keybase status --json",
                true,
                if logged_in {
                    format!("logged in as {}", info.username)
                } else {
                    "not logged in".to_string()
                },
            );
            app.identity = info;
            if logged_in {
                // Stay on the splash (it doubles as the loading screen) and
                // load the inbox; the inbox-load handler enters Inbox once the
                // conversations have arrived.
                app.screen = Screen::Splash;
                // Warm the reaction-picker emoji cache in the background so the
                // first react is instant.
                chat::request_emojis(app);
                chat::request_boot_load_inbox(app);
                // request_boot_load_inbox set the action strip already.
            } else {
                app.screen = Screen::Login;
                app.set_action(ActionState::Idle);
            }
        }
        Err(e) => {
            app.push_cmd("keybase status --json", false, e.to_string());
            app.set_action(ActionState::Error(e.to_string()));
            app.screen = Screen::Login;
        }
    }
}

/// Applies the result of a Logout request.
pub fn handle_logout_response(app: &mut App, result: Result<(), KeybaseError>) {
    match result {
        Ok(()) => {
            app.identity = IdentityInfo::default();
            app.conversations.clear();
            app.conversations_lowered.clear();
            app.filtered_cache.clear();
            app.teams.clear();
            app.screen = Screen::Login;
            app.set_action(ActionState::Done("Logged out".into()));
            app.push_cmd("keybase logout", true, "ok");
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase logout", false, e.to_string());
        }
    }
}
