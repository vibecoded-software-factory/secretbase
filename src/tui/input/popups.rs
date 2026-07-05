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

// ── Channel browser (`c` on a team) ──────────────────────────────────

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
    // Inline delete confirm (destructive): the same navigable y/n mechanics
    // as every confirm overlay (←/→/Tab move, Enter activates, default =
    // cancel), via the shared driver.
    if app.channel_confirm_delete.is_some() {
        common::run_confirm(
            app,
            key,
            |a| &mut a.channel_delete_yes,
            chat::confirm_channel_delete,
            chat::cancel_channel_delete,
        );
        return;
    }
    // `/` filter input owns typing while active (tree-search contract).
    if app.channel_filtering {
        match common::search_key(&mut app.channel_filter, key) {
            common::SearchAction::ClearAndExit | common::SearchAction::Exit => {
                app.channel_filtering = false;
            }
            common::SearchAction::Rebuild => app.channel_selected = 0,
            common::SearchAction::ToList(k) => {
                let len = app.channels_filtered().len();
                common::list_nav(&k, len, app.channel_selected, |i| app.channel_selected = i);
            }
            common::SearchAction::Idle => {}
        }
        return;
    }
    // Universal movement through the shared router — hand-rolled arms had
    // drifted (no Ctrl+D/U half-page, a private page step of 10).
    if common::list_nav(
        &key,
        app.channels_filtered().len(),
        app.channel_selected,
        |i| app.channel_selected = i,
    ) {
        return;
    }
    match key.code {
        KeyCode::Char('/') => app.channel_filtering = true,
        // Esc clears an applied filter before closing the browser.
        KeyCode::Esc if !app.channel_filter.is_empty() => {
            app.channel_filter.clear();
            app.channel_selected = 0;
        }
        KeyCode::Esc => chat::close_channel_browser(app),
        // Enter / → / l: open a channel you're in, join one you're not.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => chat::channel_browser_activate(app),
        // ── common actions (bare) ─────────────────────────────────────────
        KeyCode::Char('n') => chat::open_channel_create(app), // create (enters input)
        KeyCode::Char('r') => chat::open_channel_rename(app), // rename (enters input)
        KeyCode::Char('t') => chat::toggle_default_channel(app), // team default (auto-join)
        KeyCode::Char('m') => chat::open_members_from_browser(app), // members
        // ── destructive / loud (Shift) ────────────────────────────────────
        KeyCode::Char('L') => chat::request_leave_selected_channel(app), // leave (Shift+L)
        KeyCode::Char('x') | KeyCode::Char('X') => chat::open_channel_delete_confirm(app), // delete (confirm-gated)
        KeyCode::F(5) => chat::request_load_channels(app),
        _ => {}
    }
}

// ── Members view (listmembers / add / remove) ──────────────────────────

pub fn members(app: &mut App, key: KeyEvent) {
    // Add mode: the username input owns the keys.
    if app.members.adding {
        match key.code {
            KeyCode::Esc => chat::cancel_member_add(app),
            KeyCode::Enter => chat::request_add_members(app),
            _ => {
                common::route_line_editor(&mut app.members.add_input, key);
            }
        }
        return;
    }
    // Inline remove confirm — same navigable mechanics as every confirm.
    if app.members.confirm_remove.is_some() {
        common::run_confirm(
            app,
            key,
            |a| &mut a.members.remove_yes,
            chat::confirm_remove_member,
            chat::cancel_member_remove,
        );
        return;
    }
    // `/` filter input owns typing while active (tree-search contract).
    if app.members.filtering {
        match common::search_key(&mut app.members.filter, key) {
            common::SearchAction::ClearAndExit | common::SearchAction::Exit => {
                app.members.filtering = false;
            }
            common::SearchAction::Rebuild => app.members.selected = 0,
            common::SearchAction::ToList(k) => {
                let len = app.members.filtered().len();
                common::list_nav(&k, len, app.members.selected, |i| app.members.selected = i);
            }
            common::SearchAction::Idle => {}
        }
        return;
    }
    // Universal movement through the shared router — same drift fix as the
    // channel browser (Ctrl+D/U, unified page step).
    if common::list_nav(
        &key,
        app.members.filtered().len(),
        app.members.selected,
        |i| app.members.selected = i,
    ) {
        return;
    }
    match key.code {
        KeyCode::Char('/') => app.members.filtering = true,
        KeyCode::Esc if !app.members.filter.is_empty() => {
            app.members.filter.clear();
            app.members.selected = 0;
        }
        KeyCode::Esc => chat::close_members(app),
        // Add member(s) — bare (safe/common).
        KeyCode::Char('a') => chat::open_member_add(app),
        // Remove the selected member — bare `x`, gated by the inline confirm.
        KeyCode::Char('x') | KeyCode::Char('X') => chat::open_member_remove_confirm(app),
        KeyCode::F(5) => chat::request_load_members(app),
        _ => {}
    }
}

