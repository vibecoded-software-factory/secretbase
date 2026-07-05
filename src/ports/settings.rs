//! Settings port — persistent user preferences.

use std::path::PathBuf;

/// All user-facing preferences secretbase persists.
///
/// New fields must carry a sensible default so older config files keep
/// loading after an upgrade.
#[derive(Debug, Clone)]
pub struct UserSettings {
    /// How long the clipboard is kept after a copy before it is
    /// cleared (0 disables auto-clear).
    pub clipboard_clear_secs: u64,
    /// Hard wall-clock timeout for `keybase chat api {"method":"list"}`
    /// — exposed for users with very large inboxes.
    pub list_inbox_timeout_secs: u64,
    /// Wall-clock budget for `keybase chat api {"method":"download"}`.
    /// Distinct from the read timeout because attachments can be
    /// MB-sized over slow uplinks. Default sized for a 100 MB file
    /// over 4 Mbps (~200 KB/s).
    pub download_timeout_secs: u64,
    /// Whether to mark a conversation as read when the user opens
    /// it. `true` by default — matching the behaviour of every
    /// other chat client. Set to `false` for read-only browsing
    /// (the `peek` flag is then passed to `keybase chat api read`
    /// so the messages stay unread on the server).
    pub auto_mark_read: bool,
    /// Interval in seconds for the background inbox **safety-net**
    /// resync while idle. Real-time updates come from the
    /// `keybase chat api-listen` push stream; this periodic resync only
    /// catches drift the listener doesn't push (read-state from other
    /// devices, deletions, status changes), so it can be infrequent.
    /// `0` disables it.
    pub inbox_refresh_secs: u64,
    /// Height cap for the command-log panel (rows incl. borders); `0`
    /// hides it entirely (errors still surface via the sticky toasts).
    /// The responsive floor logic still shrinks it on short terminals.
    pub cmdlog_rows: u64,
    /// Which terminal image protocol to use for inline image attachments:
    /// `auto` (detect), `kitty`, `sixel`, `iterm`, `symbols` (chafa ANSI), or
    /// `off` to disable. Every path shells out to `chafa`.
    pub image_protocol: String,
    /// chafa `--symbols` set for the symbol render path (font-dependent). E.g.
    /// `sextant+block+space` (default, widely supported), `octant+sextant+…`
    /// (denser, needs a Unicode-16 font), or `half` (works anywhere).
    pub image_symbols: String,
    /// How emoji reactions are shown: `glyph` (the Unicode character, default)
    /// or `shortcode` (the `:alias:` text — always legible even when the
    /// terminal's font renders the emoji as tofu / monochrome).
    pub emoji_style: String,
    /// Which glyph set the UI's font-dependent icons use: `unicode` (default —
    /// renders on any font, the right choice for a headless/SSH terminal without
    /// a patched font) or `nerd` (prettier nerd-font glyphs where available).
    pub icon_style: String,
    /// **Local-only** favourited conversation ids. secretbase deliberately does
    /// *not* use Keybase's `favorite` status (the CLI can't read it back, so it
    /// would drift). This is our own star, owned and persisted entirely
    /// locally — fully under our control, never synced to Keybase.
    pub favorites: Vec<String>,
    /// **Local-only** muted conversation ids. Like `favorites`, this does *not*
    /// use Keybase's `muted` status (unreadable via the CLI). It suppresses the
    /// unread indicators secretbase itself controls (the `●` dot, the bold, the
    /// unread count + filter) — our TUI has no push notifications to silence, so
    /// muting *is* "stop nagging me in the inbox". Never synced to Keybase.
    pub muted: Vec<String>,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            clipboard_clear_secs: 30,
            list_inbox_timeout_secs: 30,
            download_timeout_secs: 300,
            auto_mark_read: true,
            inbox_refresh_secs: 180,
            cmdlog_rows: 6,
            image_protocol: "auto".to_string(),
            image_symbols: "sextant+block+space".to_string(),
            emoji_style: "glyph".to_string(),
            icon_style: "unicode".to_string(),
            favorites: Vec::new(),
            muted: Vec::new(),
        }
    }
}

/// Persistent settings storage.
pub trait SettingsPort {
    /// Reads the current settings — never fails (returns defaults if
    /// the file is missing or malformed).
    fn read(&self) -> UserSettings;

    /// Persists a single top-level `key = value`, preserving every other key
    /// and section. `value` must already be TOML-formatted by the caller:
    /// bare for numbers/bools (`30`, `true`), quoted for strings (`"kitty"`).
    /// Never fatal; returns whether the write landed (`false` = read-only
    /// filesystem / full disk — the caller should inform the user that the
    /// change won't survive a restart).
    fn write_setting(&self, key: &str, value: &str) -> bool;

    /// Persists the chosen theme preset as `name = "<preset>"` inside the
    /// `[theme]` section, preserving every other key (incl. per-color
    /// overrides). Same success contract as [`Self::write_setting`].
    fn write_theme_name(&self, name: &str) -> bool;

    /// Directory the config file lives in — used by the theme loader.
    fn config_dir(&self) -> PathBuf;
}
