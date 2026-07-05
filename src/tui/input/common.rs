//! Shared input helpers — the mechanics every per-screen handler used
//! to repeat (list movement, focus cycling, the search box, line-editor
//! key routing, and the y/n confirm overlay). Handlers stay thin and
//! consistent by delegating here — the shared input plumbing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::domain::LineEditor;
use crate::tui::app::App;
use crate::tui::screens::Focus;

/// Clamps a list-selection move: `current + delta` bounded to
/// `[0, len-1]`, or `0` when the list is empty.
pub fn clamp_move(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + delta).clamp(0, len as isize - 1) as usize
}

/// The single router for **universal list-movement keys**, so every list
/// (inbox tree, teams, channels, members, search results, reaction picker,
/// quick switcher, help) gets identical coverage and can't drift:
/// `↑`/`k`, `↓`/`j`, `PgUp`/`PgDn` (by [`PAGE_STEP`]), `g`/`Home` (top),
/// `G`/`End` (bottom).
///
/// Returns `true` if it consumed the key (after invoking `set` with the new
/// index), `false` otherwise so the caller can match its own keys. Handlers
/// call this right after the busy guard. It moves the selection only — screen-
/// specific keys (Enter, Esc, `/`, actions) stay in each handler.
///
/// [`PAGE_STEP`]: crate::tui::app::PAGE_STEP
pub fn list_nav(key: &KeyEvent, len: usize, sel: usize, set: impl FnOnce(usize)) -> bool {
    let page = crate::tui::app::PAGE_STEP as isize;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let new = match key.code {
        // vim half-page aliases — laptops without easy PgUp/PgDn keys.
        // Lists don't type, so Ctrl+U can't collide with kill-to-start here.
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => clamp_move(sel, -page, len),
        KeyCode::Char('d') | KeyCode::Char('D') if ctrl => clamp_move(sel, page, len),
        KeyCode::Up | KeyCode::Char('k') => clamp_move(sel, -1, len),
        KeyCode::Down | KeyCode::Char('j') => clamp_move(sel, 1, len),
        KeyCode::PageUp => clamp_move(sel, -page, len),
        KeyCode::PageDown => clamp_move(sel, page, len),
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => len.saturating_sub(1),
        _ => return false,
    };
    set(new);
    true
}

/// Movement router for lists that sit **behind a text input** (the global
/// search, reaction picker, quick switcher): only `↑`/`↓` and `PgUp`/`PgDn`
/// move the selection — the letter aliases (`j/k/g/G`) would be typed into the
/// query, and `Home`/`End` are the text cursor. Same contract as [`list_nav`].
pub fn list_nav_arrows(key: &KeyEvent, len: usize, sel: usize, set: impl FnOnce(usize)) -> bool {
    let page = crate::tui::app::PAGE_STEP as isize;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let new = match key.code {
        // fzf's home-row motion for query-driven pickers: the bare letters
        // belong to the query, but Ctrl+J/K are free in every popup.
        KeyCode::Char('k') | KeyCode::Char('K') if ctrl => clamp_move(sel, -1, len),
        KeyCode::Char('j') | KeyCode::Char('J') if ctrl => clamp_move(sel, 1, len),
        KeyCode::Up => clamp_move(sel, -1, len),
        KeyCode::Down => clamp_move(sel, 1, len),
        KeyCode::PageUp => clamp_move(sel, -page, len),
        KeyCode::PageDown => clamp_move(sel, page, len),
        _ => return false,
    };
    set(new);
    true
}

/// Next focus in `order` from `current`, wrapping. `forward = false`
/// goes backwards (Shift+Tab).
pub fn cycle_focus(order: &[Focus], current: Focus, forward: bool) -> Focus {
    if order.is_empty() {
        return current;
    }
    let pos = order.iter().position(|f| *f == current).unwrap_or(0);
    let len = order.len();
    let next = if forward {
        (pos + 1) % len
    } else {
        (pos + len - 1) % len
    };
    order[next]
}

/// True when a busy worker call should swallow this key. Esc always
/// passes through (so the user can leave / cancel).
pub fn busy_blocks(is_busy: bool, key: &KeyEvent) -> bool {
    is_busy && !matches!(key.code, KeyCode::Esc)
}

/// Whether `c` is a plain printable char (no Alt/Ctrl) — the guard for
/// type-to-search and text insertion.
pub fn is_text_input(modifiers: KeyModifiers) -> bool {
    !modifiers.intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
}