// ── Global search popup ───────────────────────────────────────────────

pub fn search_global(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.search_global_results.len() {
            app.search_global_selected = i;
            chat::open_selected_search_result(app);
        }
        return;
    }
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
        _ if common::list_nav_arrows(
            &key,
            app.search_global_results.len(),
            app.search_global_selected,
            |i| app.search_global_selected = i,
        ) => {}
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
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.emoji.filtered().len() {
            app.react_selected = i;
            chat::request_send_reaction(app);
        }
        return;
    }
    let len = app.emoji.filtered().len();
    if common::list_nav_arrows(&key, len, app.react_selected, |i| app.react_selected = i) {
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_react(app),
        KeyCode::Enter => chat::request_send_reaction(app),
        _ => {
            // Typing edits the search query; reset the highlight to the top
            // match (and refilter the catalogue) whenever the query changes.
            let before = app.react.text().to_string();
            common::route_line_editor(&mut app.react, key);
            if app.react.text() != before {
                app.react_selected = 0;
                app.rebuild_emoji_filter();
            }
        }
    }
}

// ── In-conversation search modal (Ctrl+F → searchregexp) ──────────────

/// GIF-search popup: type a query → Enter searches; with results, ↑↓ pick
/// and Enter sends the selected GIF; editing the query invalidates results
/// (next Enter re-searches) — the same contract as the conversation search.
pub fn giphy_search(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.giphy_results.len() {
            app.giphy_selected = i;
            chat::giphy_send_selected(app);
        }
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_giphy_search(app),
        KeyCode::Enter => {
            if app.giphy_results.is_empty() {
                chat::request_giphy_search(app);
            } else {
                chat::giphy_send_selected(app);
            }
        }
        KeyCode::F(5) => chat::request_giphy_search(app),
        _ if common::list_nav_arrows(&key, app.giphy_results.len(), app.giphy_selected, |i| {
            app.giphy_selected = i
        }) => {}
        _ => {
            let before = app.giphy_input.text().to_string();
            common::route_line_editor(&mut app.giphy_input, key);
            if app.giphy_input.text() != before {
                app.giphy_results.clear();
                app.giphy_selected = 0;
            }
        }
    }
}

pub fn conv_search(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.conv_search_results.len() {
            app.conv_search_selected = i;
            chat::conv_search_jump_selected(app);
        }
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_conv_search(app),
        KeyCode::Enter => {
            if app.conv_search_results.is_empty() {
                chat::request_conv_search(app);
            } else {
                chat::conv_search_jump_selected(app);
            }
        }
        KeyCode::F(5) => chat::request_conv_search(app),
        _ if common::list_nav_arrows(
            &key,
            app.conv_search_results.len(),
            app.conv_search_selected,
            |i| app.conv_search_selected = i,
        ) => {}
        _ => {
            let before = app.conv_search.text().to_string();
            common::route_line_editor(&mut app.conv_search, key);
            // Editing the query invalidates the old results so the next Enter
            // re-runs the search instead of jumping to a stale hit.
            if app.conv_search.text() != before {
                app.conv_search_results.clear();
                app.conv_search_selected = 0;
            }
        }
    }
}

// ── Command palette (Ctrl+P) ──────────────────────────────────────────

pub fn command_palette(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < crate::tui::flows::palette::filtered_commands(app).len() {
            app.palette_selected = i;
            crate::tui::flows::palette::palette_run_selected(app);
        }
        return;
    }
    use crate::tui::flows::palette;
    let len = palette::filtered_commands(app).len();
    if common::list_nav_arrows(&key, len, app.palette_selected, |i| {
        app.palette_selected = i
    }) {
        return;
    }
    match key.code {
        KeyCode::Esc => palette::close_command_palette(app),
        KeyCode::Enter => palette::palette_run_selected(app),
        _ => {
            let before = app.palette.text().to_string();
            common::route_line_editor(&mut app.palette, key);
            // A changed query re-filters — snap the selection back to the top.
            if app.palette.text() != before {
                app.palette_selected = 0;
            }
        }
    }
}

// ── Quick switcher (Ctrl+K) ───────────────────────────────────────────

pub fn quick_switcher(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.switcher_selectable().len() {
            app.switcher_selected = i;
            chat::quick_switcher_open_selected(app);
        }
        return;
    }
    let len = app.switcher_selectable().len();
    if common::list_nav_arrows(&key, len, app.switcher_selected, |i| {
        app.switcher_selected = i
    }) {
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_quick_switcher(app),
        KeyCode::Enter => chat::quick_switcher_open_selected(app),
        _ => {
            let before = app.switcher.text().to_string();
            common::route_line_editor(&mut app.switcher, key);
            if app.switcher.text() != before {
                app.switcher_selected = 0;
            }
        }
    }
}
