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
/// the command log. `Chat` is skipped while no conversation is open. (In-chat
/// search is a `Ctrl+F` modal now, not a focusable panel.)
const FOCUS_ORDER: [Focus; 4] = [Focus::Search, Focus::Tree, Focus::Chat, Focus::CmdLog];

/// Cycles focus, skipping the chat panel when no conversation is open.
fn cycle(app: &App, forward: bool) -> Focus {
    let f = common::cycle_focus(&FOCUS_ORDER, app.focus, forward);
    if f == Focus::Chat && app.open_conv_id.is_none() {
        return common::cycle_focus(&FOCUS_ORDER, f, forward);
    }
    f
}

/// Sets focus, seating/clearing the command-log visual-select cursor as it's
/// entered/left.
fn set_focus(app: &mut App, f: Focus) {
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
/// layout: the filter on the top-left, Chats / Chat in the body, the command
/// log spanning the bottom. `None` = no neighbour that way.
fn pane_target(focus: Focus, dir: Dir, has_conv: bool) -> Option<Focus> {
    use Dir::*;
    use Focus::*;
    match (focus, dir) {
        (Search, Right) => has_conv.then_some(Chat),
        (Search, Down) => Some(Tree),
        (Tree, Up) => Some(Search),
        (Tree, Right) => has_conv.then_some(Chat),
        (Tree, Down) => Some(CmdLog),
        (Chat, Up) => Some(Search),
        (Chat, Left) => Some(Tree),
        (Chat, Down) => Some(CmdLog),
        (CmdLog, Up) => Some(Tree),
        _ => None,
    }
}

pub fn handle(app: &mut App, key: KeyEvent) {
    // `Ctrl+W` window-nav leader: each following direction (h/j/k/l or an
    // arrow) moves between panels positionally (vim-style). It **stays armed**
    // across consecutive directions, so two keys do a diagonal (e.g. k then h
    // = up-left) without re-pressing Ctrl+W. Any non-direction key exits — Esc
    // / Enter are swallowed, anything else is re-processed normally.
    if app.pending_pane_nav {
        if let Some(dir) = key_to_dir(key.code) {
            if let Some(target) = pane_target(app.focus, dir, app.open_conv_id.is_some()) {
                set_focus(app, target);
            }
            return; // keep the leader armed for the next direction
        }
        app.pending_pane_nav = false;
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            return;
        }
        // fall through: process the exit key normally
    }

    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(key.code, KeyCode::Char('w') | KeyCode::Char('W')) {
        app.pending_pane_nav = true;
        return;
    }

    // Only the truly cross-focus keys live here — the pane jumps (Alt), the
    // global find/search (Ctrl) and focus cycling. They must fire even while
    // the filter box or compose has focus, so they carry a modifier that can't
    // be confused with typed text. Per-list *actions* are bare letters routed
    // by the focused panel's own handler (`handle_tree` / compose / cmdlog),
    // the gradient convention (bare = act on this list, Shift = destructive).
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
            // Ctrl+F finds within the open conversation — opens the search modal.
            chat::open_conv_search(app);
            return;
        }
        KeyCode::Char('l') | KeyCode::Char('L') if alt => {
            set_focus(app, Focus::CmdLog);
            return;
        }
        KeyCode::Char('g') | KeyCode::Char('G') if ctrl => {
            chat::open_search_global(app);
            return;
        }
        // Tab cycles focus — unless the @-mention popup is open, where it
        // accepts the suggestion (handled in the compose handler below).
        KeyCode::Tab if !app.mention_popup_active() => {
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

    // F5 refreshes the inbox from any focus (a plain function key, safe over a
    // text field). Every other action is bare and belongs to the focused list.
    if key.code == KeyCode::F(5) {
        chat::request_load_inbox(app);
        return;
    }

    match app.focus {
        Focus::Tree => handle_tree(app, key),
        Focus::Chat => crate::tui::input::conversation::handle(app, key),
        Focus::CmdLog => handle_cmdlog(app, key),
        // Search returns early above; keep this a no-op (not a panic) so a
        // future reorder of the routing can't crash the TUI on a keypress.
        Focus::Search => {}
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
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        // Shade a contiguous range with Alt+Shift+↑/↓ or Alt+Shift+K/J — kept
        // consistent because many terminals only deliver Shift+arrows with Alt.
        KeyCode::Char('K') if alt => app.cmdlog_extend(-1),
        KeyCode::Char('J') if alt => app.cmdlog_extend(1),
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
    use crate::tui::app::ConvAction;
    match key.code {
        // ── navigation (bare letters + arrows) ────────────────────────────
        KeyCode::Up | KeyCode::Char('k') => chat::tree_move(app, -1),
        KeyCode::Down | KeyCode::Char('j') => chat::tree_move(app, 1),
        KeyCode::PageUp => chat::tree_move(app, -10),
        KeyCode::PageDown => chat::tree_move(app, 10),
        KeyCode::Home | KeyCode::Char('g') => app.tree_selected = 0,
        KeyCode::End | KeyCode::Char('G') => chat::tree_move(app, isize::MAX),
        // Enter toggles a group / opens a conversation; →/l only open or
        // expand (never collapse, so they can't loop); ←/h collapse or close.
        KeyCode::Enter => {
            chat::tree_activate(app);
        }
        KeyCode::Right | KeyCode::Char('l') => chat::tree_forward(app),
        KeyCode::Left | KeyCode::Char('h') => chat::tree_back(app),
        // `/` jumps to the filter box (the gradient search key).
        KeyCode::Char('/') => app.focus = Focus::Search,

        // ── common actions (bare — the gradient's frequent/safe tier) ──────
        KeyCode::Char('n') => chat::open_new_conversation(app),
        KeyCode::Char('r') => chat::request_load_inbox(app),
        KeyCode::Char('y') => chat::do_copy_conversation_label(app),
        KeyCode::Char('e') => chat::request_mark_read(app),
        KeyCode::Char('u') => chat::toggle_muted_conversation(app), // local-only
        KeyCode::Char('s') => chat::toggle_favorite_conversation(app), // local-only
        KeyCode::Char('t') => crate::tui::flows::teams::open_teams(app),
        KeyCode::Char('c') => chat::open_channel_browser(app), // team channels

        // ── destructive / loud actions (Shift — the gradient's danger tier) ─
        KeyCode::Char('I') => chat::open_conv_action(app, ConvAction::Ignore),
        KeyCode::Char('B') => chat::open_conv_action(app, ConvAction::Block),
        KeyCode::Char('R') => chat::open_conv_action(app, ConvAction::Report),
        KeyCode::Char('H') => chat::open_unhide(app),
        KeyCode::Char('L') => {
            app.logout_yes = false;
            app.screen = Screen::ConfirmLogout;
        }
        _ => {}
    }
}
