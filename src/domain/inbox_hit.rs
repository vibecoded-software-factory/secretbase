//! Server-side `searchinbox` result row.
//!
//! Keybase emits a large nested object per match. We project the
//! handful of fields the TUI actually uses (conversation id +
//! display name, message id + sender + body summary) so the view
//! layer can render a flat results list.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Single hit returned by `keybase chat api {"method":"searchinbox"}`.
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct InboxHit {
    /// 64-hex conversation id — used to re-key into the cached inbox
    /// when the user picks the hit.
    pub conv_id: String,
    /// Pretty conversation name (e.g. `darumapagos#general`).
    pub conv_name: String,
    /// Message id inside the conversation.
    #[zeroize(skip)]
    pub message_id: u64,
    /// Sender username of the matching message.
    pub sender: String,
    /// Short snippet of the message body for the results list. Always
    /// truncated by the renderer — the buffer carries the full
    /// `bodySummary` from Keybase.
    pub body_summary: String,
    /// Message send time, **Unix seconds** (parsed from the hit's `ctime`
    /// milliseconds). `0` when unknown. Shown under each in-conversation
    /// search result for context.
    #[zeroize(skip)]
    pub sent_at: u64,
}
