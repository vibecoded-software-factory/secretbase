//! Input for the Teams screen.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::{chat, teams};
use crate::tui::input::common;

pub fn handle(app: &mut App, key: KeyEvent) {
    // Universal list movement (↑↓/j k, PgUp/PgDn, g/G, Home/End).
    let (len, sel) = (app.teams.len(), app.teams_selected);
    if common::list_nav(&key, len, sel, |i| app.teams_selected = i) {
        return;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => teams::close_teams(app),
        // `:` = the command line (the palette), vim-style, from any
        // non-typing surface; Ctrl+G global search works here too.
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        KeyCode::Char('g') | KeyCode::Char('G') if ctrl => {
            chat::open_search_global(app);
        }
        // Enter / → / l: browse the selected team's channels.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(tm) = app.teams.get(app.teams_selected) {
                let team = tm.name.clone();
                chat::open_channel_browser_for_team(app, team);
            }
        }
        // Refresh — bare `r` (gradient), or F5.
        KeyCode::Char('r') | KeyCode::F(5) => teams::request_load_teams(app),
        _ => {}
    }
}
