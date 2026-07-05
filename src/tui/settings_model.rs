//! The Settings overlay's **model**: the section/row/kind enums, the
//! per-setting metadata (labels, hints, control kinds, choice lists) and
//! the small pure helpers the adjusters use. Pure data + rules — the
//! stateful appliers (`App::settings_adjust`, `App::setting_value`) stay
//! on [`crate::tui::app::App`]; the renderer lives in `view::settings`.
//!
//! Everything here is re-exported from `tui::app`, so call sites keep
//! addressing `app::SettingId` & co. — file organisation, not an API move.

/// Which pane of the Settings overlay currently holds focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFocus {
    /// The left-hand list of sections.
    Sidebar,
    /// The right-hand panel showing the active section's options.
    Panel,
}

/// A section of the Settings overlay, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    /// Read-only: who you're signed in as and on which device.
    Identity,
    /// The live-previewing theme-preset picker.
    Theme,
    /// Chat behaviour: auto-mark-read, safety-net refresh cadence.
    Chat,
    /// Emoji display (glyph vs `:shortcode:`).
    Emoji,
    /// Clipboard auto-clear delay.
    Clipboard,
    /// `keybase` call timeouts (inbox list, attachment download).
    Network,
    /// Inline-image protocol + chafa symbol set.
    Images,
}

impl SettingsSection {
    /// Every section, in sidebar order.
    pub const ALL: [SettingsSection; 7] = [
        SettingsSection::Identity,
        SettingsSection::Theme,
        SettingsSection::Chat,
        SettingsSection::Emoji,
        SettingsSection::Clipboard,
        SettingsSection::Network,
        SettingsSection::Images,
    ];

    /// The sidebar label.
    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::Identity => "Identity",
            SettingsSection::Theme => "Theme",
            SettingsSection::Chat => "Chat",
            SettingsSection::Emoji => "Emoji",
            SettingsSection::Clipboard => "Clipboard",
            SettingsSection::Network => "Network",
            SettingsSection::Images => "Images",
        }
    }

    /// The editable / displayed rows of this section, in order. Empty for
    /// [`SettingsSection::Theme`], which uses its own preset picker.
    pub fn rows(self) -> &'static [SettingId] {
        use SettingId::*;
        match self {
            SettingsSection::Identity => &[Username, Device, DeviceType],
            SettingsSection::Theme => &[],
            SettingsSection::Chat => &[AutoMarkRead, SmartJoins, InboxRefresh, CmdlogRows],
            SettingsSection::Emoji => &[EmojiStyle, IconStyle],
            SettingsSection::Clipboard => &[ClipboardClear],
            SettingsSection::Network => &[ListTimeout, DownloadTimeout],
            SettingsSection::Images => &[ImageProtocol, ImageSymbols, WebPreviews, GiphyApiKey],
        }
    }
}

/// One setting (or read-only identity field) shown as a row in a Settings
/// panel. The kind ([`SettingId::kind`]) drives both how the value renders and
/// how `←/→` adjusts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    Username,
    Device,
    DeviceType,
    AutoMarkRead,
    SmartJoins,
    InboxRefresh,
    CmdlogRows,
    ClipboardClear,
    ListTimeout,
    DownloadTimeout,
    ImageProtocol,
    ImageSymbols,
    WebPreviews,
    GiphyApiKey,
    EmojiStyle,
    IconStyle,
}

/// The chafa symbol sets offered in the Images section (the meaningful presets
/// from the README; a hand-edited config value still loads, it just won't be in
/// this cycle).
pub const IMAGE_SYMBOL_SETS: [&str; 3] =
    ["sextant+block+space", "octant+sextant+block+space", "half"];

/// The inline-image protocols offered in the Images section.
pub const IMAGE_PROTOCOLS: [&str; 6] = ["auto", "kitty", "sixel", "iterm", "symbols", "off"];

/// Emoji display modes offered in the Emoji section: the Unicode `glyph`, or the
/// `:shortcode:` text (legible even when the terminal renders emoji as tofu).
pub const EMOJI_STYLES: [&str; 2] = ["glyph", "shortcode"];

/// UI icon sets offered in the Emoji section: `unicode` (any font, the headless-
/// safe default) or `nerd` (nerd-font glyphs where the terminal has the font).
pub const ICON_STYLES: [&str; 2] = ["unicode", "nerd"];

