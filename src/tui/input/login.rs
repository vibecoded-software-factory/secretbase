//! Input handling for the login-hint screen.
//!
//! Keybase has no in-TUI password login — provisioning is done with
//! `keybase login` from a terminal. The TUI screen therefore only
//! has three keybindings: `r` to retry status, `q`/`Esc` to quit,
//! `F1` to open help (handled globally).

use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::app::App;
use crate::tui::flows;

pub fn handle(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::F(5) => {
            flows::auth::request_status_check(app);
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            app.should_quit = true;
        }
        _ => {}
    }
}
