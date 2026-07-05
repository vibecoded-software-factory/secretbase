//! Search-overlay state — the three transient query/results modals: the
//! in-conversation search (`Ctrl+F`), the server-side inbox search (`Ctrl+G`)
//! and the giphy GIF search (`Alt+G`).
//!
//! Each is the same shape — a query editor, its result rows and the picker
//! cursor — so each gets a small cohesive struct here instead of nine loose
//! fields on the [`App`](crate::tui::app::App) god object. The *flows* that
//! run the searches (`searchregexp` / `searchinbox` / giphy) and the
//! search-jump machinery stay in `flows::chat`; these hold only the modal
//! state.

use crate::domain::{GiphyHit, InboxHit, LineEditor};

/// In-conversation search modal (`Ctrl+F`, `searchregexp`) — retained across
/// the conversation so `n`/`N` can cycle its hits.
#[derive(Default)]
pub struct ConvSearchState {
    /// Query for the search box.
    pub query: LineEditor,
    /// Matches, scoped to the open conversation.
    pub results: Vec<InboxHit>,
    /// Selected row in [`Self::results`].
    pub selected: usize,
}

/// Server-side inbox search modal (`Ctrl+G`, `searchinbox`).
#[derive(Default)]
pub struct GlobalSearchState {
    /// Query box.
    pub query: LineEditor,
    /// Hits returned for the last query.
    pub results: Vec<InboxHit>,
    /// Selected row in [`Self::results`].
    pub selected: usize,
}

/// Giphy GIF search modal (`Alt+G`) — needs the user's own giphy API key
/// (`giphy_api_key`, a setting).
#[derive(Default)]
pub struct GiphyState {
    /// Query editor.
    pub query: LineEditor,
    /// Fetched hits.
    pub results: Vec<GiphyHit>,
    /// Picker cursor into [`Self::results`].
    pub selected: usize,
}
