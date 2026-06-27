//! Authentication / identity flows.
//!
//! Keybase does not expose a username/password login on the CLI — the
//! local `keybased` service handles provisioning and stores
//! credentials. The TUI therefore has only three operations, each
//! split into a `request_*` (fires the worker call) and a
//! `handle_*_response` (applies the result):
//!
//! 1. `request_boot_status` — fired once at boot from `tui::run`.
//!    Routes Splash → Inbox/Login on response, chaining a
//!    LoadInbox when logged in.
//! 2. `request_status_check` — same call, fired interactively (e.g.
//!    F5 on the login screen). Just refreshes identity.
//! 3. `request_logout` — runs `keybase logout`, resets state on
//!    success.

use crate::domain::IdentityInfo;
use crate::ports::KeybaseError;
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};

/// Initial post-construction call: queues a `keybase status --json`
/// against the worker and tags it as the bootstrap variant so the
/// response handler will chain a LoadInbox + screen transition.
pub fn request_boot_status(app: &mut App) {
    if !app.begin(InFlight::BootStatus) {
        return;
    }
    app.set_action(ActionState::Running("Checking session…".into()));
    // Best-effort: a dead worker is reported once the response never
    // arrives (the run loop would surface the timeout via the
    // feedback strip — but in practice the worker is alive for the
    // entire process lifetime).
    let _ = app.worker_tx.send(WorkerRequest::Status);
}

/// Interactive re-check (F5 on the login screen).
pub fn request_status_check(app: &mut App) {
    if !app.begin(InFlight::CheckStatus) {
        return;
    }
    app.set_action(ActionState::Running("Checking session…".into()));
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

/// Applies the result of a Status request. `is_boot` distinguishes
/// the boot variant (chains inbox + screen transition) from the
/// interactive variant (just refresh).
pub fn handle_status_response(
    app: &mut App,
    result: Result<IdentityInfo, KeybaseError>,
    is_boot: bool,
) {
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
            if is_boot {
                if logged_in {
                    app.screen = Screen::Inbox;
                    // Warm the reaction-picker emoji cache in the background
                    // so the first react is instant.
                    chat::request_emojis(app);
                    // Chain the initial inbox load through the worker.
                    chat::request_load_inbox(app);
                    // request_load_inbox set the action strip already.
                    return;
                } else {
                    app.screen = Screen::Login;
                    app.set_action(ActionState::Idle);
                    return;
                }
            }
            // Interactive (non-boot) path: if the user logged in
            // elsewhere and pressed R on the Login screen, move into the
            // inbox instead of leaving Login as a dead-end (mirrors the
            // boot branch).
            if logged_in && app.screen == Screen::Login {
                app.screen = Screen::Inbox;
                chat::request_load_inbox(app);
                return;
            }
            app.set_action(ActionState::Done("Session refreshed".into()));
        }
        Err(e) => {
            app.push_cmd("keybase status --json", false, e.to_string());
            app.set_action(ActionState::Error(e.to_string()));
            if is_boot {
                app.screen = Screen::Login;
            }
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
