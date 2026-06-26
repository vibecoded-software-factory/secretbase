//! Input for the Teams screen.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::teams;

pub fn handle(app: &mut App, key: KeyEvent) {
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Esc => teams::close_teams(app),
        KeyCode::Up | KeyCode::Char('k') => teams::move_up(app),
        KeyCode::Down | KeyCode::Char('j') => teams::move_down(app),
        KeyCode::F(5) => teams::request_load_teams(app),
        KeyCode::Char('r') | KeyCode::Char('R') if alt => teams::request_load_teams(app),
        KeyCode::Char('i') | KeyCode::Char('I') if alt => teams::close_teams(app),
        _ => {}
    }
}
