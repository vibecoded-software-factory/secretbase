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
    /// Interval in seconds between automatic inbox refreshes while
    /// idle. `0` disables the background refresh.
    pub inbox_refresh_secs: u64,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            clipboard_clear_secs: 30,
            list_inbox_timeout_secs: 30,
            download_timeout_secs: 300,
            auto_mark_read: true,
            inbox_refresh_secs: 30,
        }
    }
}

/// Persistent settings storage.
pub trait SettingsPort {
    /// Reads the current settings — never fails (returns defaults if
    /// the file is missing or malformed).
    fn read(&self) -> UserSettings;

    /// Writes the `auto_mark_read` flag.
    fn write_auto_mark_read(&self, auto: bool);

    /// Writes the `clipboard_clear_secs` value.
    fn write_clipboard_clear_secs(&self, secs: u64);

    /// Persists the chosen theme preset as `name = "<preset>"` inside the
    /// `[theme]` section, preserving every other key (incl. per-color
    /// overrides). Best-effort, like the other writers.
    fn write_theme_name(&self, name: &str);

    /// Directory the config file lives in — used by the theme loader.
    fn config_dir(&self) -> PathBuf;
}
