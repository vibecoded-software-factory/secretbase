//! Pinned-message state — the open conversation's pin banner plus the
//! session/persisted bookkeeping the JSON API can't give us.
//!
//! # Why this is its own type
//!
//! The first slice of the chat-state decomposition, and the seventh seam off
//! the [`App`](crate::tui::app::App) god object. The pin subsystem is a
//! self-contained cluster of eight fields: what the open conversation's pin
//! banner shows (`msg_id` / `present` / `sender` / `envelope_id`, recomputed
//! by `App::rebuild_msg_meta` from the loaded history) and the local
//! bookkeeping that survives the API's blind spots — `local` (the only record
//! of *which* message we pinned, since `read` strips the pin payload),
//! `bodies` (out-of-window pin targets fetched on demand), `fetch_attempted`
//! (one attempt per target) and `dismissed` (GUI-parity ✕, per pin envelope).
//! They change together and read only each other, so grouping them gives the
//! subsystem one home.
//!
//! # What stays outside
//!
//! `App::set_local_pin` and `App::set_pin_dismissed` stay `App` methods,
//! because they **persist** to `config.toml` through `App`'s settings port —
//! the same bridge pattern as the other seams (the coupling to other `App`
//! state lives in one honest place). They mutate this type's maps and then
//! write the mirror into `settings_cache` + the port. The `{"method":"get"}`
//! fetch of an out-of-window pin body runs on the worker and lives in the
//! flow layer. This type holds only the data.

use std::collections::{HashMap, HashSet};

use crate::domain::Message;

/// The pinned-message subsystem's state: the banner projection plus the local
/// bookkeeping. See the [module docs](self) for the split with the persisting
/// `App` methods.
#[derive(Default)]
pub struct PinState {
    /// Message id pinned in the open conversation, derived from the most
    /// recent `Pin` system message (or [`Self::local`] when the API strips the
    /// target). `None` when there's no pin (or it's older than loaded history).
    pub msg_id: Option<u64>,
    /// Whether the loaded history contains an (undeleted) `pin` message — i.e.
    /// the conversation **has** an active pin. The JSON API strips the pin
    /// payload, so presence is often all we can know.
    pub present: bool,
    /// Sender of the newest pin message — the banner's fallback text when the
    /// pinned *target* isn't known.
    pub sender: Option<String>,
    /// Pin targets **we** set, per conversation — the only way to know *which*
    /// message is pinned, since the API doesn't carry the pin payload.
    /// Persisted to config (`pins` key) by `App::set_local_pin`.
    pub local: HashMap<String, u64>,
    /// Pinned-message bodies fetched on demand (`{"method":"get"}`) when the
    /// known target is older than the loaded window — the banner shows a real
    /// snippet instead of just the id. Keyed by conversation id; the `Message`
    /// zeroizes on drop.
    pub bodies: HashMap<String, Message>,
    /// `(conv, msg_id)` pin-body fetches already issued — one attempt per
    /// target, so a failing `get` can't loop on every reload.
    pub fetch_attempted: HashSet<(String, u64)>,
    /// Id of the newest pin **envelope** in the loaded history (the `pin`
    /// message itself, not its target) — what a local dismiss records.
    pub envelope_id: Option<u64>,
    /// **Local-only** dismissed pin banners (conv → dismissed envelope id),
    /// persisted as the `pins_dismissed` config key by `App::set_pin_dismissed`.
    /// A newer pin gets a new envelope id, so the banner revives automatically.
    pub dismissed: HashMap<String, u64>,
}
