//! Theme system.
//!
//! Reads the optional `[theme]` section of `config.toml`. Every key is
//! optional — only the entries present override the built-in defaults,
//! so partial configs are valid.
//!
//! ```toml
//! [theme]
//! accent       = "#cba6f7"   # active borders, cursor, highlights
//! inactive     = "#6c7086"   # inactive panel borders
//! selected_bg  = "#313244"   # selected row background
//! success      = "#a6e3a1"   # success messages
//! error        = "#f38ba8"   # error messages
//! dim          = "#585b70"   # secondary text, hints, counters
//! foreground   = "#cdd6f4"   # main text (omit to inherit terminal fg)
//! placeholder  = "#505578"   # empty-input "type here…" hints
//! muted        = "#3c3e50"   # decorative separators / barely-visible borders
//! star_dim     = "#262248"   # dimmest decorative star (splash/login)
//! star_mid     = "#5a5494"   # mid-brightness star
//! star_bright  = "#b9b2f8"   # rare bright star
//! conv_dm      = "#89b4fa"   # impteam (one-to-one) conversations
//! conv_team    = "#cba6f7"   # explicit Keybase team conversations
//! conv_unread  = "#f9e2af"   # unread badge
//! ```

use std::path::Path;

use ratatui::style::Color;

/// Resolved color palette.
#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub inactive: Color,
    pub selected_bg: Color,
    pub success: Color,
    pub error: Color,
    pub dim: Color,
    /// Main body-text color. Defaults to [`Color::Reset`] so the TUI
    /// inherits the terminal's foreground — the most portable choice.
    pub foreground: Color,
    /// "Type here…" placeholder hint inside empty input boxes.
    pub placeholder: Color,
    /// Decorative separators and barely-visible borders.
    pub muted: Color,
    /// Dimmest decorative star.
    pub star_dim: Color,
    /// Mid-brightness decorative star.
    pub star_mid: Color,
    /// Rare bright decorative star.
    pub star_bright: Color,
    /// DM (impteam) conversation marker color.
    pub conv_dm: Color,
    /// Team conversation marker color.
    pub conv_team: Color,
    /// Unread badge / "active" indicator.
    pub conv_unread: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // Restrained night-sky palette — cyan accent over a near-black
        // backdrop, mirroring the sibling TUIs. Overridable via the
        // [theme] block in `config.toml`.
        Self {
            accent: Color::Cyan,
            inactive: Color::Rgb(140, 140, 160),
            selected_bg: Color::Rgb(30, 60, 80),
            success: Color::Green,
            error: Color::Red,
            dim: Color::DarkGray,
            foreground: Color::Reset,
            placeholder: Color::Rgb(80, 85, 120),
            muted: Color::Rgb(60, 62, 80),
            star_dim: Color::Rgb(38, 34, 72),
            star_mid: Color::Rgb(90, 84, 148),
            star_bright: Color::Rgb(185, 178, 248),
            conv_dm: Color::Rgb(91, 143, 255),
            conv_team: Color::Rgb(192, 96, 224),
            conv_unread: Color::Rgb(255, 200, 0),
        }
    }
}

/// Loads the theme from the `[theme]` section of
/// `<config_dir>/config.toml`.
///
/// Returns [`Theme::default`] when the file or section is missing.
pub fn load(config_dir: &Path) -> Theme {
    let file = config_dir.join("config.toml");
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Theme::default();
    };
    parse_theme_section(&text)
}

