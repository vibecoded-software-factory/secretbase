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
            app.search_global_selected = app.search_global_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            let max = app.search_global_results.len().saturating_sub(1);
            app.search_global_selected = (app.search_global_selected + 1).min(max);
        }
        _ => {
            common::route_line_editor(&mut app.search_global_input, key);
        }
    }
}

// ── Confirm delete message popup ──────────────────────────────────────

pub fn confirm_delete_message(app: &mut App, key: KeyEvent) {
    let commit = |app: &mut App| {
        app.screen = Screen::Conversation;
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
        _ => {
            common::route_line_editor(&mut app.react, key);
        }
    }
}
