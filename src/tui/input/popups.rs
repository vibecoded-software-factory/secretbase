//! Input handlers for the popup overlays: new-conversation, global
//! search, confirm-delete-message, reaction input, attachment download.

use crossterm::event::{KeyCode, KeyEvent};

use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::input::common::{self, ConfirmInput};
use crate::tui::screens::Screen;

// ── New conversation popup ────────────────────────────────────────────

pub fn new_conversation(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => chat::close_new_conversation(app),
        KeyCode::Enter => chat::request_create_new_conversation(app),
        _ => {
            common::route_line_editor(&mut app.new_conv, key);
        }
    }
}

// ── Global search popup ───────────────────────────────────────────────

pub fn search_global(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => chat::close_search_global(app),
        KeyCode::Enter => {
            if !app.search_global_results.is_empty() {
                chat::open_selected_search_result(app);
            } else {
                chat::request_search_inbox_remote(app);
            }
        }
        KeyCode::F(5) => chat::request_search_inbox_remote(app),
        KeyCode::Up => {
            let len = app.search_global_results.len();
            app.search_global_selected = common::clamp_move(app.search_global_selected, -1, len);
        }
        KeyCode::Down => {
            let len = app.search_global_results.len();
            app.search_global_selected = common::clamp_move(app.search_global_selected, 1, len);
        }
        _ => {
            common::route_line_editor(&mut app.search_global_input, key);
        }
    }
}

// ── Confirm delete message popup ──────────────────────────────────────

pub fn confirm_delete_message(app: &mut App, key: KeyEvent) {
    let commit = |app: &mut App| {
        app.screen = Screen::Inbox;
        app.focus = crate::tui::screens::Focus::Chat;
        chat::request_delete_selected_message(app);
    };
    match common::confirm_key(key) {
        ConfirmInput::Commit => commit(app),
        ConfirmInput::Cancel => chat::close_delete_confirm(app),
        ConfirmInput::Activate => {
            if app.delete_msg_yes {
                commit(app);
            } else {
                chat::close_delete_confirm(app);
            }
        }
        ConfirmInput::Yes => app.delete_msg_yes = true,
        ConfirmInput::No => app.delete_msg_yes = false,
        ConfirmInput::Toggle => app.delete_msg_yes = !app.delete_msg_yes,
        ConfirmInput::Ignore => {}
    }
}

// ── Reaction input popup ──────────────────────────────────────────────

pub fn react(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => chat::close_react(app),
        KeyCode::Enter => chat::request_send_reaction(app),
        KeyCode::Up => react_move(app, -1),
        KeyCode::Down => react_move(app, 1),
        _ => {
            // Typing edits the search query; reset the highlight to the top
            // match whenever the query actually changes.
            let before = app.react.text().to_string();
            common::route_line_editor(&mut app.react, key);
            if app.react.text() != before {
                app.react_selected = 0;
            }
        }
    }
}

fn react_move(app: &mut App, delta: isize) {
    let len = app.filtered_emoji_indices().len();
    app.react_selected = common::clamp_move(app.react_selected, delta, len);
}

// ── Quick switcher (Ctrl+K) ───────────────────────────────────────────

pub fn quick_switcher(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => chat::close_quick_switcher(app),
        KeyCode::Enter => chat::quick_switcher_open_selected(app),
        KeyCode::Up => switcher_move(app, -1),
        KeyCode::Down => switcher_move(app, 1),
        _ => {
            let before = app.switcher.text().to_string();
            common::route_line_editor(&mut app.switcher, key);
            if app.switcher.text() != before {
                app.switcher_selected = 0;
            }
        }
    }
}

fn switcher_move(app: &mut App, delta: isize) {
    let len = app.switcher_selectable().len();
    app.switcher_selected = common::clamp_move(app.switcher_selected, delta, len);
}
