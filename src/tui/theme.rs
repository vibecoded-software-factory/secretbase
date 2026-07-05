//! Theme system.
//!
//! Reads the optional `[theme]` section of `config.toml`. Every key is
//! optional — only the entries present override the built-in defaults,
//! so partial configs are valid.
//!
//! ```toml
//! [theme]
//! accent       = "#cba6f7"   # active borders, cursor, highlights
//! inactive     = "#a6adc8"   # inactive panel borders — a *bright*, near-text
//!                            #   gray (focus = the active border, accent+bold)
//! selected_bg  = "#313244"   # selected row background
//! success      = "#a6e3a1"   # success messages
//! error        = "#f38ba8"   # error messages
//! dim          = "#9aa0b8"   # readable secondary text / counters / hints
//!                            #   (a subtext — kept legible, NOT a border tint)
//! foreground   = "#cdd6f4"   # main text (omit to inherit terminal fg)
//! placeholder  = "#505578"   # empty-input "type here…" hints (recessive)
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
    /// Readable **secondary** text (a subtext): counters, hints, timestamps,
    /// command-log detail. Kept legible — de-emphasis is hierarchy, not a
    /// near-invisible tint. For genuinely-recessive chrome use `inactive`
    /// (borders), `placeholder` (empty inputs) or `muted` (separators).
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
    /// Palette of distinguishable hues used to color **other** users' names in
    /// the message stream — one per sender, picked deterministically by
    /// [`Theme::user_color`]. Derived from the preset's raw hues so every theme
    /// gets a coherent set; `accent` is deliberately excluded so a peer never
    /// looks like *you* (your own messages render in `accent`).
    pub user_colors: Vec<Color>,
}

/// A named base palette — the raw colors a [`Preset`] is built from.
///
/// The 13 core roles are the raw colors a [`Preset`] is built from;
/// [`Theme::from_palette`] maps them — plus the app's domain-specific
/// roles — into the concrete [`Theme`] used at render time.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub base: Color,
    pub surface: Color,
    pub overlay: Color,
    pub muted: Color,
    pub text: Color,
    pub accent: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub orange: Color,
}

/// A bundled, named theme. The default (and the shared default across
/// the three TUIs) is [`Preset::CatppuccinMocha`]. Selected via
/// `name = "<preset>"` in the `[theme]` section of `config.toml`, or
/// live from the in-app theme picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    CatppuccinMocha,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    Dracula,
    Nord,
    TokyoNight,
    TokyoNightStorm,
    GruvboxDark,
    RosePine,
    Everforest,
    Kanagawa,
    OneDark,
    SolarizedDark,
    MonokaiPro,
    SolarizedLight,
    RosePineDawn,
    CatppuccinLatte,
}

impl Preset {
    /// Every bundled preset, in picker order (dark first, light last).
    pub const ALL: [Preset; 17] = [
        Preset::CatppuccinMocha,
        Preset::CatppuccinFrappe,
        Preset::CatppuccinMacchiato,
        Preset::Dracula,
        Preset::Nord,
        Preset::TokyoNight,
        Preset::TokyoNightStorm,
        Preset::GruvboxDark,
        Preset::RosePine,
        Preset::Everforest,
        Preset::Kanagawa,
        Preset::OneDark,
        Preset::SolarizedDark,
        Preset::MonokaiPro,
        Preset::SolarizedLight,
        Preset::RosePineDawn,
        Preset::CatppuccinLatte,
    ];

    /// The shared default preset across the three TUIs — used when the
    /// config names no preset.
    pub const DEFAULT: Preset = Preset::Nord;

