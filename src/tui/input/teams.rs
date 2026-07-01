//! Input for the Teams screen.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::{chat, teams};

pub fn handle(app: &mut App, key: KeyEvent) {
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Esc => teams::close_teams(app),
        KeyCode::Up | KeyCode::Char('k') => teams::move_up(app),
        KeyCode::Down | KeyCode::Char('j') => teams::move_down(app),
        // Enter / → / l: browse the selected team's channels.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(tm) = app.teams.get(app.teams_selected) {
                let team = tm.name.clone();
                chat::open_channel_browser_for_team(app, team);
            }
        }
        KeyCode::F(5) => teams::request_load_teams(app),
        KeyCode::Char('r') | KeyCode::Char('R') if alt => teams::request_load_teams(app),
        KeyCode::Char('i') | KeyCode::Char('I') if alt => teams::close_teams(app),
        _ => {}
    }
}