/// Parses individual color overrides from the `[theme]` section.
fn parse_theme_section(text: &str) -> Theme {
    let mut t = Theme::default();
    let mut in_theme = false;

    for line in text.lines() {
        let line = line.trim();
        if line == "[theme]" {
            in_theme = true;
            continue;
        }
        if line.starts_with('[') {
            in_theme = false;
            continue;
        }
        if !in_theme {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = rest.trim();
        let val = if val.starts_with('"') {
            val.trim_start_matches('"')
                .split('"')
                .next()
                .unwrap_or("")
                .trim()
        } else {
            val.split(' ').next().unwrap_or("").trim()
        };
        // Length check is on *bytes*, so we must require ASCII —
        // otherwise a 7-byte multibyte string like `#€abc` passes
        // the length test and `parse_hex` panics slicing inside the
        // multibyte char. ASCII is enough for `#RRGGBB`.
        if val.len() != 7 || !val.starts_with('#') || !val.is_ascii() {
            continue;
        }
        let color = parse_hex(val);
        match key {
            "accent" => t.accent = color,
            "inactive" => t.inactive = color,
            "selected_bg" => t.selected_bg = color,
            "success" => t.success = color,
            "error" => t.error = color,
            "dim" => t.dim = color,
            "foreground" => t.foreground = color,
            "placeholder" => t.placeholder = color,
            "muted" => t.muted = color,
            "star_dim" => t.star_dim = color,
            "star_mid" => t.star_mid = color,
            "star_bright" => t.star_bright = color,
            "conv_dm" => t.conv_dm = color,
            "conv_team" => t.conv_team = color,
            "conv_unread" => t.conv_unread = color,
            _ => {}
        }
    }
    t
}

/// Parses a hex color string like `"#cba6f7"` into [`Color::Rgb`].
///
/// Defense in depth: callers already gate on `is_ascii()`, but this
/// function double-checks so any future call site can pass arbitrary
/// strings without risking a panic from byte-indexed slicing inside
/// a multibyte UTF-8 code point.
fn parse_hex(s: &str) -> Color {
    let s = s.trim_start_matches('#');
    if !s.is_ascii() || s.len() != 6 {
        return Color::Reset;
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_hex_roundtrips_known_value() {
        assert_eq!(parse_hex("#cba6f7"), Color::Rgb(0xcb, 0xa6, 0xf7));
    }

    #[test]
    fn theme_default_uses_reset_for_foreground() {
        assert_eq!(Theme::default().foreground, Color::Reset);
    }

    #[test]
    fn parse_section_overrides_only_listed_keys() {
        let toml = "\
            auto_mark_read = true\n\
            [theme]\n\
            accent = \"#112233\"\n\
            foreground = \"#445566\"\n";
        let t = parse_theme_section(toml);
        assert_eq!(t.accent, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(t.foreground, Color::Rgb(0x44, 0x55, 0x66));
        assert_eq!(t.success, Theme::default().success);
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let theme = load(tmp.path());
        assert_eq!(theme.accent, Theme::default().accent);
    }

    // ── Multibyte-safe hex parsing ──────────────────────────────

    #[test]
    fn parse_hex_returns_reset_for_multibyte_input() {
        // `€abc` is 6 bytes (€ = 3 bytes UTF-8) but only 4 chars.
        // The pre-fix code would panic slicing inside `€`; the fix
        // adds an `is_ascii()` gate so we degrade to Color::Reset
        // instead.
        assert_eq!(parse_hex("\u{20AC}abc"), Color::Reset);
        // With the `#` prefix the byte length is 7, which is exactly
        // what the caller's length guard expected — that's the case
        // that bypassed validation in the old code.
        assert_eq!(parse_hex("#\u{20AC}abc"), Color::Reset);
    }

    #[test]
    fn parse_hex_returns_reset_for_short_input() {
        assert_eq!(parse_hex("#abc"), Color::Reset);
        assert_eq!(parse_hex(""), Color::Reset);
    }

    #[test]
    fn parse_theme_section_survives_multibyte_value_without_panic() {
        // Real-world reproduction: someone writes a non-ASCII hex
        // by accident. The 7-byte string passes the byte-length
        // check; the pre-fix code panicked inside `parse_hex`.
        // Now we just skip the line and keep the default colour.
        let toml = "\
            [theme]\n\
            accent = \"#\u{20AC}abc\"\n";
        let t = parse_theme_section(toml);
        assert_eq!(t.accent, Theme::default().accent);
    }

    #[test]
    fn parse_theme_section_accepts_valid_ascii_hex_alongside_invalid() {
        // A bad colour next to a good one must not poison the
        // good one — the bad line is skipped, the good line is
        // applied.
        let toml = "\
            [theme]\n\
            accent = \"#\u{20AC}abc\"\n\
            success = \"#112233\"\n";
        let t = parse_theme_section(toml);
        assert_eq!(t.accent, Theme::default().accent);
        assert_eq!(t.success, Color::Rgb(0x11, 0x22, 0x33));
    }
}
