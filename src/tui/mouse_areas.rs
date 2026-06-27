//! Mouse hit-testing rectangles populated by the view layer and
//! consumed by the input layer.
//!
//! The view stores screen-space `Rect`s for every clickable region
//! before returning from `draw`; mouse events are translated into
//! semantic focus targets / row indices by checking which rect
//! contains the click coordinates.

use ratatui::layout::Rect;

use crate::tui::screens::Focus;

/// Bag of all clickable rectangles for the active frame.
///
/// Reset to default at the start of every render so stale rects from
/// the previous frame can't cause phantom clicks after a resize.
///
/// `frame_size` records the terminal dimensions these rects were
/// computed against. The input layer compares it with the live
/// terminal size before honouring a click — if they differ the
/// click predates the most recent render and its coordinates map
/// to a layout that no longer exists on screen.
#[derive(Debug, Default, Clone)]
pub struct MouseAreas {
    pub search: Rect,
    pub filters: Rect,
    pub bytype: Rect,
    pub list: Rect,
    pub cmd_log: Rect,
    /// The conversation message viewport (for click-to-select + scroll).
    pub messages: Rect,
    /// One screen rect per visible message line, paired with its index in
    /// `App::messages` — for click-to-select. Built fresh each render.
    pub message_rows: Vec<(Rect, usize)>,
    /// `(width, height)` of the frame these rects were computed
    /// against. `(0, 0)` for a freshly-reset bag.
    pub frame_size: (u16, u16),
}

impl MouseAreas {
    /// Resets every rect and stamps the frame size. Called once at the
    /// top of each render (mirrors jewel's `MouseAreas::reset`).
    pub fn reset(&mut self, width: u16, height: u16) {
        *self = Self {
            frame_size: (width, height),
            ..Self::default()
        };
    }

    /// Returns the focus target containing `(col, row)`, if any.
    pub fn focus_for(&self, col: u16, row: u16) -> Option<Focus> {
        if rect_contains(self.search, col, row) {
            return Some(Focus::Search);
        }
        if rect_contains(self.filters, col, row) {
            return Some(Focus::Filters);
        }
        if rect_contains(self.bytype, col, row) {
            return Some(Focus::ByType);
        }
        if rect_contains(self.list, col, row) {
            return Some(Focus::List);
        }
        if rect_contains(self.cmd_log, col, row) {
            return Some(Focus::CmdLog);
        }
        None
    }
}

/// True when `(x, y)` falls inside `r`. Shared by [`crate::tui::input`]
/// helpers (mirrors jewel's `mouse_areas::hit_test`).
pub fn hit_test(x: u16, y: u16, r: Rect) -> bool {
    rect_contains(r, x, y)
}

fn rect_contains(r: Rect, col: u16, row: u16) -> bool {
    if r.width == 0 || r.height == 0 {
        return false;
    }
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}
