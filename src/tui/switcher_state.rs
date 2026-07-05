//! Quick-switcher state (`Ctrl+K`) — the fuzzy query, the selected row and the
//! screen it was opened from.
//!
//! A small cohesive seam off the [`App`](crate::tui::app::App) god object. The
//! switcher's *projections* (`switcher_results` / `switcher_rows` /
//! `switcher_selectable`) stay `App` methods because they rank over
//! `conversations` / `drafts` / `mentioned`; this type holds only the overlay's
//! own three fields.

use crate::domain::LineEditor;
use crate::tui::screens::Screen;

/// The `Ctrl+K` quick-switcher overlay's state. See the [module docs](self)
/// for the split with the `App`-side projections.
pub struct SwitcherState {
    /// Fuzzy query.
    pub query: LineEditor,
    /// Selected row (indexes `App::switcher_selectable`).
    pub selected: usize,
    /// Screen the switcher was opened from, restored on cancel.
    pub from: Screen,
}

impl Default for SwitcherState {
    fn default() -> Self {
        Self {
            query: LineEditor::default(),
            selected: 0,
            from: Screen::Inbox,
        }
    }
}