    /// The stable config key (lower-kebab) written to `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Preset::CatppuccinMocha => "catppuccin-mocha",
            Preset::CatppuccinFrappe => "catppuccin-frappe",
            Preset::CatppuccinMacchiato => "catppuccin-macchiato",
            Preset::Dracula => "dracula",
            Preset::Nord => "nord",
            Preset::TokyoNight => "tokyonight",
            Preset::TokyoNightStorm => "tokyonight-storm",
            Preset::GruvboxDark => "gruvbox-dark",
            Preset::RosePine => "rose-pine",
            Preset::Everforest => "everforest",
            Preset::Kanagawa => "kanagawa",
            Preset::OneDark => "one-dark",
            Preset::SolarizedDark => "solarized-dark",
            Preset::MonokaiPro => "monokai-pro",
            Preset::SolarizedLight => "solarized-light",
            Preset::RosePineDawn => "rose-pine-dawn",
            Preset::CatppuccinLatte => "catppuccin-latte",
        }
    }

    /// The human-readable label shown in the picker.
    pub fn label(self) -> &'static str {
        match self {
            Preset::CatppuccinMocha => "Catppuccin Mocha",
            Preset::CatppuccinFrappe => "Catppuccin Frappé",
            Preset::CatppuccinMacchiato => "Catppuccin Macchiato",
            Preset::Dracula => "Dracula",
            Preset::Nord => "Nord",
            Preset::TokyoNight => "Tokyo Night",
            Preset::TokyoNightStorm => "Tokyo Night Storm",
            Preset::GruvboxDark => "Gruvbox Dark",
            Preset::RosePine => "Rosé Pine",
            Preset::Everforest => "Everforest",
            Preset::Kanagawa => "Kanagawa Wave",
            Preset::OneDark => "One Dark",
            Preset::SolarizedDark => "Solarized Dark",
            Preset::MonokaiPro => "Monokai Pro",
            Preset::SolarizedLight => "Solarized Light (light)",
            Preset::RosePineDawn => "Rosé Pine Dawn (light)",
            Preset::CatppuccinLatte => "Catppuccin Latte (light)",
        }
    }

    /// Resolves a config `name` value (case-insensitive) to a preset.
    pub fn from_name(name: &str) -> Option<Preset> {
        let n = name.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| p.name() == n)
    }

    /// The next preset in [`Self::ALL`], wrapping — used by the picker.
    pub fn next(self) -> Preset {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The previous preset in [`Self::ALL`], wrapping.
    pub fn prev(self) -> Preset {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// The raw base colors of this preset.
    pub fn palette(self) -> Palette {
        let h = parse_hex;
        match self {
            Preset::CatppuccinMocha => Palette {
                base: h("#1e1e2e"),
                surface: h("#313244"),
                overlay: h("#6c7086"),
                muted: h("#45475a"),
                text: h("#cdd6f4"),
                accent: h("#cba6f7"),
                red: h("#f38ba8"),
                green: h("#a6e3a1"),
                yellow: h("#f9e2af"),
                blue: h("#89b4fa"),
                magenta: h("#f5c2e7"),
                cyan: h("#94e2d5"),
                orange: h("#fab387"),
            },
            Preset::Dracula => Palette {
                base: h("#282a36"),
                surface: h("#44475a"),
                overlay: h("#6272a4"),
                muted: h("#3a3c4e"),
                text: h("#f8f8f2"),
                accent: h("#bd93f9"),
                red: h("#ff5555"),
                green: h("#50fa7b"),
                yellow: h("#f1fa8c"),
                blue: h("#8be9fd"),
                magenta: h("#ff79c6"),
                cyan: h("#8be9fd"),
                orange: h("#ffb86c"),
            },
            Preset::Nord => Palette {
                base: h("#2e3440"),
                surface: h("#3b4252"),
                overlay: h("#4c566a"),
                muted: h("#434c5e"),
                text: h("#d8dee9"),
                accent: h("#88c0d0"),
                red: h("#bf616a"),
                green: h("#a3be8c"),
                yellow: h("#ebcb8b"),
                blue: h("#81a1c1"),
                magenta: h("#b48ead"),
                cyan: h("#8fbcbb"),
                orange: h("#d08770"),
            },
            Preset::CatppuccinFrappe => Palette {
                base: h("#303446"),
                surface: h("#414559"),
                overlay: h("#737994"),
                muted: h("#51576d"),
                text: h("#c6d0f5"),
                accent: h("#ca9ee6"),
                red: h("#e78284"),
                green: h("#a6d189"),
                yellow: h("#e5c890"),
                blue: h("#8caaee"),
                magenta: h("#f4b8e4"),
                cyan: h("#81c8be"),
                orange: h("#ef9f76"),
            },
            Preset::CatppuccinMacchiato => Palette {
                base: h("#24273a"),
                surface: h("#363a4f"),
                overlay: h("#6e738d"),
                muted: h("#494d64"),
                text: h("#cad3f5"),
                accent: h("#c6a0f6"),
                red: h("#ed8796"),
                green: h("#a6da95"),
                yellow: h("#eed49f"),
                blue: h("#8aadf4"),
                magenta: h("#f5bde6"),
                cyan: h("#8bd5ca"),
                orange: h("#f5a97f"),
            },
            Preset::TokyoNight => Palette {
                base: h("#1a1b26"),
                surface: h("#292e42"),
                overlay: h("#565f89"),
                muted: h("#3b4261"),
                text: h("#c0caf5"),
                accent: h("#7aa2f7"),
                red: h("#f7768e"),
                green: h("#9ece6a"),
                yellow: h("#e0af68"),
                blue: h("#7aa2f7"),
                magenta: h("#bb9af7"),
                cyan: h("#7dcfff"),
                orange: h("#ff9e64"),
            },
            Preset::TokyoNightStorm => Palette {
                base: h("#24283b"),
                surface: h("#2f344d"),
                overlay: h("#565f89"),
                muted: h("#3b4261"),
                text: h("#c0caf5"),
                accent: h("#7aa2f7"),
                red: h("#f7768e"),
                green: h("#9ece6a"),
                yellow: h("#e0af68"),
                blue: h("#7aa2f7"),
                magenta: h("#bb9af7"),
                cyan: h("#7dcfff"),
                orange: h("#ff9e64"),
            },
            Preset::GruvboxDark => Palette {
                base: h("#282828"),
                surface: h("#3c3836"),
                overlay: h("#928374"),
                muted: h("#504945"),
                text: h("#ebdbb2"),
                accent: h("#fe8019"),
                red: h("#fb4934"),
                green: h("#b8bb26"),
                yellow: h("#fabd2f"),
                blue: h("#83a598"),
                magenta: h("#d3869b"),
                cyan: h("#8ec07c"),
                orange: h("#fe8019"),
            },
            Preset::RosePine => Palette {
                base: h("#191724"),
                surface: h("#26233a"),
                overlay: h("#6e6a86"),
                muted: h("#403d52"),
                text: h("#e0def4"),
                accent: h("#c4a7e7"),
                red: h("#eb6f92"),
                green: h("#31748f"),
                yellow: h("#f6c177"),
                blue: h("#9ccfd8"),
                magenta: h("#c4a7e7"),
                cyan: h("#ebbcba"),
                orange: h("#ebbcba"),
            },
            Preset::Everforest => Palette {
                base: h("#2d353b"),
                surface: h("#343f44"),
                overlay: h("#859289"),
                muted: h("#475258"),
                text: h("#d3c6aa"),
                accent: h("#a7c080"),
                red: h("#e67e80"),
                green: h("#a7c080"),
                yellow: h("#dbbc7f"),
                blue: h("#7fbbb3"),
                magenta: h("#d699b6"),
                cyan: h("#83c092"),
                orange: h("#e69875"),
            },
            Preset::Kanagawa => Palette {
                base: h("#1f1f28"),
                surface: h("#2a2a37"),
                overlay: h("#727169"),
                muted: h("#363646"),
                text: h("#dcd7ba"),
                accent: h("#7e9cd8"),
                red: h("#e46876"),
                green: h("#98bb6c"),
                yellow: h("#e6c384"),
                blue: h("#7e9cd8"),
                magenta: h("#957fb8"),
                cyan: h("#7aa89f"),
                orange: h("#ffa066"),
            },
            Preset::OneDark => Palette {
                base: h("#282c34"),
                surface: h("#3e4451"),
                overlay: h("#5c6370"),
                muted: h("#353b45"),
                text: h("#abb2bf"),
                accent: h("#61afef"),
                red: h("#e06c75"),
                green: h("#98c379"),
                yellow: h("#e5c07b"),
                blue: h("#61afef"),
                magenta: h("#c678dd"),
                cyan: h("#56b6c2"),
                orange: h("#d19a66"),
            },
            Preset::SolarizedDark => Palette {
                base: h("#002b36"),
                surface: h("#073642"),
                overlay: h("#586e75"),
                muted: h("#0a4753"),
                text: h("#93a1a1"),
                accent: h("#268bd2"),
                red: h("#dc322f"),
                green: h("#859900"),
                yellow: h("#b58900"),
                blue: h("#268bd2"),
                magenta: h("#d33682"),
                cyan: h("#2aa198"),
                orange: h("#cb4b16"),
            },
            Preset::MonokaiPro => Palette {
                base: h("#2d2a2e"),
                surface: h("#403e41"),
                overlay: h("#727072"),
                muted: h("#38353a"),
                text: h("#fcfcfa"),
                accent: h("#ff6188"),
                red: h("#ff6188"),
                green: h("#a9dc76"),
                yellow: h("#ffd866"),
                blue: h("#78dce8"),
                magenta: h("#ab9df2"),
                cyan: h("#78dce8"),
                orange: h("#fc9867"),
            },
            Preset::SolarizedLight => Palette {
                base: h("#fdf6e3"),
                surface: h("#eee8d5"),
                overlay: h("#93a1a1"),
                muted: h("#e4ddc8"),
                text: h("#657b83"),
                accent: h("#268bd2"),
                red: h("#dc322f"),
                green: h("#859900"),
                yellow: h("#b58900"),
                blue: h("#268bd2"),
                magenta: h("#d33682"),
                cyan: h("#2aa198"),
                orange: h("#cb4b16"),
            },
            Preset::RosePineDawn => Palette {
                base: h("#faf4ed"),
                surface: h("#f2e9e1"),
                overlay: h("#9893a5"),
                muted: h("#dfdad9"),
                text: h("#575279"),
                accent: h("#907aa9"),
                red: h("#b4637a"),
                green: h("#286983"),
                yellow: h("#ea9d34"),
                blue: h("#56949f"),
                magenta: h("#907aa9"),
                cyan: h("#d7827e"),
                orange: h("#d7827e"),
            },
            Preset::CatppuccinLatte => Palette {
                base: h("#eff1f5"),
                surface: h("#ccd0da"),
                overlay: h("#9ca0b0"),
                muted: h("#bcc0cc"),
                text: h("#4c4f69"),
                accent: h("#8839ef"),
                red: h("#d20f39"),
                green: h("#40a02b"),
                yellow: h("#df8e1d"),
                blue: h("#1e66f5"),
                magenta: h("#ea76cb"),
                cyan: h("#179299"),
                orange: h("#fe640b"),
            },
        }
    }
}

impl Theme {
    /// Emphasis / interaction style — accent + bold (the focused-title /
    /// key-letter / active-tab weight). One place instead of twenty inline
    /// `fg(accent).add_modifier(BOLD)` assemblies.
    pub fn emphasis(&self) -> ratatui::style::Style {
        ratatui::style::Style::default()
            .fg(self.accent)
            .add_modifier(ratatui::style::Modifier::BOLD)
    }

    /// Destructive / error headline style — error + bold.
    pub fn danger_title(&self) -> ratatui::style::Style {
        ratatui::style::Style::default()
            .fg(self.error)
            .add_modifier(ratatui::style::Modifier::BOLD)
    }

    /// Builds a full theme from a base [`Palette`]. The core fields map
    /// identically across the three TUIs; the secretbase-specific fields
    /// (the splash starfield + conversation markers) are derived from
    /// the palette roles so every preset gets a coherent set for free.
    pub fn from_palette(p: &Palette) -> Theme {
        Theme {
            accent: p.accent,
            // Unfocused panel borders. A **bright, near-text** gray (lazygit
            // renders inactive borders in the terminal's default foreground, so
            // they read clearly white-ish); what marks focus is the *active*
            // border going accent + bold, not the inactive one fading out.
            // Derived overlay→text 0.6 so it lifts well off the background on
            // every preset.
            inactive: mix(p.overlay, p.text, 0.6),
            selected_bg: p.surface,
            success: p.green,
            error: p.red,
            // `dim` is the **readable secondary** tier (a subtext), NOT the
            // near-border tint. Secondary text / counters / hints must stay
            // legible — de-emphasis comes from the *hierarchy* (a brighter
            // primary, the active-border accent, the selection bg), lazygit-
            // style, never from painting text almost the colour of the border.
            // The genuinely-recessive roles stay dark: `inactive` (borders),
            // `placeholder` (empty inputs), `muted` (separators). Derived as a
            // blend overlay→text so every preset gets a coherent subtext free.
            dim: mix(p.overlay, p.text, 0.5),
            foreground: p.text,
            // Empty-input "type here…" hints: recessive, but lifted slightly off
            // the background so the hint is still readable (not the raw overlay).
            placeholder: mix(p.overlay, p.text, 0.25),
            muted: p.muted,
            // Starfield: a fade from the background up toward the accent.
            star_dim: mix(p.accent, p.base, 0.78),
            star_mid: mix(p.accent, p.base, 0.45),
            star_bright: mix(p.accent, p.text, 0.25),
            conv_dm: p.blue,
            conv_team: p.magenta,
            conv_unread: p.yellow,
            // A spread of the preset's hues for per-user name colors (accent
            // excluded — that marks *your own* messages). Order is stable so a
            // given username always lands on the same hue within a preset.
            user_colors: {
                // Presets whose accent doubles as one of the hue slots
                // (tokyonight's blue, gruvbox's orange, …) must still keep
                // peers distinguishable from *you* — filter, don't trust
                // the palette author.
                let mut v = vec![
                    p.blue, p.magenta, p.cyan, p.orange, p.green, p.yellow, p.red,
                ];
                v.retain(|c| *c != p.accent);
                v.dedup();
                v
            },
        }
    }

    /// Deterministically maps a username to one of [`Theme::user_colors`], so a
    /// peer's name renders in a stable, per-user hue across the whole session
    /// (Discord/IRC-style) instead of every sender sharing one color. Falls back
    /// to `foreground` if the set is somehow empty. Uses a small FNV-1a hash so
    /// the mapping is pure and stable (no RNG, no per-run drift).
    pub fn user_color(&self, name: &str) -> Color {
        if self.user_colors.is_empty() {
            return self.foreground;
        }
        let mut h: u32 = 0x811c_9dc5;
        for b in name.as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        self.user_colors[(h as usize) % self.user_colors.len()]
    }
}

/// Decomposes a `Color` into RGB, treating non-RGB colors as black.
fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0, 0, 0),
    }
}

