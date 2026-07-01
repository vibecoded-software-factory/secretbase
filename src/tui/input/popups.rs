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

// ── Unhide popup (restore a blocked/reported conv by name) ─────────────

pub fn unhide_conversation(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => chat::close_unhide(app),
        KeyCode::Enter => chat::request_unhide_conversation(app),
        _ => {
            common::route_line_editor(&mut app.unhide_input, key);
        }
    }
}

// ── Channel browser (Alt+K on a team) ──────────────────────────────────

pub fn channel_browser(app: &mut App, key: KeyEvent) {
    // Create mode: the new-channel-name input owns the keys.
    if app.channel_creating {
        match key.code {
            KeyCode::Esc => chat::cancel_channel_create(app),
            KeyCode::Enter => chat::request_create_channel(app),
            _ => {
                common::route_line_editor(&mut app.channel_new_name, key);
            }
        }
        return;
    }
    // Rename mode: the new-name input owns the keys.
    if app.channel_renaming.is_some() {
        match key.code {
            KeyCode::Esc => chat::cancel_channel_rename(app),
            KeyCode::Enter => chat::request_rename_channel(app),
            _ => {
                common::route_line_editor(&mut app.channel_new_name, key);
            }
        }
        return;
    }
    // Inline delete confirm (destructive): y confirms, n / Esc cancels.
    if app.channel_confirm_delete.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => chat::confirm_channel_delete(app),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                chat::cancel_channel_delete(app)
            }
            _ => {}
        }
        return;
    }
    let alt = key.modifiers.contains(crossterm::event::KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => chat::close_channel_browser(app),
        KeyCode::Up | KeyCode::Char('k') => chat::channel_browser_move(app, -1),
        KeyCode::Down | KeyCode::Char('j') => chat::channel_browser_move(app, 1),
        KeyCode::PageUp => chat::channel_browser_move(app, -10),
        KeyCode::PageDown => chat::channel_browser_move(app, 10),
        KeyCode::Home | KeyCode::Char('g') => chat::channel_browser_move(app, isize::MIN),
        KeyCode::End | KeyCode::Char('G') => chat::channel_browser_move(app, isize::MAX),
        // Create a new channel (enters create mode).
        KeyCode::Char('n') | KeyCode::Char('N') if alt => chat::open_channel_create(app),
        // Enter / →: open a channel you're in, join one you're not.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => chat::channel_browser_activate(app),
        // Leave a joined channel.
        KeyCode::Char('x') | KeyCode::Char('X') => chat::request_leave_selected_channel(app),
        // Rename / delete the selected channel (admin).
        KeyCode::Char('r') | KeyCode::Char('R') => chat::open_channel_rename(app),
        KeyCode::Char('d') | KeyCode::Char('D') => chat::open_channel_delete_confirm(app),
        KeyCode::F(5) => chat::request_load_channels(app),
        _ => {}
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
