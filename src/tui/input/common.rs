//! Shared input helpers — the mechanics every per-screen handler used
//! to repeat (list movement, focus cycling, the search box, line-editor
//! key routing, and the y/n confirm overlay). Handlers stay thin and
//! consistent by delegating here — the shared input plumbing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::domain::LineEditor;
use crate::tui::screens::Focus;

/// Clamps a list-selection move: `current + delta` bounded to
/// `[0, len-1]`, or `0` when the list is empty.
pub fn clamp_move(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + delta).clamp(0, len as isize - 1) as usize
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
    match key.code {
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
