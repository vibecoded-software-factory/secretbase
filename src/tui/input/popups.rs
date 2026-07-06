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
    if app.channel_browser.creating {
        match key.code {
            KeyCode::Esc => chat::cancel_channel_create(app),
            KeyCode::Enter => chat::request_create_channel(app),
            _ => {
                common::route_line_editor(&mut app.channel_browser.new_name, key);
            }
        }
        return;
    }
    // Rename mode: the new-name input owns the keys.
    if app.channel_browser.renaming.is_some() {
        match key.code {
            KeyCode::Esc => chat::cancel_channel_rename(app),
            KeyCode::Enter => chat::request_rename_channel(app),
            _ => {
                common::route_line_editor(&mut app.channel_browser.new_name, key);
            }
        }
        return;
    }
    // Inline delete confirm (destructive): the same navigable y/n mechanics
    // as every confirm overlay (←/→/Tab move, Enter activates, default =
    // cancel), via the shared driver.
    if app.channel_browser.confirm_delete.is_some() {
        common::run_confirm(
            app,
            key,
            |a| &mut a.channel_browser.delete_yes,
            chat::confirm_channel_delete,
            chat::cancel_channel_delete,
        );
        return;
    }
    // `/` filter input owns typing while active (tree-search contract).
    if app.channel_browser.filtering {
        match common::search_key(&mut app.channel_browser.filter, key) {
            common::SearchAction::ClearAndExit | common::SearchAction::Exit => {
                app.channel_browser.filtering = false;
            }
            common::SearchAction::Rebuild => app.channel_browser.selected = 0,
            common::SearchAction::ToList(k) => {
                let len = app.channel_browser.filtered().len();
                common::list_nav(&k, len, app.channel_browser.selected, |i| {
                    app.channel_browser.selected = i
                });
            }
            common::SearchAction::Idle => {}
        }
        return;
    }
    // Universal movement through the shared router — hand-rolled arms had
    // drifted (no Ctrl+D/U half-page, a private page step of 10).
    if common::list_nav(
        &key,
        app.channel_browser.filtered().len(),
        app.channel_browser.selected,
        |i| app.channel_browser.selected = i,
    ) {
        return;
    }
    match key.code {
        KeyCode::Char('/') => app.channel_browser.filtering = true,
        // Esc clears an applied filter before closing the browser.
        KeyCode::Esc if !app.channel_browser.filter.is_empty() => {
            app.channel_browser.filter.clear();
            app.channel_browser.selected = 0;
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
        if i < app.global_search.results.len() {
            app.global_search.selected = i;
            chat::open_selected_search_result(app);
        }
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_search_global(app),
        KeyCode::Enter => {
            if !app.global_search.results.is_empty() {
                chat::open_selected_search_result(app);
            } else {
                chat::request_search_inbox_remote(app);
            }
        }
        KeyCode::F(5) => chat::request_search_inbox_remote(app),
        _ if common::list_nav_arrows(
            &key,
            app.global_search.results.len(),
            app.global_search.selected,
            |i| app.global_search.selected = i,
        ) => {}
        _ => {
            common::route_line_editor(&mut app.global_search.query, key);
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
        if i < app.giphy.results.len() {
            app.giphy.selected = i;
            chat::giphy_send_selected(app);
        }
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_giphy_search(app),
        KeyCode::Enter => {
            if app.giphy.results.is_empty() {
                chat::request_giphy_search(app);
            } else {
                chat::giphy_send_selected(app);
            }
        }
        KeyCode::F(5) => chat::request_giphy_search(app),
        _ if common::list_nav_arrows(&key, app.giphy.results.len(), app.giphy.selected, |i| {
            app.giphy.selected = i
        }) => {}
        _ => {
            let before = app.giphy.query.text().to_string();
            common::route_line_editor(&mut app.giphy.query, key);
            if app.giphy.query.text() != before {
                app.giphy.results.clear();
                app.giphy.selected = 0;
            }
        }
    }
}

pub fn conv_search(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.conv_search.results.len() {
            app.conv_search.selected = i;
            chat::conv_search_jump_selected(app);
        }
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_conv_search(app),
        KeyCode::Enter => {
            if app.conv_search.results.is_empty() {
                chat::request_conv_search(app);
            } else {
                chat::conv_search_jump_selected(app);
            }
        }
        KeyCode::F(5) => chat::request_conv_search(app),
        _ if common::list_nav_arrows(
            &key,
            app.conv_search.results.len(),
            app.conv_search.selected,
            |i| app.conv_search.selected = i,
        ) => {}
        _ => {
            let before = app.conv_search.query.text().to_string();
            common::route_line_editor(&mut app.conv_search.query, key);
            // Editing the query invalidates the old results so the next Enter
            // re-runs the search instead of jumping to a stale hit.
            if app.conv_search.query.text() != before {
                app.conv_search.results.clear();
                app.conv_search.selected = 0;
            }
        }
    }
}

// ── Command palette (Ctrl+P) ──────────────────────────────────────────

pub fn command_palette(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < crate::tui::flows::palette::filtered_commands(app).len() {
            app.palette.selected = i;
            crate::tui::flows::palette::palette_run_selected(app);
        }
        return;
    }
    use crate::tui::flows::palette;
    let len = palette::filtered_commands(app).len();
    if common::list_nav_arrows(&key, len, app.palette.selected, |i| {
        app.palette.selected = i
    }) {
        return;
    }
    match key.code {
        KeyCode::Esc => palette::close_command_palette(app),
        KeyCode::Enter => palette::palette_run_selected(app),
        _ => {
            let before = app.palette.query.text().to_string();
            common::route_line_editor(&mut app.palette.query, key);
            // A changed query re-filters — snap the selection back to the top.
            if app.palette.query.text() != before {
                app.palette.selected = 0;
            }
        }
    }
}

// ── Quick switcher (Ctrl+K) ───────────────────────────────────────────

pub fn quick_switcher(app: &mut App, key: KeyEvent) {
    // Alt+1..9 — instant pick-and-activate of the Nth row.
    if let Some(i) = common::alt_digit(&key) {
        if i < app.switcher_selectable().len() {
            app.switcher.selected = i;
            chat::quick_switcher_open_selected(app);
        }
        return;
    }
    let len = app.switcher_selectable().len();
    if common::list_nav_arrows(&key, len, app.switcher.selected, |i| {
        app.switcher.selected = i
    }) {
        return;
    }
    match key.code {
        KeyCode::Esc => chat::close_quick_switcher(app),
        KeyCode::Enter => chat::quick_switcher_open_selected(app),
        _ => {
            let before = app.switcher.query.text().to_string();
            common::route_line_editor(&mut app.switcher.query, key);
            if app.switcher.query.text() != before {
                app.switcher.selected = 0;
            }
        }
    }
}

// ── Per-message action menu (right-click) ─────────────────────────────

/// The per-message action menu (`Screen::MessageActions`): navigate the fixed
/// action list, `Enter`/`l`/`→` runs the highlighted action, `Esc` closes.
pub fn message_actions(app: &mut App, key: KeyEvent) {
    let len = crate::tui::flows::chat::MESSAGE_ACTIONS.len();
    if common::list_nav(&key, len, app.msg_actions_selected, |i| {
        app.msg_actions_selected = i
    }) {
        return;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => chat::run_message_action(app),
        KeyCode::Esc => chat::close_message_actions(app),
        _ => {}
    }
}
