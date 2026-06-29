//! Sendable emojis for the reaction picker.
//!
//! Two sources are merged: the team's **custom** emojis (from
//! `keybase chat api emojilist`) and the **full standard Unicode emoji set**
//! (from the `emojis` crate, which embeds the official Unicode data + the
//! gemoji shortcodes that Keybase/GitHub/Slack use). Keybase's `emojilist`
//! returns only the custom ones, so the stock glyphs come from here.

/// One sendable emoji.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Emoji {
    /// Shortcode without colons (e.g. `+1`, `fire`, a team's `partyparrot`).
    /// For custom emojis this is sent as the reaction body wrapped in colons;
    /// stock emojis send their raw glyph instead (see `request_send_reaction`).
    pub alias: String,
    /// What the picker shows: the unicode glyph for stock emojis, or
    /// `:alias:` for custom (image-hosted) ones the terminal can't draw.
    pub display: String,
    /// Lowercased space-separated search terms (alias + name) the picker
    /// filters on, so "thumb" finds 👍 even though its shortcode is `+1`.
    pub keywords: String,
}

/// The **full** standard Unicode emoji set (~1.9k glyphs), each keyed by its
/// gemoji shortcode. Keybase doesn't serve the stock set, so this provides it;
/// the team's custom emojis are merged on top at runtime. Aliases are kept
/// unique so the runtime merge/dedupe is well-defined.
pub fn standard() -> Vec<Emoji> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in emojis::iter() {
        let name = e.name().to_lowercase();
        // Prefer the gemoji shortcode; fall back to a name-derived slug.
        let alias = e.shortcode().map(str::to_string).unwrap_or_else(|| {
            name.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect()
        });
        if !seen.insert(alias.clone()) {
            continue;
        }
        out.push(Emoji {
            display: e.as_str().to_string(),
            keywords: format!("{alias} {name}"),
            alias,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_is_the_full_unicode_set_and_searchable() {
        let s = standard();
        // The full set, not a curated handful.
        assert!(s.len() > 1000, "expected the full unicode emoji set");
        // "thumb" finds 👍 via its name even though the shortcode differs.
        let thumb = s.iter().find(|e| e.display == "👍").expect("thumbs up");
        assert!(thumb.keywords.contains("thumb"));
        // The alias is always part of the search terms.
        assert!(s.iter().all(|e| e.keywords.contains(&e.alias)));
        // Aliases must be unique (so the runtime merge/dedupe is well-defined).
        let mut seen = std::collections::HashSet::new();
        assert!(
            s.iter().all(|e| seen.insert(e.alias.as_str())),
            "duplicate alias in the standard set"
        );
    }
}
