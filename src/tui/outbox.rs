//! Optimistic send outbox types — the `PendingSend` bubble and its
//! `SendState`, moved out of the [`App`](crate::tui::app::App) god object into
//! a cohesive home.
//!
//! The outbox itself is a single `Vec<PendingSend>` on `App` (kept separate
//! from `messages` so a re-read can't drop a still-pending or failed send).
//! Its flows — the optimistic echo on send, the delivery/failure flip, the
//! prune-on-reconcile and the oldest-first resend — live in `flows::chat`.
//! This module just gives the two types their own place; both are re-exported
//! from `app` so existing `app::PendingSend` / `app::SendState` paths keep
//! working unchanged.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Delivery state of an optimistic [`PendingSend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendState {
    /// In flight — waiting for the send reply.
    Pending,
    /// The send succeeded; awaiting the reconciling re-read that will replace
    /// this bubble with the real message (then it is pruned).
    Delivered,
    /// The send failed — kept on screen so the user can resend it.
    Failed,
}

/// An optimistically-rendered outgoing message (the `App::outbox` queue).
/// Shown the instant the user hits Enter and tracked to delivery, so the send
/// never feels like it waited on a round-trip — and a failure stays visible
/// with a resend affordance instead of silently vanishing.
#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct PendingSend {
    /// Conversation this send targets (matches `Conversation::id`).
    pub conv_id: String,
    /// Message body — chat content, wiped on drop.
    pub body: String,
    /// Threaded-reply target, if any.
    #[zeroize(skip)]
    pub reply_to: Option<u64>,
    /// Wall-clock send time (unix millis) for the bubble's timestamp.
    #[zeroize(skip)]
    pub sent_at_ms: u64,
    /// Current delivery state.
    #[zeroize(skip)]
    pub state: SendState,
}
