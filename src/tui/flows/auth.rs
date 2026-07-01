//! Authentication / identity flows.
//!
//! Keybase's CLI has no simple username/password login — the local
//! `keybased` service owns device provisioning. Two login paths exist,
//! and the TUI offers both from the Login screen:
//!
//! * **paper key** (`request_login_paperkey`) — the only *non-interactive*
//!   login the CLI supports: `keybase login --devicename <d> <user>` with
//!   the paper key on stdin. Works on a device never provisioned for the
//!   account. Split into a `request_*` / `handle_*_response` pair like every
//!   worker call.
//! * **native / interactive** (`request_native_login`) — for an
//!   already-provisioned device (e.g. after a logout), keybase asks for the
//!   passphrase via pinentry/terminal, which can't be scripted. The TUI
//!   therefore cedes the terminal to interactive `keybase login` (handled in
//!   the run loop via `App::pending_native_login`) and re-checks status after.
//!
//! Plus the two session operations, each a `request_*` / `handle_*_response`:
//!
//! 1. `request_status` — `keybase status --json`. Fired once at boot
//!    from `tui::run` and from the Login screen's retry (R/F5). On
//!    success it keeps the splash up as a **loading screen** until the
//!    inbox is fetched, then enters the inbox with everything already
//!    loaded; logged out → Login.
//! 2. `request_logout` — runs `keybase logout`, resets state on
//!    success.

use crate::domain::{IdentityInfo, LineEditor};
use crate::ports::KeybaseError;
use crate::tui::action::ActionState;
use crate::tui::app::{App, LoginField};
use crate::tui::flows::chat;
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerRequest};
use zeroize::Zeroizing;

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

/// Pre-fills the Login form when the app lands on the signed-out screen:
/// the username from the last-known default, the device name with a
/// sensible default. Only fills empty fields so a retry keeps what the
/// user typed.
pub fn prepare_login_screen(app: &mut App) {
    if app.login_username.is_empty() && !app.identity.default_username.is_empty() {
        app.login_username = LineEditor::from_text(&app.identity.default_username);
    }
    if app.login_device.is_empty() {
        app.login_device = LineEditor::from_text("secretbase");
    }
    app.login_focus = LoginField::Username;
}

/// Fires the non-interactive **paper-key** login from the Login form.
/// Validates the three fields, then hands the paper key to the worker in a
/// zeroizing buffer. Only usable on a device not yet provisioned for the
/// account — a failure (e.g. "already provisioned") surfaces in the strip,
/// pointing the user at the native login.
pub fn request_login_paperkey(app: &mut App) {
    let username = app.login_username.text().trim().to_string();
    let device = app.login_device.text().trim().to_string();
    if username.is_empty() {
        app.set_action(ActionState::Error("Username is required".into()));
        app.login_focus = LoginField::Username;
        return;
    }
    if device.is_empty() {
        app.set_action(ActionState::Error("Device name is required".into()));
        app.login_focus = LoginField::Device;
        return;
    }
    if app.login_paperkey.text().trim().is_empty() {
        app.set_action(ActionState::Error("Paper key is required".into()));
        app.login_focus = LoginField::PaperKey;
        return;
    }
    let paperkey = Zeroizing::new(app.login_paperkey.text().trim().to_string());
    if !app.begin(InFlight::LoginPaperkey) {
        return;
    }
    app.set_action(ActionState::Running("Logging in…".into()));
    let _ = app.worker_tx.send(WorkerRequest::LoginPaperkey {
        username,
        device,
        paperkey,
    });
}

/// Asks the run loop to cede the terminal to interactive `keybase login`
/// (the passphrase path for an already-provisioned device). The username, if
/// present, is passed as the `[username]` argument. The run loop suspends the
/// TUI, runs the command, restores, and re-checks status.
pub fn request_native_login(app: &mut App) {
    let username = app.login_username.text().trim().to_string();
    app.pending_native_login = Some(username);
}

/// Applies the result of a paper-key login. On success the paper key is
/// wiped and a fresh `status` check loads the inbox and enters it; on
/// failure the CLI's message is surfaced (it usually explains what to do,
/// e.g. provision with another device or use the native login).
pub fn handle_login_paperkey_response(app: &mut App, result: Result<(), KeybaseError>) {
    match result {
        Ok(()) => {
            app.login_paperkey = LineEditor::default(); // wipe the secret
            app.push_cmd("keybase login", true, "logged in");
            // Re-check status: logged in → loads chats and enters the inbox.
            request_status(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase login", false, e.to_string());
        }
    }
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
                prepare_login_screen(app);
                app.set_action(ActionState::Idle);
            }
        }
        Err(e) => {
            app.push_cmd("keybase status --json", false, e.to_string());
            app.set_action(ActionState::Error(e.to_string()));
            app.screen = Screen::Login;
            prepare_login_screen(app);
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
            prepare_login_screen(app);
            app.set_action(ActionState::Done("Logged out".into()));
            app.push_cmd("keybase logout", true, "ok");
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase logout", false, e.to_string());
        }
    }
}
