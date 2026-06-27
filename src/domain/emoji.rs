//! A sendable emoji, from `keybase chat api {"method":"emojilist"}`.
//!
//! Powers the reaction picker so the user can browse/search reactions
//! instead of having to recall a `:shortcode:` from memory.

/// One sendable emoji.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Emoji {
    /// Shortcode without colons (e.g. `+1`, `fire`, a team's `partyparrot`).
    /// Sent as the reaction body, wrapped in colons.
    pub alias: String,
    /// What the picker shows: the unicode glyph for stock emojis, or
    /// `:alias:` for custom (image-hosted) ones the terminal can't draw.
    pub display: String,
}