/// How a [`SettingId`] is displayed and adjusted.
pub enum SettingKind {
    /// Read-only label (identity fields).
    Info,
    /// Boolean on/off.
    Toggle,
    /// Numeric stepper (seconds), clamped to `[min, max]`, `0` shown as "off"
    /// when `min == 0`.
    Number { step: u64, min: u64, max: u64 },
    /// Cycle through a fixed option list.
    Choice(&'static [&'static str]),
    /// Free-text secret (API key): rendered masked, edited via an input
    /// popup (`Enter` opens, `Enter` saves, `Esc` cancels).
    Secret,
}

impl SettingId {
    /// The row label shown in the panel.
    pub fn label(self) -> &'static str {
        match self {
            SettingId::Username => "Username",
            SettingId::Device => "Device",
            SettingId::DeviceType => "Device type",
            SettingId::AutoMarkRead => "Mark read on open",
            SettingId::SmartJoins => "Hide silent joins/leaves",
            SettingId::InboxRefresh => "Inbox resync",
            SettingId::CmdlogRows => "Command log rows",
            SettingId::ClipboardClear => "Clipboard auto-clear",
            SettingId::ListTimeout => "Inbox list timeout",
            SettingId::DownloadTimeout => "Download timeout",
            SettingId::ImageProtocol => "Image protocol",
            SettingId::ImageSymbols => "Symbol set",
            SettingId::WebPreviews => "Web media (giphy)",
            SettingId::GiphyApiKey => "Giphy API key",
            SettingId::EmojiStyle => "Display",
            SettingId::IconStyle => "Icons",
        }
    }

    /// The control kind — drives rendering + adjustment.
    pub fn kind(self) -> SettingKind {
        match self {
            SettingId::Username | SettingId::Device | SettingId::DeviceType => SettingKind::Info,
            SettingId::AutoMarkRead | SettingId::SmartJoins | SettingId::WebPreviews => {
                SettingKind::Toggle
            }
            SettingId::GiphyApiKey => SettingKind::Secret,
            SettingId::CmdlogRows => SettingKind::Number {
                step: 1,
                min: 0,
                max: 6,
            },
            SettingId::InboxRefresh => SettingKind::Number {
                step: 30,
                min: 0,
                max: 3600,
            },
            SettingId::ClipboardClear => SettingKind::Number {
                step: 5,
                min: 0,
                max: 600,
            },
            SettingId::ListTimeout => SettingKind::Number {
                step: 5,
                min: 5,
                max: 600,
            },
            SettingId::DownloadTimeout => SettingKind::Number {
                step: 30,
                min: 10,
                max: 3600,
            },
            SettingId::ImageProtocol => SettingKind::Choice(&IMAGE_PROTOCOLS),
            SettingId::ImageSymbols => SettingKind::Choice(&IMAGE_SYMBOL_SETS),
            SettingId::EmojiStyle => SettingKind::Choice(&EMOJI_STYLES),
            SettingId::IconStyle => SettingKind::Choice(&ICON_STYLES),
        }
    }

    /// A short hint shown under the focused row.
    pub fn hint(self) -> &'static str {
        match self.kind() {
            SettingKind::Info => "read-only",
            SettingKind::Toggle => "←/→ or Enter to toggle",
            SettingKind::Number { .. } => "←/→ to adjust",
            SettingKind::Choice(_) => "←/→ to choose",
            SettingKind::Secret => "Enter to edit (stored in config.toml, never logged)",
        }
    }
}

/// `s` or an em-dash placeholder when empty (read-only identity fields).
pub(crate) fn or_dash(s: &str) -> String {
    if s.is_empty() {
        "—".to_string()
    } else {
        s.to_string()
    }
}

/// `cur ± delta·step`, clamped to `[min, max]` (saturating, no underflow).
pub(crate) fn step_clamp(cur: u64, delta: isize, step: u64, min: u64, max: u64) -> u64 {
    let next = cur as isize + delta * step as isize;
    next.clamp(min as isize, max as isize) as u64
}

/// The option `delta` steps from `cur` in `opts`, wrapping. Falls back to the
/// first option when `cur` isn't in the list (a hand-edited config value).
pub(crate) fn cycle(opts: &[&str], cur: &str, delta: isize) -> String {
    let n = opts.len() as isize;
    let i = opts.iter().position(|&o| o == cur).unwrap_or(0) as isize;
    opts[(((i + delta) % n + n) % n) as usize].to_string()
}
