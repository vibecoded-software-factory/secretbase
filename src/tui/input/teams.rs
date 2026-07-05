//! Input for the Teams screen.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::{chat, teams};
use crate::tui::input::common;

pub fn handle(app: &mut App, key: KeyEvent) {
    // `/` filter input owns typing while active (the tree-search contract:
    // Esc clears and exits, Enter keeps the query and returns to the list).
    if app.teams.filtering {
        match common::search_key(&mut app.teams.filter, key) {
            common::SearchAction::ClearAndExit | common::SearchAction::Exit => {
                app.teams.filtering = false;
            }
            common::SearchAction::Rebuild => app.teams.selected = 0,
            common::SearchAction::ToList(k) => {
                let len = app.teams.filtered().len();
                common::list_nav(&k, len, app.teams.selected, |i| app.teams.selected = i);
            }
            common::SearchAction::Idle => {}
        }
        return;
    }
    // Universal list movement (↑↓/j k, PgUp/PgDn, g/G, Home/End) over the
    // filtered projection.
    let (len, sel) = (app.teams.filtered().len(), app.teams.selected);
    if common::list_nav(&key, len, sel, |i| app.teams.selected = i) {
        return;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('/') => app.teams.filtering = true,
        // Esc clears an applied filter first, then leaves the screen.
        KeyCode::Esc if !app.teams.filter.is_empty() => {
            app.teams.filter.clear();
            app.teams.selected = 0;
        }
        KeyCode::Esc => teams::close_teams(app),
        // `:` = the command line (the palette), vim-style, from any
        // non-typing surface; Ctrl+G global search works here too.
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        KeyCode::Char('g') | KeyCode::Char('G') if ctrl => {
            chat::open_search_global(app);
        }
        // Enter / → / l: browse the selected team's channels.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            let filtered = app.teams.filtered();
            if let Some(tm) = filtered
                .get(app.teams.selected)
                .and_then(|&i| app.teams.list.get(i))
            {
                let team = tm.name.clone();
                chat::open_channel_browser_for_team(app, team);
            }
        }
        // Refresh — bare `r` (gradient), or F5.
        KeyCode::Char('r') | KeyCode::F(5) => teams::request_load_teams(app),
        _ => {}
    }
}
