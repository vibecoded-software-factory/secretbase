//! Input handling for the Login screen (signed-out).
//!
//! A form: Username / Device / Paper key fields plus two
//! action buttons ("Log in" = paper-key login, "Log in in terminal" =
//! interactive native login). Because the fields are **text inputs** they own
//! bare letters as typed text, so the screen's actions live on non-text keys
//! (the gradient convention): `Tab`/`Shift+Tab` (or `↑`/`↓`) cycle focus,
//! `Enter` submits the focused action, `F2` reveals the paper key, `F5` retries
//! the status check, `Esc`/`Ctrl+C` quit.

use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::app::{App, LoginField};
use crate::tui::flows;
use crate::tui::input::common;

pub fn handle(app: &mut App, key: KeyEvent) {
    let field = app.login_focus;
    match key.code {
        KeyCode::Esc => {
            app.should_quit = true;
            return;
        }
        KeyCode::F(5) => {
            flows::auth::request_status(app);
            return;
        }
        KeyCode::F(2) => {
            app.login_reveal = !app.login_reveal;
            return;
        }
        KeyCode::Tab | KeyCode::Down => {
            cycle_focus(app, true);
            return;
        }
        KeyCode::BackTab | KeyCode::Up => {
            cycle_focus(app, false);
            return;
        }
        // Enter submits the focused action; on a field it runs the primary
        // (paper-key) login, so a filled-in form logs in without tabbing.
        KeyCode::Enter => {
            submit(app, field);
            return;
        }
        // Space activates a button; inside a field it's a literal space (paper
        // keys are space-separated), so it falls through to the editor.
        KeyCode::Char(' ')
            if matches!(field, LoginField::SubmitPaperkey | LoginField::SubmitNative) =>
        {
            submit(app, field);
            return;
        }
        _ => {}
    }

    // Remaining keys type into the focused field (buttons ignore them).
    let editor = match field {
        LoginField::Username => &mut app.login_username,
        LoginField::Device => &mut app.login_device,
        LoginField::PaperKey => &mut app.login_paperkey,
        LoginField::SubmitPaperkey | LoginField::SubmitNative => return,
    };
    common::route_line_editor(editor, key);
}

/// Runs the action for the focused element: the native (interactive) login
/// for its button, the paper-key login for everything else.
fn submit(app: &mut App, field: LoginField) {
    match field {
        LoginField::SubmitNative => flows::auth::request_native_login(app),
        _ => flows::auth::request_login_paperkey(app),
    }
}

/// Moves focus to the next/previous element in [`LoginField::ORDER`], wrapping.
fn cycle_focus(app: &mut App, forward: bool) {
    let order = LoginField::ORDER;
    let pos = order
        .iter()
        .position(|f| *f == app.login_focus)
        .unwrap_or(0);
    let len = order.len();
    let next = if forward {
        (pos + 1) % len
    } else {
        (pos + len - 1) % len
    };
    app.login_focus = order[next];
}
