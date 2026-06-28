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

/// Focus cycle order — the tree filter, the conversation tree, the open chat,
/// the in-chat search box, the command log. `Chat` and `ChatSearch` are
/// skipped while no conversation is open.
const FOCUS_ORDER: [Focus; 5] = [
    Focus::Search,
    Focus::Tree,
    Focus::Chat,
    Focus::ChatSearch,
    Focus::CmdLog,
];

/// Cycles focus, skipping the chat-only panels when no conversation is open.
fn cycle(app: &App, forward: bool) -> Focus {
    let mut f = common::cycle_focus(&FOCUS_ORDER, app.focus, forward);
    if matches!(f, Focus::Chat | Focus::ChatSearch) && app.open_conv_id.is_none() {
        f = common::cycle_focus(&FOCUS_ORDER, f, forward);
        // Skip the second chat-only panel too (two in a row).
        if matches!(f, Focus::Chat | Focus::ChatSearch) && app.open_conv_id.is_none() {
            f = common::cycle_focus(&FOCUS_ORDER, f, forward);
        }
    }
    f
}

/// Sets focus and keeps the in-chat search mode in sync: the search box is
/// "active" exactly when `ChatSearch` holds focus, so Tab in/out toggles it.
fn set_focus(app: &mut App, f: Focus) {
    if f == Focus::ChatSearch {
        chat::open_conv_search(app);
    } else if app.focus == Focus::ChatSearch {
        chat::close_conv_search(app);
    }
    // Entering the command log seats the visual-select cursor on the newest
    // line; leaving it drops any selection.
    if f == Focus::CmdLog {
        app.enter_cmdlog();
    } else if app.focus == Focus::CmdLog {
        app.cmdlog_marks.clear();
    }
    app.focus = f;
}

/// A spatial direction for `Ctrl+W` window navigation.
#[derive(Clone, Copy)]
enum Dir {
    Left,
    Down,
    Up,
    Right,
}

/// Maps a key to a direction (`h/j/k/l` or an arrow) for window nav.
fn key_to_dir(code: KeyCode) -> Option<Dir> {
    match code {
        KeyCode::Left | KeyCode::Char('h') => Some(Dir::Left),
        KeyCode::Down | KeyCode::Char('j') => Some(Dir::Down),
        KeyCode::Up | KeyCode::Char('k') => Some(Dir::Up),
        KeyCode::Right | KeyCode::Char('l') => Some(Dir::Right),
        _ => None,
    }
}

/// The panel reached by moving `dir` from `focus`, given the Home's spatial
/// layout: filter / in-chat search on the top row, Chats / Chat in the body,
/// command log spanning the bottom. `None` = no neighbour that way.
fn pane_target(focus: Focus, dir: Dir, has_conv: bool) -> Option<Focus> {
    use Dir::*;
    use Focus::*;
    match (focus, dir) {
        (Search, Right) => has_conv.then_some(ChatSearch),
        (Search, Down) => Some(Tree),
        (ChatSearch, Left) => Some(Search),
        (ChatSearch, Down) => Some(Chat),
        (Tree, Up) => Some(Search),
        (Tree, Right) => has_conv.then_some(Chat),
        (Tree, Down) => Some(CmdLog),
        (Chat, Up) => Some(ChatSearch),
        (Chat, Left) => Some(Tree),
        (Chat, Down) => Some(CmdLog),
        (CmdLog, Up) => Some(Tree),
        _ => None,
    }
}

