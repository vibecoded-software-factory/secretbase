//! Select-mode state — the message multi-select cursor, marks, visual anchor,
//! viewport-align request and the sequential batch action over the selection.
//!
//! # Why this is its own type
//!
//! The second slice of the chat-state decomposition (eighth seam off the
//! [`App`](crate::tui::app::App) god object). Everything the conversation's
//! Select mode needs is here: the cursor, the marked ids, the `v` visual
//! anchor, the `zz`/`zt`/`zb` align request, whether the mode was entered from
//! Compose, and the in-progress delete/react batch. They change together and
//! nothing outside Select mode reads them.
//!
//! The batch-action machinery is driven from the flow layer (the serial
//! worker fires one request at a time and the response handlers advance it),
//! so the enum lives here but the orchestration stays in `flows::chat`. This
//! type holds only the state.
//!
//! [`AlignReq`] and [`PendingBatch`] live here too and are re-exported from
//! [`crate::tui::app`], so existing `app::AlignReq` / `app::PendingBatch`
//! paths keep working unchanged.

use std::collections::HashSet;

/// Where `zz`/`zt`/`zb` put the selected message in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignReq {
    Center,
    Top,
    Bottom,
}

/// A sequential batch of per-message operations over a multi-selection. Each
/// carries the ids **remaining** to process plus `done`/`total` for the
/// progress toast. The worker is serial, so the batch fires one request at a
/// time — each response advances to the next — instead of firing N concurrent
/// requests the busy-guard would drop.
#[derive(Clone, Debug)]
pub enum PendingBatch {
    Delete {
        remaining: Vec<u64>,
        done: usize,
        total: usize,
    },
    React {
        body: String,
        remaining: Vec<u64>,
        done: usize,
        total: usize,
    },
}

/// The conversation's Select-mode state. See the [module docs](self) for the
/// split with the flow-layer batch orchestration.
#[derive(Default)]
pub struct SelectState {
    /// Currently selected message index into the loaded history. `None` when
    /// the user is in Compose mode (default); `Some` puts the conversation in
    /// Select mode, where single-key shortcuts trigger message actions.
    pub cursor: Option<usize>,
    /// Whether the current Select-mode action (react/delete popup) was
    /// triggered from Compose via an `Alt+` shortcut. When `true`, the popup's
    /// cancel path returns the user to Compose instead of leaving them stuck
    /// in Select mode.
    pub from_compose: bool,
    /// Messages marked for a multi-select action, by **message id** — an id
    /// survives the re-read that reprojects the list (a remote edit/delete
    /// shifts every index), so a batch action can never land on the wrong
    /// message. Empty = the action falls back to the cursor message.
    pub marks: HashSet<u64>,
    /// A **sequential** batch of per-message ops (delete / react) over the
    /// multi-selection, in progress. `None` when idle.
    pub batch: Option<PendingBatch>,
    /// Anchor for `Shift+↑/↓` range shading — the fixed end of the contiguous
    /// selection while the cursor moves.
    pub anchor: Option<usize>,
    /// Armed by `z` in Select mode: the next key (`z`/`t`/`b`) aligns the
    /// selected message in the viewport (vim's `zz`/`zt`/`zb`).
    pub z_pending: bool,
    /// One-shot alignment request consumed by the next messages render.
    pub align: Option<AlignReq>,
}
