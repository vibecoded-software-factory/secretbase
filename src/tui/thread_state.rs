//! Loaded conversation thread — the open conversation's messages plus the
//! projections derived from them (the id→index lookup and the topic/headline).
//!
//! # Why this is its own type
//!
//! These three fields are the loaded read-history and what `rebuild_msg_meta`
//! derives from it in one pass; they change together on every load / append /
//! prepend. Grouping them gives the thread one home.
//!
//! # What stays on `App`
//!
//! The rebuild is a cross-cutting bridge and stays an `App` method:
//! `rebuild_msg_meta` reprojects this type **and** bumps the render-cache
//! epoch (`msg_cache_epoch`, which theme / settings / emoji changes also
//! bump), and resolves the pin subsystem. So the *coordination* lives on
//! `App`; this type holds only the data. The optimistic `outbox` is
//! deliberately **not** here — it is kept separate from `messages` so a
//! re-read can't drop a still-pending send.

use std::collections::HashMap;

use crate::domain::Message;

/// The open conversation's loaded messages + the projections derived from
/// them. See the [module docs](self) for the split with `App::rebuild_msg_meta`.
#[derive(Default)]
pub struct ThreadState {
    /// Messages of the open conversation, in chronological order (oldest
    /// first, latest last — keybase's `read` reply is newest-first, the flow
    /// reverses it).
    pub messages: Vec<Message>,
    /// Latest channel topic/headline in the loaded history — the chat's
    /// adaptive header line, cached so the render doesn't rescan per frame.
    /// Zeroized on drop (it's chat content).
    pub conv_headline: Option<zeroize::Zeroizing<String>>,
    /// `message id → index into `messages`` for O(1) lookups (reply quotes,
    /// the pin header). Rebuilt by `App::rebuild_msg_meta`.
    pub msg_index: HashMap<u64, usize>,
}
