//! Input for the Teams screen.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::{chat, teams};
use crate::tui::input::common;

pub fn handle(app: &mut App, key: KeyEvent) {
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    // Universal list movement (↑↓/j k, PgUp/PgDn, g/G, Home/End).
    let (len, sel) = (app.teams.len(), app.teams_selected);
    if common::list_nav(&key, len, sel, |i| app.teams_selected = i) {
        return;
    }
    match key.code {
        KeyCode::Esc => teams::close_teams(app),
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
