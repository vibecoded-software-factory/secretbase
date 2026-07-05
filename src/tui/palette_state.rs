//! Command-palette state (`Ctrl+P`) — the fuzzy query, the selected row and
//! the screen it was opened from.
//!
//! A small cohesive seam off the [`App`](crate::tui::app::App) god object. The
//! command list + filtering (`flows::palette::palette_commands` /
//! `filtered_commands` / `palette_rows`) stay in the flow layer because they
//! are context-aware over the whole app state; this type holds only the
//! overlay's own three fields.

use crate::domain::LineEditor;
use crate::tui::screens::Screen;

/// The `Ctrl+P` command-palette overlay's state. See the [module docs](self)
/// for the split with the flow-layer command list.
pub struct PaletteState {
    /// Fuzzy query.
    pub query: LineEditor,
    /// Selected row (indexes `flows::palette::filtered_commands`).
    pub selected: usize,
    /// Screen the palette was opened from, restored on cancel / after running.
    pub from: Screen,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            query: LineEditor::default(),
            selected: 0,
            from: Screen::Inbox,
        }
    }
}
