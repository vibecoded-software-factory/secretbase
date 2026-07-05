//! Command-log state — the rolling `keybase …` command log plus its
//! visual-multi-select cursor, marks and scroll.
//!
//! # Why this is its own type
//!
//! The sixth decomposition seam off the [`App`](crate::tui::app::App) god
//! object. The command log's five fields — the entry ring buffer, the scroll
//! offset, and the visual-select cursor / marks / anchor — are one cohesive
//! panel state that changes together, and the panel's operations (enter,
//! move, extend a range, toggle a mark) were five methods on `App` reading
//! only those fields. Grouping them here also unifies the naming that had
//! drifted on `App` (`cmd_log` / `cmd_log_scroll` vs `cmdlog_cursor` /
//! `cmdlog_marks`) under one consistent `app.cmdlog.*`.
//!
//! # What stays outside: `App::push_cmd`
//!
//! Appending an entry stays an `App` method, because it bridges to state this
//! type doesn't own: it takes the elapsed duration from `App::last_op_elapsed`
//! and mirrors the line to the debug log. `App::push_cmd` builds the
//! [`CmdEntry`] and hands it to [`CmdLogState::push`], which owns the storage
//! (the ring-buffer trim + keeping the cursor/marks aligned to the trimmed
//! front). Same bridge pattern as the other seams: the coupling to other
//! `App` state lives in one honest place, the pure storage lives here.

use std::collections::HashSet;

use crate::tui::action::CmdEntry;

/// Maximum number of command-log entries kept in memory.
pub const CMD_LOG_LIMIT: usize = 50;

/// The command-log panel's state: the entry ring buffer, its scroll offset,
/// and the visual-multi-select cursor / marks / anchor. See the
/// [module docs](self) for the split with [`App::push_cmd`](crate::tui::app::App::push_cmd).
#[derive(Default)]
pub struct CmdLogState {
    /// Rolling command-log entries, oldest first, capped at [`CMD_LOG_LIMIT`].
    pub entries: Vec<CmdEntry>,
    /// Number of lines scrolled UP from the bottom. `0` keeps the latest entry
    /// pinned to the bottom-visible row; larger values walk back through
    /// history. Reset to `0` on every [`Self::push`].
    pub scroll: usize,
    /// Cursor over the log (absolute index into [`Self::entries`]) when the
    /// panel holds focus — used for the visual multi-select.
    pub cursor: usize,
    /// Lines marked for copy (absolute indices). Empty = none; copying then
    /// falls back to the cursor line.
    pub marks: HashSet<usize>,
    /// Anchor for `Shift+↑/↓` range shading — the fixed end of the contiguous
    /// selection while the cursor moves.
    pub anchor: Option<usize>,
}

impl CmdLogState {
    /// Appends `entry`, trimming the ring buffer to [`CMD_LOG_LIMIT`] from the
    /// front and keeping the visual-select cursor / marks pointing at the same
    /// entries after the trim. Resets the scroll so the freshest entry shows.
    pub fn push(&mut self, entry: CmdEntry) {
        self.entries.push(entry);
        let over = self.entries.len().saturating_sub(CMD_LOG_LIMIT);
        if over > 0 {
            self.entries.drain(..over);
            self.cursor = self.cursor.saturating_sub(over);
            self.marks = self
                .marks
                .iter()
                .filter_map(|&i| i.checked_sub(over))
                .collect();
        }
        self.scroll = 0;
    }

    /// Enters the panel: seat the cursor on the newest entry and clear any
    /// prior selection.
    pub fn enter(&mut self) {
        self.cursor = self.entries.len().saturating_sub(1);
        self.marks.clear();
        self.anchor = None;
    }

    /// Moves the cursor by `delta`, clamped to the log. While a `v` anchor is
    /// set, every motion extends the shaded range (vim visual).
    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let max = (len - 1) as isize;
        let cur = (self.cursor.min(len - 1)) as isize;
        self.cursor = cur.saturating_add(delta).clamp(0, max) as usize;
        if let Some(anchor) = self.anchor {
            let (lo, hi) = (anchor.min(self.cursor), anchor.max(self.cursor));
            self.marks = (lo..=hi).collect();
        }
    }

    /// `v` — toggle the visual anchor; turning it off clears the shading.
    pub fn toggle_anchor(&mut self) {
        if self.anchor.take().is_some() {
            self.marks.clear();
        } else if !self.entries.is_empty() {
            let cur = self.cursor.min(self.entries.len() - 1);
            self.anchor = Some(cur);
            self.marks = [cur].into_iter().collect();
        }
    }

    /// Extends a contiguous shaded selection by `delta` (`Shift+↑/↓`): the
    /// anchor holds while the cursor moves; the range is re-marked each step.
    pub fn extend(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let max = (len - 1) as isize;
        let cur = self.cursor.min(len - 1);
        let anchor = *self.anchor.get_or_insert(cur);
        let new = (cur as isize).saturating_add(delta).clamp(0, max) as usize;
        self.cursor = new;
        let (lo, hi) = (anchor.min(new), anchor.max(new));
        self.marks = (lo..=hi).collect();
    }

    /// Toggles the mark on the cursor's line (multi-select).
    pub fn toggle_mark(&mut self) {
        self.anchor = None;
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let c = self.cursor.min(len - 1);
        if !self.marks.remove(&c) {
            self.marks.insert(c);
        }
    }
}