pub fn handle(app: &mut App, key: KeyEvent) {
    // `Ctrl+W` window-nav leader: the next key is a direction (h/j/k/l or an
    // arrow) that moves between panels positionally (vim-style). One-shot.
    if app.pending_pane_nav {
        app.pending_pane_nav = false;
        if let Some(dir) = key_to_dir(key.code)
            && let Some(target) = pane_target(app.focus, dir, app.open_conv_id.is_some())
        {
            set_focus(app, target);
        }
        return;
    }

    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(key.code, KeyCode::Char('w') | KeyCode::Char('W')) {
        app.pending_pane_nav = true;
        return;
    }

    // Modifier-gated actions and focus cycling work regardless of focus,
    // INCLUDING while the search box is focused — they can't be confused
    // with text input, and the hint bar promises the action is always
    // one keystroke away.
    match key.code {
        // ── Go-to focus: each panel's border tag is its Alt+letter combo,
        // working from any focus (even mid-compose). ─────────────────────────
        KeyCode::Char('c') | KeyCode::Char('C') if alt => {
            set_focus(app, Focus::Tree);
            return;
        }
        KeyCode::Char('m') | KeyCode::Char('M') if alt && app.open_conv_id.is_some() => {
            set_focus(app, Focus::Chat);
            return;
        }
        KeyCode::Char('f') | KeyCode::Char('F') if alt => {
            // Alt+F filters the chat list (the left search box).
            app.focus = Focus::Search;
            return;
        }
        KeyCode::Char('f') | KeyCode::Char('F') if ctrl && app.open_conv_id.is_some() => {
            // Ctrl+F finds within the open conversation (the classic find).
            set_focus(app, Focus::ChatSearch);
            return;
        }
        KeyCode::Char('l') | KeyCode::Char('L') if alt => {
            set_focus(app, Focus::CmdLog);
            return;
        }
        // ── Conversation actions ─────────────────────────────────────────────
        KeyCode::Char('y') | KeyCode::Char('Y') if alt => {
            chat::do_copy_conversation_label(app);
            return;
        }
        KeyCode::Char('e') | KeyCode::Char('E') if alt => {
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
            set_focus(app, cycle(app, true));
            return;
        }
        KeyCode::BackTab => {
            set_focus(app, cycle(app, false));
            return;
        }
        _ => {}
    }

    // The search box owns the remaining (text) keys while focused.
    if app.focus == Focus::Search {
        return handle_search(app, key);
    }

    match key.code {
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
        // ChatSearch routes to the same handler: conv_search_active is set, so
        // conversation::handle delegates to its in-conversation search keys.
        Focus::Chat | Focus::ChatSearch => crate::tui::input::conversation::handle(app, key),
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

/// Command-log panel: a visual multi-select. The cursor walks the log
/// (scrolling to stay in view), `Space` marks lines, and `y`/`Enter` copies
/// the marked lines (or the cursor line) to the clipboard.
fn handle_cmdlog(app: &mut App, key: KeyEvent) {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        // Shift+↑/↓ shade a contiguous range; plain arrows move the cursor.
        KeyCode::Up if shift => app.cmdlog_extend(-1),
        KeyCode::Down if shift => app.cmdlog_extend(1),
        KeyCode::Up | KeyCode::Char('k') => app.cmdlog_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.cmdlog_move(1),
        KeyCode::PageUp => app.cmdlog_move(-5),
        KeyCode::PageDown => app.cmdlog_move(5),
        KeyCode::Home | KeyCode::Char('g') => app.cmdlog_move(isize::MIN),
        KeyCode::End | KeyCode::Char('G') => app.cmdlog_move(isize::MAX),
        // Multi-select: toggle the cursor line.
        KeyCode::Char(' ') => app.cmdlog_toggle_mark(),
        // Copy the marked lines (or the cursor line): full line vs detail only.
        KeyCode::Char('y') | KeyCode::Enter => chat::do_copy_cmd_log(app, true),
        KeyCode::Char('c') => chat::do_copy_cmd_log(app, false),
        // Esc clears the selection, then (next press) leaves the panel.
        KeyCode::Esc => {
            if app.cmdlog_marks.is_empty() {
                app.focus = Focus::Tree;
            } else {
                app.cmdlog_marks.clear();
                app.cmdlog_anchor = None;
            }
        }
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
        // Enter / → / l opens a conversation (moving focus to the chat) or
        // folds a group (the go-to letters are global Alt+combos).
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            chat::tree_activate(app);
        }
        _ => {}
    }
}
