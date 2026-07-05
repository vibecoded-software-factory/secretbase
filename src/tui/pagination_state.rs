//! Message viewport state — how the open conversation's history is scrolled
//! and paged: the scroll offset, the older-page cursor, and the derived cues
//! (max-back, new-since-scroll, the new-messages divider anchor).
//!
//! # Why this is its own type
//!
//! The third slice of the chat-state decomposition (ninth seam off the
//! [`App`](crate::tui::app::App) god object). These eight fields describe one
//! thing — where the reader is in the loaded history and how the next older
//! page loads — and they move together across the read handlers, the input
//! layer and the renderer. Grouping them de-stutters the names
//! (`app.pagination.scroll` vs `messages_scroll` + `messages_next` + …) and
//! gives the message viewport one home.
//!
//! # What stays outside
//!
//! The pagination *flows* — `request_load_older_messages`, the read handlers
//! that prepend older pages and shift the Select cursor, `maybe_backfill` —
//! live in `flows::chat`; they read and assign this state but the worker
//! coupling stays there. This type holds only the fields. `messages` itself
//! (and its `msg_index` / render-cache epoch) stays on `App` for now — the
//! hard invalidation core is a later slice.

/// The open conversation's message-viewport scroll + pagination state. See the
/// [module docs](self) for the split with the read-handler flows.
#[derive(Default)]
pub struct PaginationState {
    /// First visible row in the message view. The renderer pins the "latest"
    /// line to the bottom by default; this offset scrolls back into history.
    pub scroll: usize,
    /// When set, the next `read` reply **keeps** the current scroll offset
    /// instead of snapping to the latest message. Set by control-op re-reads
    /// (delete / edit / react) so acting on a message you scrolled up to
    /// doesn't yank you back to the bottom. Consumed by the read handler.
    pub preserve_scroll: bool,
    /// Cursor for the next *older* page, supplied by the Keybase service in the
    /// previous `read` reply. `None` once it signals the bottom of history.
    pub next: Option<String>,
    /// Whether a pagination call is in flight — debounces repeated Up presses
    /// while the next older page loads.
    pub loading_older: bool,
    /// The maximum bottom-relative scroll offset the view rendered last frame.
    /// The input handler reads it to know when the reader has hit the top of
    /// loaded history (so it can trigger a pagination fetch).
    pub max_back: usize,
    /// Count of messages that arrived (via push) while the reader was
    /// **scrolled up** away from the latest — drives the floating
    /// "▼ N new · End" jump-to-latest pill. Reset to 0 once back at the bottom
    /// (or on open/close).
    pub new_since: usize,
    /// Remaining auto-backfill budget (older pages the read handlers may chain
    /// without user input) — a projection-heavy conversation can collapse a
    /// 50-slot page to one visible message, so an open/reload backfills until
    /// the viewport has something to scroll over. Reset per fresh load.
    pub backfill_pages: u8,
    /// For the **currently open** conversation, the message id below-or-equal
    /// to which everything was already seen last time it was open — the anchor
    /// for the `new messages` divider. `None` on a first-ever open (no
    /// baseline) or when nothing new has arrived. Set on open, cleared on close.
    pub unread_boundary: Option<u64>,
}
