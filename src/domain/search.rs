//! Lightweight fuzzy ranking for the conversation list.
//!
//! Kept deliberately simple — Keybase's own `searchinbox` is the
//! authoritative full-text search; this layer is just for the
//! incremental "filter as you type" affordance.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::domain::conversation::{Conversation, conversation_label};

/// Per-conversation derived data — the lowercased projection used for
/// incremental search PLUS the user-facing display label that the
/// inbox / conversation views render on every frame. Both are
/// computed once at load time so the hot rendering / search paths
/// stay allocation-free.
///
/// The struct name still reads "Lowered" for historical continuity
/// — the lowercased fields are what motivated the cache originally.
/// The `display_label` was added later to amortise the
/// split/filter/join inside [`conversation_label`].
#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct LoweredConversation {
    /// `channel.name.to_lowercase()`.
    pub channel_name: String,
    /// `topic_name.to_lowercase()` (empty for DMs).
    pub topic_name: String,
    /// Creator username, lowercased.
    pub creator: String,
    /// User-visible conversation label (case-preserved). Computed
    /// with the local user's username stripped from DM lists, so
    /// `from()` requires the username at build time.
    pub display_label: String,
}

impl LoweredConversation {
    /// Builds the projection. `me` is the local user's username — it
    /// is stripped from the DM display label so the user doesn't see
    /// their own name next to every chat. Pass `None` when the
    /// session is anonymous (the label falls back to the full
    /// participant list).
    pub fn from(conv: &Conversation, me: Option<&str>) -> Self {
        Self {
            channel_name: conv.channel.name.to_lowercase(),
            topic_name: conv
                .channel
                .topic_name
                .as_deref()
                .unwrap_or("")
                .to_lowercase(),
            creator: conv
                .creator_info
                .as_ref()
                .map(|c| c.username.to_lowercase())
                .unwrap_or_default(),
            display_label: conversation_label(conv, me),
        }
    }
}

/// Scores `query` against a conversation's pre-lowercased projection.
///
/// The query is lowercased once by the caller for efficiency — pass it
/// in already-normalised — and the [`LoweredConversation`] amortises the
/// per-field lowercasing across keystrokes. Higher scores rank higher.
///
/// Weighting:
///   * channel name match → 100
///   * topic name match    →  60
///   * creator match       →  20
///
/// Substring matches always beat non-matches. The empty query returns
/// `1` for every conversation so an unsearched list keeps its input
/// order.
pub fn fuzzy_score_lowered(l: &LoweredConversation, query_lc: &str) -> u32 {
    if query_lc.is_empty() {
        return 1;
    }
    let mut score = 0u32;
    if l.channel_name.contains(query_lc) {
        score += 100;
    }
    if l.topic_name.contains(query_lc) {
        score += 60;
    }
    if l.creator.contains(query_lc) {
        score += 20;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lc(channel: &str, topic: &str, creator: &str) -> LoweredConversation {
        LoweredConversation {
            channel_name: channel.into(),
            topic_name: topic.into(),
            creator: creator.into(),
            display_label: channel.into(),
        }
    }

    #[test]
    fn empty_query_returns_one() {
        assert_eq!(fuzzy_score_lowered(&lc("foo", "bar", "baz"), ""), 1);
    }

    #[test]
    fn channel_match_outranks_topic_match() {
        let l = lc("phoenix", "general", "alice");
        assert!(fuzzy_score_lowered(&l, "phoenix") > fuzzy_score_lowered(&l, "general"));
    }

    #[test]
    fn miss_returns_zero() {
        assert_eq!(
            fuzzy_score_lowered(&lc("phoenix", "general", "alice"), "nope"),
            0
        );
    }
}
