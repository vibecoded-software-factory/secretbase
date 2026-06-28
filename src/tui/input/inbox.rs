//! Input handling for the main inbox screen.
//!
//!   * `Tab` / `Shift+Tab` cycle keyboard focus through the panels.
//!   * Inside a panel, `↑`/`↓` (or `j`/`k`) navigate.
//!   * `Alt+`-prefixed shortcuts trigger actions globally regardless
//!     of focus (copy, mark read, refresh, new convo, …).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::input::common::{self, SearchAction};
use crate::tui::screens::{Focus, Screen};

/// Focus cycle order — Search, the status filter, the conversation tree, the
/// open chat, the command log. `Chat` is skipped while no conversation is open.
const FOCUS_ORDER: [Focus; 5] = [
    Focus::Search,
    Focus::Filters,
    Focus::Tree,
    Focus::Chat,
    Focus::CmdLog,
];

/// Cycles focus, skipping `Chat` when there's no open conversation to focus.
fn cycle(app: &App, forward: bool) -> Focus {
    let mut f = common::cycle_focus(&FOCUS_ORDER, app.focus, forward);
    if f == Focus::Chat && app.open_conv_id.is_none() {
        f = common::cycle_focus(&FOCUS_ORDER, f, forward);
    }
    f
}

pub fn handle(app: &mut App, key: KeyEvent) {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Modifier-gated actions and focus cycling work regardless of focus,
    // INCLUDING while the search box is focused — they can't be confused
    // with text input, and the hint bar promises the action is always
    // one keystroke away.
    match key.code {
        KeyCode::Char('c') | KeyCode::Char('C') if alt => {
            chat::do_copy_conversation_label(app);
            return;
        }
        KeyCode::Char('m') | KeyCode::Char('M') if alt => {
            chat::request_mark_read(app);
            return;
        }
        KeyCode::Char('r') | KeyCode::Char('R') if alt => {
            chat::request_load_inbox(app);
            return;
        }
        KeyCode::Char('n') | KeyCode::Char('N') if alt => {
            chat::open_new_conversation(app);
            return;
        }
        KeyCode::Char('u') | KeyCode::Char('U') if alt => {
            chat::request_mute_conversation(app);
            return;
        }
        KeyCode::Char('o') | KeyCode::Char('O') if alt => {
            chat::request_unmute_conversation(app);
            return;
        }
        KeyCode::Char('g') | KeyCode::Char('G') if ctrl => {
            chat::open_search_global(app);
            return;
        }
        KeyCode::Char('t') | KeyCode::Char('T') if alt => {
            crate::tui::flows::teams::open_teams(app);
            return;
        }
        KeyCode::Char('i') | KeyCode::Char('I') if alt => {
            chat::open_conv_action(app, crate::tui::app::ConvAction::Ignore);
            return;
        }
        KeyCode::Tab => {
            app.focus = cycle(app, true);
            return;
        }
        KeyCode::BackTab => {
            app.focus = cycle(app, false);
            return;
        }
        _ => {}
    }

    // The search box owns the remaining (text) keys while focused.
    if app.focus == Focus::Search {
        return handle_search(app, key);
    }

    match key.code {
        KeyCode::Char('/') => {
            app.focus = Focus::Search;
            return;
        }
        KeyCode::F(5) => {
            chat::request_load_inbox(app);
            return;
        }
        KeyCode::Char('L') => {
            app.logout_yes = false;
            app.screen = Screen::ConfirmLogout;
            return;
        }
        _ => {}
    }

    match app.focus {
        Focus::Tree => handle_tree(app, key),
        Focus::Chat => crate::tui::input::conversation::handle(app, key),
        Focus::Filters => handle_filters(app, key),
        Focus::CmdLog => handle_cmdlog(app, key),
        Focus::Search => unreachable!("handled above"),
    }
}

fn handle_search(app: &mut App, key: KeyEvent) {
    match common::search_key(&mut app.search, key) {
        SearchAction::Idle => {}
        SearchAction::Rebuild => {
            app.rebuild_filter();
            // A changed query re-ranks the tree — snap the cursor to the top.
            app.tree_selected = 0;
            app.list_scroll = 0;
        }
        SearchAction::ClearAndExit => {
            app.rebuild_filter();
            app.tree_selected = 0;
            app.list_scroll = 0;
            app.focus = Focus::Tree;
        }
        SearchAction::Exit => app.focus = Focus::Tree,
        SearchAction::ToList(k) => {
            app.focus = Focus::Tree;
            handle_tree(app, k);
        }
    }
}

fn handle_cmdlog(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.cmd_log_scroll = app.cmd_log_scroll.saturating_add(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.cmd_log_scroll = app.cmd_log_scroll.saturating_sub(1);
        }
        KeyCode::PageUp => app.cmd_log_scroll = app.cmd_log_scroll.saturating_add(5),
        KeyCode::PageDown => app.cmd_log_scroll = app.cmd_log_scroll.saturating_sub(5),
        KeyCode::Home | KeyCode::Char('g') => app.cmd_log_scroll = usize::MAX,
        KeyCode::End | KeyCode::Char('G') => app.cmd_log_scroll = 0,
        _ => {}
    }
}

fn handle_tree(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => chat::tree_move(app, -1),
        KeyCode::Down | KeyCode::Char('j') => chat::tree_move(app, 1),
        KeyCode::PageUp => chat::tree_move(app, -10),
        KeyCode::PageDown => chat::tree_move(app, 10),
        KeyCode::Home | KeyCode::Char('g') => app.tree_selected = 0,
        KeyCode::End | KeyCode::Char('G') => chat::tree_move(app, isize::MAX),
        // Enter / → opens a conversation (moving focus to the chat) or folds
        // a group; `l` keeps the vim-style expand affordance.
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            chat::tree_activate(app);
        }
        _ => {}
    }
}

fn handle_filters(app: &mut App, key: KeyEvent) {
    match key.code {
        // Horizontal bar — arrows on either axis flip the status.
        KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
            chat::cycle_status(app, -1)
        }
        KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j') => {
            chat::cycle_status(app, 1)
        }
        KeyCode::Enter => app.focus = Focus::Tree,
        _ => {}
    }
}