/// Routes one key into a [`LineEditor`] (cursor + edits). Returns `true`
/// if the *text changed* (caller rebuilds a filter), `false` for pure
/// cursor moves / ignored keys. Used by the search box and every popup
/// input.
pub fn route_line_editor(editor: &mut LineEditor, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        // ── word-wise editing (readline / vim-insert idioms) — modifier
        // arms first so they never fall through to the plain-key ones.
        // Living here means every input (compose, filter, popups, login)
        // gets them for free.
        KeyCode::Char('w') | KeyCode::Char('W') if ctrl => {
            editor.delete_word_back();
            true
        }
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => {
            editor.kill_to_start();
            true
        }
        KeyCode::Char('a') | KeyCode::Char('A') if ctrl => {
            editor.home();
            false
        }
        KeyCode::Char('e') | KeyCode::Char('E') if ctrl => {
            editor.end();
            false
        }
        KeyCode::Left if ctrl => {
            editor.word_left();
            false
        }
        KeyCode::Right if ctrl => {
            editor.word_right();
            false
        }
        KeyCode::Backspace => {
            editor.backspace();
            true
        }
        KeyCode::Delete => {
            editor.delete();
            true
        }
        KeyCode::Char(c) if is_text_input(key.modifiers) => {
            editor.insert(c);
            true
        }
        KeyCode::Left => {
            editor.left();
            false
        }
        KeyCode::Right => {
            editor.right();
            false
        }
        KeyCode::Home => {
            editor.home();
            false
        }
        KeyCode::End => {
            editor.end();
            false
        }
        _ => false,
    }
}

/// Outcome of a key pressed while the search box has focus. The caller
/// applies it against its own `rebuild_filter()` / focus, keeping the
/// `App` borrow disjoint from the editor borrow.
pub enum SearchAction {
    /// Cursor moved / nothing to do.
    Idle,
    /// Text changed — rebuild the filter.
    Rebuild,
    /// Clear + rebuild + leave the search box (Esc).
    ClearAndExit,
    /// Leave the search box, keep the query (Enter).
    Exit,
    /// Hand this key to the list handler (Up/Down moved off the box).
    ToList(KeyEvent),
}

/// Interprets one key for the shared search box, mutating `editor`.
pub fn search_key(editor: &mut LineEditor, key: KeyEvent) -> SearchAction {
    match key.code {
        KeyCode::Esc => {
            editor.clear();
            SearchAction::ClearAndExit
        }
        KeyCode::Enter => SearchAction::Exit,
        KeyCode::Down | KeyCode::Up => SearchAction::ToList(key),
        _ => {
            if route_line_editor(editor, key) {
                SearchAction::Rebuild
            } else {
                SearchAction::Idle
            }
        }
    }
}

/// One key for a y/n confirm overlay, decoupled from which overlay it
/// is. The caller maps each variant onto its own `*_yes` state + commit
/// action.
pub enum ConfirmInput {
    /// `y` — commit unconditionally.
    Commit,
    /// `n` / Esc — cancel.
    Cancel,
    /// Enter — commit iff the highlighted button is "yes".
    Activate,
    /// Highlight "yes" (←/h).
    Yes,
    /// Highlight "no" (→/l).
    No,
    /// Toggle the highlight (Tab/BackTab).
    Toggle,
    /// Nothing.
    Ignore,
}

/// Classifies a key for a confirm overlay (pure on the key).
pub fn confirm_key(key: KeyEvent) -> ConfirmInput {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmInput::Commit,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ConfirmInput::Cancel,
        KeyCode::Enter => ConfirmInput::Activate,
        KeyCode::Left | KeyCode::Char('h') => ConfirmInput::Yes,
        KeyCode::Right | KeyCode::Char('l') => ConfirmInput::No,
        KeyCode::Tab | KeyCode::BackTab => ConfirmInput::Toggle,
        _ => ConfirmInput::Ignore,
    }
}

/// Drives one navigable y/n confirm from a classified key. `yes` projects
/// the highlighted-button flag out of `App` (kept `false` by default so
/// **cancel** is highlighted for a destructive action); `commit` / `cancel`
/// are the two exits. Every confirm — the centered overlays *and* the
/// inline browser/member ones — routes here, so they can't drift apart.
pub fn run_confirm(
    app: &mut App,
    key: KeyEvent,
    yes: impl Fn(&mut App) -> &mut bool,
    commit: impl Fn(&mut App),
    cancel: impl Fn(&mut App),
) {
    match confirm_key(key) {
        ConfirmInput::Commit => commit(app),
        ConfirmInput::Cancel => cancel(app),
        ConfirmInput::Activate => {
            if *yes(app) {
                commit(app);
            } else {
                cancel(app);
            }
        }
        ConfirmInput::Yes => *yes(app) = true,
        ConfirmInput::No => *yes(app) = false,
        ConfirmInput::Toggle => {
            let y = yes(app);
            *y = !*y;
        }
        ConfirmInput::Ignore => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_move_bounds_and_empty() {
        assert_eq!(clamp_move(0, -1, 5), 0);
        assert_eq!(clamp_move(4, 1, 5), 4);
        assert_eq!(clamp_move(2, 1, 5), 3);
        assert_eq!(clamp_move(0, 1, 0), 0);
    }

    #[test]
    fn cycle_focus_wraps_both_ways() {
        let order = [Focus::Search, Focus::Tree];
        assert_eq!(cycle_focus(&order, Focus::Search, true), Focus::Tree);
        assert_eq!(cycle_focus(&order, Focus::Tree, true), Focus::Search);
        assert_eq!(cycle_focus(&order, Focus::Search, false), Focus::Tree);
    }
}
