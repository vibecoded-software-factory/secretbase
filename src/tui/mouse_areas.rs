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
    pub source: Rect,
    pub list: Rect,
    pub cmd_log: Rect,
    /// The conversation message viewport (for click-to-select + scroll).
    pub messages: Rect,
    /// One screen rect per visible message line, paired with its index in
    /// `App::messages` — for click-to-select. Built fresh each render.
    pub message_rows: Vec<(Rect, usize)>,
    /// The compose bar's clickable `GIF` chip (opens the giphy search).
    pub compose_gif: Rect,
    /// The compose bar's clickable `emoji` chip (opens the insert picker).
    pub compose_emoji: Rect,
    /// The compose bar's clickable `attach` chip (opens the file picker).
    pub compose_attach: Rect,
    /// `(width, height)` of the frame these rects were computed
    /// against. `(0, 0)` for a freshly-reset bag.
    pub frame_size: (u16, u16),
}

impl MouseAreas {
    /// Resets every rect and stamps the frame size. Called once at the
    /// top of each render.
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
        if rect_contains(self.source, col, row) {
            return Some(Focus::Tree);
        }
        if rect_contains(self.list, col, row) {
            return Some(Focus::Chat);
        }
        if rect_contains(self.cmd_log, col, row) {
            return Some(Focus::CmdLog);
        }
        None
    }
}

/// True when `(x, y)` falls inside `r`. Shared by [`crate::tui::input`]
/// helpers.
pub fn hit_test(x: u16, y: u16, r: Rect) -> bool {
    rect_contains(r, x, y)
}

fn rect_contains(r: Rect, col: u16, row: u16) -> bool {
    if r.width == 0 || r.height == 0 {
        return false;
    }
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn rect_containment_is_inclusive_left_top_exclusive_right_bottom() {
        let rect = r(2, 3, 4, 2); // cols 2..6, rows 3..5
        assert!(hit_test(2, 3, rect));
        assert!(hit_test(5, 4, rect));
        assert!(!hit_test(6, 3, rect), "right edge is exclusive");
        assert!(!hit_test(2, 5, rect), "bottom edge is exclusive");
        assert!(!hit_test(1, 3, rect));
    }

    #[test]
    fn empty_rect_never_matches() {
        assert!(!hit_test(0, 0, r(0, 0, 0, 5)));
        assert!(!hit_test(0, 0, r(0, 0, 5, 0)));
    }

    #[test]
    fn focus_for_maps_rects_to_targets() {
        let m = MouseAreas {
            search: r(0, 0, 10, 3),
            source: r(0, 3, 10, 5),
            list: r(10, 3, 10, 5),
            cmd_log: r(0, 8, 20, 3),
            ..Default::default()
        };
        assert_eq!(m.focus_for(1, 1), Some(Focus::Search));
        assert_eq!(m.focus_for(1, 4), Some(Focus::Tree));
        assert_eq!(m.focus_for(11, 4), Some(Focus::Chat));
        assert_eq!(m.focus_for(5, 9), Some(Focus::CmdLog));
        assert_eq!(m.focus_for(19, 1), None, "outside every rect");
    }
}
