//! UI glyphs with a font-independence fallback.
//!
//! A couple of sidebar icons use **nerd-font** private-use-area glyphs, which
//! render as tofu (□) on terminals without a patched font. [`IconSet`] lets the
//! user pick the safe **Unicode** set (the default — renders on any font) or the
//! richer **nerd** set, mirroring the `emoji_style` lever: a TUI can't set the
//! terminal's font, so the only knob it has is *which glyph* to emit. Selected
//! via the `icon_style` setting (Settings → Emoji, or `config.toml`).
//!
//! Everything else the app draws (arrows, `·`, box-drawing, the braille spinner,
//! ★/●/📌) is already widely-supported Unicode and doesn't need a fallback.

/// Which glyph set the UI draws its font-dependent icons from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IconSet {
    /// Widely-supported ASCII/Unicode — renders on any font (the default).
    Unicode,
    /// Nerd-font glyphs — prettier, but need a patched (Nerd) font.
    Nerd,
}

/// Resolves the `icon_style` setting to an [`IconSet`]. Anything other than an
/// explicit `nerd` is the safe Unicode set, so an unset / typo'd value never
/// leaves the user staring at tofu.
pub fn resolve(setting: &str) -> IconSet {
    match setting.trim().to_ascii_lowercase().as_str() {
        "nerd" => IconSet::Nerd,
        _ => IconSet::Unicode,
    }
}

impl IconSet {
    /// Sidebar icon for the **Direct messages** group.
    pub fn group_dm(self) -> &'static str {
        match self {
            IconSet::Nerd => "󰭹",
            IconSet::Unicode => "@",
        }
    }

    /// Sidebar icon for a **team** group (inbox tree) / team row (Teams screen).
    pub fn group_team(self) -> &'static str {
        match self {
            IconSet::Nerd => "󰀎",
            IconSet::Unicode => "#",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_to_unicode_and_reads_nerd() {
        assert_eq!(resolve("nerd"), IconSet::Nerd);
        assert_eq!(resolve("NERD"), IconSet::Nerd);
        // Unset / unknown / the explicit unicode value all fall back to Unicode.
        assert_eq!(resolve("unicode"), IconSet::Unicode);
        assert_eq!(resolve(""), IconSet::Unicode);
        assert_eq!(resolve("garbage"), IconSet::Unicode);
    }

    #[test]
    fn both_sets_yield_non_empty_distinct_glyphs() {
        for set in [IconSet::Unicode, IconSet::Nerd] {
            assert!(!set.group_dm().is_empty());
            assert!(!set.group_team().is_empty());
            assert_ne!(set.group_dm(), set.group_team());
        }
    }
}