/// Linearly blends `a` toward `b` by `t` (0.0 = all `a`, 1.0 = all `b`).
/// Used to derive the starfield tints from palette roles.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let f = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    Color::Rgb(f(ar, br), f(ag, bg), f(ab, bb))
}

impl Default for Theme {
    fn default() -> Self {
        // The shared default across the three TUIs is Catppuccin Mocha,
        // but `foreground` stays `Reset` so text inherits the terminal
        // until the user opts into a full preset (via `name = …` or the
        // in-app picker).
        let mut t = Theme::from_palette(&Preset::DEFAULT.palette());
        t.foreground = Color::Reset;
        t
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

/// Returns the [`Preset`] named in the `[theme]` section of
/// `<config_dir>/config.toml`, if it resolves. Used to preselect the
/// in-app theme picker on the Settings screen.
pub fn configured_preset(config_dir: &Path) -> Option<Preset> {
    let text = std::fs::read_to_string(config_dir.join("config.toml")).ok()?;
    theme_name(&text).as_deref().and_then(Preset::from_name)
}

/// Extracts the raw `name = "<preset>"` value from the `[theme]`
/// section, if present. [`Preset::from_name`] validates it.
fn theme_name(text: &str) -> Option<String> {
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
        if key.trim() != "name" {
            continue;
        }
        let rest = rest.trim();
        let val = if rest.starts_with('"') {
            rest.trim_start_matches('"').split('"').next().unwrap_or("")
        } else {
            // Unquoted: stop at a trailing comment / whitespace.
            rest.split('#')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("")
        };
        if !val.is_empty() {
            return Some(val.trim().to_string());
        }
    }
    None
}

/// Parses the `[theme]` section: a `name = "<preset>"` picks the base
/// palette, then individual color keys override it.
fn parse_theme_section(text: &str) -> Theme {
    // `name` picks the base palette; per-key hex entries below override
    // it. Two passes so an override wins regardless of line order.
    let mut t = match theme_name(text).as_deref().and_then(Preset::from_name) {
        Some(p) => Theme::from_palette(&p.palette()),
        None => Theme::default(),
    };
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

    // ── Named presets ───────────────────────────────────────────

    #[test]
    fn named_preset_sets_the_base_palette() {
        let t = parse_theme_section("[theme]\nname = \"dracula\"\n");
        assert_eq!(t.accent, parse_hex("#bd93f9"));
        assert_eq!(t.error, parse_hex("#ff5555"));
        // A preset sets an explicit foreground (unlike the bare default).
        assert_eq!(t.foreground, parse_hex("#f8f8f2"));
    }

    #[test]
    fn preset_name_is_case_insensitive_and_unquoted() {
        let t = parse_theme_section("[theme]\nname = NORD\n");
        assert_eq!(t.accent, parse_hex("#88c0d0"));
    }

    #[test]
    fn explicit_keys_override_the_preset() {
        // Override wins even though `name` is declared last.
        let toml = "[theme]\naccent = \"#000000\"\nname = \"dracula\"\n";
        let t = parse_theme_section(toml);
        assert_eq!(t.accent, Color::Rgb(0, 0, 0));
        assert_eq!(t.error, parse_hex("#ff5555"));
    }

    #[test]
    fn unknown_preset_name_falls_back_to_default() {
        let t = parse_theme_section("[theme]\nname = \"solarized-zorp\"\n");
        assert_eq!(t.accent, Theme::default().accent);
        assert_eq!(t.foreground, Color::Reset);
    }

    #[test]
    fn every_preset_resolves_and_round_trips_its_name() {
        for p in Preset::ALL {
            assert_eq!(Preset::from_name(p.name()), Some(p));
            let t = Theme::from_palette(&p.palette());
            assert_ne!(t.accent, Color::Reset);
        }
    }

    fn luma(c: Color) -> u32 {
        let (r, g, b) = rgb(c);
        // Rough perceptual weighting (no need for gamma here).
        2 * r as u32 + 3 * g as u32 + b as u32
    }

    /// Both the secondary-text tier (`dim`) and the unfocused-border tier
    /// (`inactive`) must be **lifted out of the dark `overlay` band** — each
    /// sits closer to the readable `text` than the raw `overlay` does. This is
    /// the legibility invariant (the regression to guard is mapping either role
    /// back to `overlay`, which made text/borders barely legible); their
    /// relative order doesn't matter (borders may be brighter than subtext).
    #[test]
    fn dim_and_borders_are_lifted_out_of_the_dark_band() {
        for p in Preset::ALL {
            let t = Theme::from_palette(&p.palette());
            let pal = p.palette();
            let to_text = |c: Color| luma(c).abs_diff(luma(pal.text));
            let overlay_gap = to_text(pal.overlay);
            for (role, c) in [("dim", t.dim), ("inactive", t.inactive)] {
                assert_ne!(
                    c,
                    pal.overlay,
                    "{}: {role} must not be raw overlay",
                    p.name()
                );
                assert!(
                    to_text(c) < overlay_gap,
                    "{}: {role} should sit closer to text than overlay does",
                    p.name()
                );
            }
        }
    }

    #[test]
    fn preset_next_prev_wrap() {
        assert_eq!(Preset::CatppuccinMocha.prev(), Preset::CatppuccinLatte);
        assert_eq!(Preset::CatppuccinLatte.next(), Preset::CatppuccinMocha);
    }

    #[test]
    fn user_color_is_deterministic_and_from_the_set() {
        let t = Theme::from_palette(&Preset::Nord.palette());
        // Same name → same color, every time (pure hash, no RNG).
        assert_eq!(t.user_color("alice"), t.user_color("alice"));
        // The result is always one of the theme's user_colors.
        for name in ["alice", "bob", "charlie", "dave", "eve", "mallory"] {
            assert!(t.user_colors.contains(&t.user_color(name)));
        }
        // The palette spreads names across more than one hue (not all identical).
        let distinct: std::collections::HashSet<_> = [
            "alice", "bob", "charlie", "dave", "eve", "mallory", "trent", "peggy",
        ]
        .iter()
        .map(|n| t.user_color(n))
        .collect();
        assert!(distinct.len() > 1, "usernames should not all share one hue");
    }

    #[test]
    fn user_colors_exclude_accent_so_peers_never_look_like_you() {
        // Your own messages render in `accent`; a peer must never collide with
        // that, so `accent` is kept out of the per-user set on every preset.
        for p in Preset::ALL {
            let t = Theme::from_palette(&p.palette());
            assert!(
                !t.user_colors.contains(&t.accent),
                "{}: user_colors must not include accent",
                p.name()
            );
        }
    }

    #[test]
    fn user_color_falls_back_to_foreground_when_set_empty() {
        let mut t = Theme::from_palette(&Preset::Nord.palette());
        t.user_colors.clear();
        assert_eq!(t.user_color("anyone"), t.foreground);
    }
}
