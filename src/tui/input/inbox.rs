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

/// Focus cycle order — the tree filter, the conversation tree, the right pane
/// (`Chat`: an open conversation, a section list, or the Find landing), the
/// command log. (In-chat search is a `Ctrl+F` modal now, not a focusable panel.)
const FOCUS_ORDER: [Focus; 4] = [Focus::Search, Focus::Tree, Focus::Chat, Focus::CmdLog];

/// Cycles focus, skipping only the command log when the `cmdlog_rows` setting
/// hides it — the right pane (`Chat`) is always reachable.
fn cycle(app: &App, forward: bool) -> Focus {
    let mut f = common::cycle_focus(&FOCUS_ORDER, app.focus, forward);
    // Skip unreachable panels: a closed chat, and the command log when the
    // `cmdlog_rows` setting hides it.
    for _ in 0..FOCUS_ORDER.len() {
        // Chat (the right pane) is always reachable now: an open conversation,
        // a section list (Teams / channel browser), or the Find landing when
        // nothing is open. Only the command log can be hidden.
        let skip = f == Focus::CmdLog && app.settings_cache.cmdlog_rows == 0;
        if !skip {
            break;
        }
        f = common::cycle_focus(&FOCUS_ORDER, f, forward);
    }
    f
}

/// Sets focus, seating/clearing the command-log visual-select cursor as it's
/// entered/left.
fn set_focus(app: &mut App, f: Focus) {
    if f == Focus::CmdLog {
        app.cmdlog.enter();
    } else if app.focus == Focus::CmdLog {
        app.cmdlog.marks.clear();
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
        // tmux's prefix+z: zoom the chat column (needs an open conversation
        // to be worth anything). Exits the leader.
        if matches!(key.code, KeyCode::Char('z') | KeyCode::Char('Z')) {
            app.pending_pane_nav = false;
            if app.open_conv_id.is_some() {
                app.pane_zoomed = !app.pane_zoomed;
                app.focus = Focus::Chat;
            }
            return;
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
        // Ctrl+W is "delete word back" in any typing surface (readline /
        // vim insert) — the pane-nav leader would eat it *and* the h/j/k/l
        // that follow, yanking focus mid-sentence. The leader only arms
        // where nothing types (tree, cmdlog, Chat in Select mode); in
        // typing surfaces the key falls through to `route_line_editor`.
        let typing =
            app.focus == Focus::Search || (app.focus == Focus::Chat && app.select.cursor.is_none());
        if !typing {
            app.pending_pane_nav = true;
            return;
        }
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
        KeyCode::Char('m') | KeyCode::Char('M') if alt => {
            // Go to the Messages section — leaving the Teams / channel-browser
            // section if we're in it, else focusing the open conversation.
            match app.screen {
                Screen::Teams => crate::tui::flows::teams::close_teams(app),
                Screen::ChannelBrowser => crate::tui::flows::chat::close_channel_browser(app),
                _ if app.open_conv_id.is_some() => set_focus(app, Focus::Chat),
                _ => {}
            }
            return;
        }
        KeyCode::Char('f') | KeyCode::Char('F') if alt => {
            // Alt+F filters the chat list; searching implies the Find landing.
            app.find_active = true;
            app.focus = Focus::Search;
            return;
        }
        KeyCode::Char('f') | KeyCode::Char('F') if ctrl && app.open_conv_id.is_some() => {
            // Ctrl+F finds within the open conversation — opens the search modal.
            chat::open_conv_search(app);
            return;
        }
        KeyCode::Char('l') | KeyCode::Char('L') if alt => {
            // Gated when the `cmdlog_rows` setting hides the panel — a
            // go-to combo must never land on an invisible target.
            if app.settings_cache.cmdlog_rows > 0 {
                set_focus(app, Focus::CmdLog);
            }
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
        // The right pane is contextual: the Teams section's list, or the chat.
        Focus::Chat if app.screen == Screen::Teams => crate::tui::input::teams::handle(app, key),
        Focus::Chat if app.screen == Screen::ChannelBrowser => {
            crate::tui::input::popups::channel_browser(app, key)
        }
        // No conversation open → the Messages overview by default, or the Find
        // search landing when the Find tab is active — not the compose handler.
        Focus::Chat if app.open_conv_id.is_none() && app.find_active => handle_find(app, key),
        Focus::Chat if app.open_conv_id.is_none() => handle_overview(app, key),
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
            // A changed query re-ranks the tree — snap both cursors to the top.
            app.tree_selected = 0;
            app.find_selected = 0;
            app.list_scroll = 0;
        }
        SearchAction::ClearAndExit => {
            app.rebuild_filter();
            app.tree_selected = 0;
            app.find_selected = 0;
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

/// The **Find a conversation** landing (right pane, nothing open): a list over
/// the filtered conversations. `↑/↓`/`j`/`k` pick, `Enter`/`l` open; `/` filters
/// (the same query as the tree), `n` new, `t` teams, `:` palette.
fn handle_find(app: &mut App, key: KeyEvent) {
    let len = app.filtered_cache.len();
    if common::list_nav(&key, len, app.find_selected, |i| app.find_selected = i) {
        return;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(&i) = app.filtered_cache.get(app.find_selected) {
                let id = app.conversations[i].id.clone();
                chat::enter_conversation(app, id);
            }
        }
        KeyCode::Char('/') => app.focus = Focus::Search,
        KeyCode::Char('n') => chat::open_new_conversation(app),
        KeyCode::Char('t') => crate::tui::flows::teams::open_teams(app),
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        _ => {}
    }
}

/// The **Messages overview** landing (right pane, nothing open, default): the
/// Unread/Mentions activity list. `↑/↓`/`j`/`k` pick, `Enter`/`l` open; `/`
/// switches to Find search, `n` new, `t` teams, `:` palette.
fn handle_overview(app: &mut App, key: KeyEvent) {
    let entries = app.overview_entries();
    if common::list_nav(&key, entries.len(), app.overview_selected, |i| {
        app.overview_selected = i
    }) {
        return;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(&i) = entries.get(app.overview_selected) {
                let id = app.conversations[i].id.clone();
                chat::enter_conversation(app, id);
            }
        }
        // `/` (and Alt+F below) flips to the Find search landing.
        KeyCode::Char('/') => {
            app.find_active = true;
            app.focus = Focus::Search;
        }
        KeyCode::Char('n') => chat::open_new_conversation(app),
        KeyCode::Char('t') => crate::tui::flows::teams::open_teams(app),
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        _ => {}
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
        KeyCode::Char('K') if alt => app.cmdlog.extend(-1),
        KeyCode::Char('J') if alt => app.cmdlog.extend(1),
        KeyCode::Up if shift => app.cmdlog.extend(-1),
        KeyCode::Down if shift => app.cmdlog.extend(1),
        // `v` anchors a visual range; j/k then extend it (vim), same as the
        // chat's Select mode.
        KeyCode::Char('v') => app.cmdlog.toggle_anchor(),
        // `:` = the command line (the palette), vim-style.
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        KeyCode::Char('u') | KeyCode::Char('U')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.cmdlog.move_cursor(-5)
        }
        KeyCode::Char('d') | KeyCode::Char('D')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.cmdlog.move_cursor(5)
        }
        KeyCode::Up | KeyCode::Char('k') => app.cmdlog.move_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.cmdlog.move_cursor(1),
        KeyCode::PageUp => app.cmdlog.move_cursor(-5),
        KeyCode::PageDown => app.cmdlog.move_cursor(5),
        KeyCode::Home | KeyCode::Char('g') => app.cmdlog.move_cursor(isize::MIN),
        KeyCode::End | KeyCode::Char('G') => app.cmdlog.move_cursor(isize::MAX),
        // Multi-select: toggle the cursor line.
        KeyCode::Char(' ') => app.cmdlog.toggle_mark(),
        // Copy the marked lines (or the cursor line): full line vs detail only.
        KeyCode::Char('y') | KeyCode::Enter => chat::do_copy_cmd_log(app, true),
        KeyCode::Char('c') => chat::do_copy_cmd_log(app, false),
        // Esc clears the selection, then (next press) leaves the panel.
        KeyCode::Esc => {
            if app.cmdlog.marks.is_empty() {
                app.focus = Focus::Tree;
            } else {
                app.cmdlog.marks.clear();
                app.cmdlog.anchor = None;
            }
        }
        _ => {}
    }
}

fn handle_tree(app: &mut App, key: KeyEvent) {
    use crate::tui::app::ConvAction;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        // `:` = the command line (the palette), vim-style.
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        // ── navigation (bare letters + arrows; Ctrl+D/U half-page) ────────
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => {
            chat::tree_move(app, -(crate::tui::app::PAGE_STEP as isize))
        }
        KeyCode::Char('d') | KeyCode::Char('D') if ctrl => {
            chat::tree_move(app, crate::tui::app::PAGE_STEP as isize)
        }
        KeyCode::Up | KeyCode::Char('k') => chat::tree_move(app, -1),
        KeyCode::Down | KeyCode::Char('j') => chat::tree_move(app, 1),
        KeyCode::PageUp => chat::tree_move(app, -(crate::tui::app::PAGE_STEP as isize)),
        KeyCode::PageDown => chat::tree_move(app, crate::tui::app::PAGE_STEP as isize),
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
