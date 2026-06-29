//! [`crate::ports::SettingsPort`] implementation backed by a single
//! `~/.config/secretbase/config.toml` file.
//!
//! The parser is intentionally hand-rolled and forgiving — only the keys
//! recognised by [`UserSettings`] are read; everything else (including
//! the `[theme]` section consumed by the TUI) is preserved verbatim on
//! rewrites.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::ports::{SettingsPort, UserSettings};

/// Default clipboard auto-clear delay (30 seconds). Set
/// `clipboard_clear_secs = 0` to disable.
const DEFAULT_CLIPBOARD_CLEAR_SECS: u64 = 30;

/// Default wall-clock budget for `keybase chat api {"method":"list"}`.
/// Sized for a healthy decrypt of a large inbox on a slow machine
/// without masking a wedged child for too long. Override with
/// `list_inbox_timeout_secs = N` in `config.toml`.
const DEFAULT_LIST_INBOX_TIMEOUT_SECS: u64 = 30;

/// Default wall-clock budget for
/// `keybase chat api {"method":"download"}`. Picked to cover a
/// ~100 MB attachment over a 4 Mbps uplink with margin; override
/// with `download_timeout_secs = N` for larger files / slower links.
const DEFAULT_DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// Default cadence (in seconds) for the background inbox **safety-net**
/// resync. Real-time updates arrive via the `keybase chat api-listen`
/// push stream, so this only needs to catch drift the listener doesn't
/// push — hence a relaxed default. Set `inbox_refresh_secs = 0` to
/// disable.
const DEFAULT_INBOX_REFRESH_SECS: u64 = 180;

/// Owner-only file mode for `config.toml`. The file does not carry
/// credentials (Keybase keeps those in the local service) but it can
/// carry user preferences and identifiers, so we keep it private.
const CONFIG_FILE_MODE: u32 = 0o600;

/// Owner-only directory mode for `~/.config/secretbase/`.
const CONFIG_DIR_MODE: u32 = 0o700;

/// File-backed settings adapter.
#[derive(Debug, Clone)]
pub struct TomlSettingsAdapter {
    dir: PathBuf,
}

impl TomlSettingsAdapter {
    /// Builds an adapter rooted at `~/.config/secretbase/`.
    ///
    /// Falls back to the current directory when `$HOME` is unset.
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Self {
            dir: home.join(".config").join("secretbase"),
        }
    }

    /// Returns the absolute path to `config.toml`.
    fn file(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    /// Ensures the config directory exists with `0o700` perms.
    fn ensure_dir(&self) {
        let _ = fs::DirBuilder::new()
            .recursive(true)
            .mode(CONFIG_DIR_MODE)
            .create(&self.dir);
        let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(CONFIG_DIR_MODE));
    }

    /// Rewrites `config.toml`, preserving any keys we don't manage
    /// (e.g. the `[theme]` block).
    fn rewrite(&self, updater: impl Fn(&mut UpdateBuffer)) {
        self.ensure_dir();
        let existing = fs::read_to_string(self.file()).unwrap_or_default();
        let mut buf = UpdateBuffer::parse(&existing);
        updater(&mut buf);
        write_file_secure(&self.file(), &buf.render());
    }
}

/// Atomically replaces `path` with `contents`.
///
/// Writes to a sibling `<file>.tmp` first, `fsync`s it, then uses
/// `rename(2)` (atomic on the same filesystem) to swap into place.
/// The destination either reflects the previous contents or the new
/// ones — never a half-truncated draft, even on power loss between
/// the truncate and the write.
///
/// All errors are silently swallowed and the destination is left
/// untouched, mirroring the original "settings writes never break
/// the TUI" contract.
fn write_file_secure(path: &Path, contents: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    let mut tmp_name = path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    if tmp_name.is_empty() {
        return;
    }
    tmp_name.push(".tmp");
    let tmp_path = parent.join(tmp_name);

    {
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(CONFIG_FILE_MODE)
            .open(&tmp_path)
        {
            Ok(f) => f,
            Err(_) => return,
        };
        if file.write_all(contents.as_bytes()).is_err() {
            let _ = fs::remove_file(&tmp_path);
            return;
        }
        // Flush data + metadata so the bytes reach disk before the
        // rename. Without this, a crash between write and rename
        // could leave the swapped-in file empty on next boot.
        let _ = file.sync_all();
    }

    if fs::rename(&tmp_path, path).is_err() {
        // Rename failed: leave the original intact and clean up
        // the temp file rather than poisoning the slot.
        let _ = fs::remove_file(&tmp_path);
        return;
    }

    // `rename(2)` preserves the source mode, which was set to 0600
    // by `.mode()` above. The explicit `set_permissions` defends
    // against a pre-existing tmp that someone chmod'd out of band
    // between two adjacent writes — practically a no-op.
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(CONFIG_FILE_MODE));
}

impl Default for TomlSettingsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsPort for TomlSettingsAdapter {
    fn read(&self) -> UserSettings {
        self.ensure_dir();
        let mut cfg = UserSettings {
            clipboard_clear_secs: DEFAULT_CLIPBOARD_CLEAR_SECS,
            list_inbox_timeout_secs: DEFAULT_LIST_INBOX_TIMEOUT_SECS,
            download_timeout_secs: DEFAULT_DOWNLOAD_TIMEOUT_SECS,
            auto_mark_read: true,
            inbox_refresh_secs: DEFAULT_INBOX_REFRESH_SECS,
            image_protocol: "auto".to_string(),
            image_symbols: "sextant+block+space".to_string(),
        };
        let Ok(text) = fs::read_to_string(self.file()) else {
            return cfg;
        };
        // Tighten perms on the pre-existing file too. Without this,
        // a config.toml created by an external tool (or carried
        // over from an older secretbase build with looser perms)
        // would stay readable to group/others until the next time
        // the TUI happened to *write* — which might be never.
        let _ = fs::set_permissions(self.file(), fs::Permissions::from_mode(CONFIG_FILE_MODE));

        // Section-aware: only the implicit top-level table holds our
        // keys. Anything after a `[section]` header (e.g. `[theme]`)
        // is ignored — TOML has no syntax for "exit" back to top
        // level, so once we cross a header we stay in skip mode.
        let mut in_section = false;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_section = true;
                continue;
            }
            if in_section {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = unquote(strip_inline_comment(value.trim()));
            match key {
                "clipboard_clear_secs" => {
                    if let Ok(n) = value.parse::<u64>() {
                        cfg.clipboard_clear_secs = n;
                    }
                }
                "list_inbox_timeout_secs" => {
                    if let Ok(n) = value.parse::<u64>()
                        && n > 0
                    {
                        cfg.list_inbox_timeout_secs = n;
                    }
                }
                "download_timeout_secs" => {
                    if let Ok(n) = value.parse::<u64>()
                        && n > 0
                    {
                        cfg.download_timeout_secs = n;
                    }
                }
                "auto_mark_read" => {
                    cfg.auto_mark_read = value == "true";
                }
                "inbox_refresh_secs" => {
                    if let Ok(n) = value.parse::<u64>() {
                        cfg.inbox_refresh_secs = n;
                    }
                }
                "image_protocol" if !value.is_empty() => {
                    cfg.image_protocol = value.to_ascii_lowercase();
                }
                "image_symbols" if !value.is_empty() => {
                    cfg.image_symbols = value.to_ascii_lowercase();
                }
                _ => {}
            }
        }
        cfg
    }

    fn write_auto_mark_read(&self, auto: bool) {
        self.rewrite(|buf| {
            buf.set("auto_mark_read", if auto { "true" } else { "false" });
        });
    }

    fn write_clipboard_clear_secs(&self, secs: u64) {
        self.rewrite(|buf| {
            buf.set("clipboard_clear_secs", &secs.to_string());
        });
    }

    fn write_theme_name(&self, name: &str) {
        let name = name.to_string();
        self.rewrite(move |buf| {
            buf.set_theme_name(&name);
        });
    }

    fn config_dir(&self) -> PathBuf {
        self.dir.clone()
    }
}

/// Returns the leading portion of `s` before the first `#` that
/// appears outside a double-quoted region. The trailing whitespace
/// before the `#` is trimmed.
///
/// TOML allows `# comment` at end-of-line; for our keys (numeric /
/// boolean) the quote-tracking is overkill but keeps the helper
/// safe to reuse for future string-valued keys.
fn strip_inline_comment(s: &str) -> &str {
    let mut in_quote = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return s[..i].trim_end(),
            _ => {}
        }
    }
    s
}

/// Strips one outer layer of double quotes from `s` when both ends
/// are quoted (`"foo"` → `foo`). Single-quote literals and other
/// shapes are returned unchanged — we only need the basic TOML
/// quoted-string flavour to round-trip the four keys we own.
fn unquote(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Lightweight TOML buffer for round-trip rewrites that preserve any
/// keys/sections we don't recognise (e.g. `[theme]`).
struct UpdateBuffer {
    /// All non-`[theme]` lines, in their original order — with the
    /// keys we own already filtered out.
    preserved: Vec<String>,
    /// `key = value` pairs we own. Re-rendered in stable order so
    /// the file does not churn on every write.
    owned: Vec<(String, String)>,
}

impl UpdateBuffer {
    /// Recognised keys — used to detect "lines we own" while parsing
    /// the existing file. Anything outside this list is preserved
    /// verbatim.
    const OWNED: &'static [&'static str] = &[
        "clipboard_clear_secs",
        "list_inbox_timeout_secs",
        "download_timeout_secs",
        "auto_mark_read",
        "inbox_refresh_secs",
        "image_protocol",
        "image_symbols",
    ];

    fn parse(text: &str) -> Self {
        let mut preserved = Vec::new();
        let mut owned: Vec<(String, String)> = Vec::new();
        let mut in_other_section = false;

        for raw in text.lines() {
            let trimmed = raw.trim();
            // Any non-empty `[section]` header that is not the implicit
            // top-level switches us into "preserve verbatim" mode for
            // every subsequent line until the next top-level section.
            if trimmed.starts_with('[') {
                in_other_section = true;
                preserved.push(raw.to_string());
                continue;
            }
            if in_other_section {
                preserved.push(raw.to_string());
                continue;
            }
            // At top level: split on '=' and check if we own the key.
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                if Self::OWNED.contains(&key) {
                    let val = v.trim().to_string();
                    // Replace if we already saw this key earlier.
                    if let Some(existing) = owned.iter_mut().find(|(ek, _)| ek == key) {
                        existing.1 = val;
                    } else {
                        owned.push((key.to_string(), val));
                    }
                    continue;
                }
            }
            preserved.push(raw.to_string());
        }

        Self { preserved, owned }
    }

    /// Sets an owned key (overwrites if present, appends otherwise).
    fn set(&mut self, key: &str, value: &str) {
        if let Some(existing) = self.owned.iter_mut().find(|(k, _)| k == key) {
            existing.1 = value.to_string();
        } else {
            self.owned.push((key.to_string(), value.to_string()));
        }
    }

    /// Sets `name = "<name>"` inside the preserved `[theme]` section,
    /// replacing any existing `name` line there; creates the section
    /// when absent. The `[theme]` block belongs to the theme loader, so
    /// we touch only the single `name` line.
    fn set_theme_name(&mut self, name: &str) {
        match self.preserved.iter().position(|l| l.trim() == "[theme]") {
            Some(h) => {
                // Drop an existing `name` line within this section.
                let mut i = h + 1;
                while i < self.preserved.len() {
                    let t = self.preserved[i].trim();
                    if t.starts_with('[') {
                        break; // next section starts
                    }
                    if t.split_once('=').is_some_and(|(k, _)| k.trim() == "name") {
                        self.preserved.remove(i);
                        continue;
                    }
                    i += 1;
                }
                self.preserved.insert(h + 1, format!("name = \"{name}\""));
            }
            None => {
                self.preserved.push("[theme]".to_string());
                self.preserved.push(format!("name = \"{name}\""));
            }
        }
    }

    /// Renders the buffer back to a TOML string.
    fn render(&self) -> String {
        let mut out = String::new();
        // Owned section first — keeps the top of the file canonical so
        // diffs against the previous version stay small.
        for (k, v) in &self.owned {
            out.push_str(&format!("{k} = {v}\n"));
        }
        // Preserved tail — everything else, verbatim.
        if !self.preserved.is_empty() && !out.is_empty() {
            out.push('\n');
        }
        for line in &self.preserved {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn adapter_in(tmp: &TempDir) -> TomlSettingsAdapter {
        TomlSettingsAdapter {
            dir: tmp.path().to_path_buf(),
        }
    }

    #[test]
    fn read_returns_defaults_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, DEFAULT_CLIPBOARD_CLEAR_SECS);
        assert_eq!(cfg.list_inbox_timeout_secs, DEFAULT_LIST_INBOX_TIMEOUT_SECS);
        assert_eq!(cfg.download_timeout_secs, DEFAULT_DOWNLOAD_TIMEOUT_SECS);
        assert!(cfg.auto_mark_read);
    }

    #[test]
    fn read_picks_up_download_timeout_override() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        fs::write(a.file(), "download_timeout_secs = 900\n").unwrap();
        let cfg = a.read();
        assert_eq!(cfg.download_timeout_secs, 900);
    }

    #[test]
    fn download_timeout_zero_keeps_default() {
        // Mirror the list_inbox_timeout_secs semantics: `0` means
        // "don't override", not "no timeout".
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        fs::write(a.file(), "download_timeout_secs = 0\n").unwrap();
        let cfg = a.read();
        assert_eq!(cfg.download_timeout_secs, DEFAULT_DOWNLOAD_TIMEOUT_SECS);
    }

    #[test]
    fn write_then_read_round_trips_owned_keys() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.write_auto_mark_read(false);
        a.write_clipboard_clear_secs(120);
        let cfg = a.read();
        assert!(!cfg.auto_mark_read);
        assert_eq!(cfg.clipboard_clear_secs, 120);
    }

    #[test]
    fn rewrites_preserve_theme_block_verbatim() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        fs::write(
            a.file(),
            "auto_mark_read = true\n[theme]\naccent = \"#abcdef\"\n",
        )
        .unwrap();
        a.write_auto_mark_read(false);
        let txt = fs::read_to_string(a.file()).unwrap();
        assert!(txt.contains("[theme]"));
        assert!(txt.contains("accent = \"#abcdef\""));
        assert!(txt.contains("auto_mark_read = false"));
    }

    // ── Section-scoped + quote-aware parsing ────────────────────

    fn write_config(a: &TomlSettingsAdapter, body: &str) {
        a.ensure_dir();
        fs::write(a.file(), body).unwrap();
    }

    #[test]
    fn read_ignores_keys_inside_other_sections() {
        // Same-named keys inside `[theme]` (or any non-top-level
        // section) must NOT override the top-level defaults. Real
        // TOML semantics — and matches what `UpdateBuffer::parse`
        // does for round-trip preservation.
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        write_config(
            &a,
            "[theme]\n\
             clipboard_clear_secs = 999\n\
             auto_mark_read = false\n",
        );
        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, DEFAULT_CLIPBOARD_CLEAR_SECS);
        assert!(cfg.auto_mark_read, "auto_mark_read should keep its default");
    }

    #[test]
    fn read_accepts_quoted_boolean() {
        // TOML lets users wrap values in quotes. `auto_mark_read =
        // "true"` used to be silently treated as `false` because the
        // value contained the literal `"true"` (quotes included)
        // which is not equal to `true`.
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        write_config(&a, "auto_mark_read = \"true\"\n");
        let cfg = a.read();
        assert!(cfg.auto_mark_read);
    }

    #[test]
    fn read_accepts_quoted_numeric() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        write_config(&a, "clipboard_clear_secs = \"45\"\n");
        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, 45);
    }

    #[test]
    fn read_strips_inline_comments() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        write_config(
            &a,
            "clipboard_clear_secs = 90 # auto-clear delay\n\
             inbox_refresh_secs = 60 # ticks\n",
        );
        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, 90);
        assert_eq!(cfg.inbox_refresh_secs, 60);
    }

    #[test]
    fn read_skips_comment_lines() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        write_config(
            &a,
            "# this is a comment\n\
             clipboard_clear_secs = 50\n\
             # another\n",
        );
        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, 50);
    }

    #[test]
    fn read_tolerates_keys_without_spaces_around_equals() {
        // The old strip_prefix-based parser hard-coded `"key = "`
        // and silently dropped `key=value` (valid TOML).
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        write_config(&a, "clipboard_clear_secs=15\n");
        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, 15);
    }

    // ── Helpers ─────────────────────────────────────────────────

    #[test]
    fn unquote_strips_balanced_pair() {
        assert_eq!(unquote("\"foo\""), "foo");
        assert_eq!(unquote("foo"), "foo");
        assert_eq!(unquote("\"\""), "");
        // Unbalanced — leave alone.
        assert_eq!(unquote("\"foo"), "\"foo");
        assert_eq!(unquote("foo\""), "foo\"");
    }

    // ── Atomic write (tmp + rename) ─────────────────────────────

    #[test]
    fn write_leaves_no_tmp_file_after_success() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.write_auto_mark_read(false);
        // The sibling `.tmp` must be gone — rename swapped it in.
        let tmp_sidecar = a.file().with_extension("toml.tmp");
        // file().with_extension(".tmp") replaces .toml with .tmp,
        // but our naming pushes `.tmp` AFTER the extension. Build
        // the expected path explicitly.
        let expected_tmp = {
            let mut s = a.file().into_os_string();
            s.push(".tmp");
            std::path::PathBuf::from(s)
        };
        assert!(
            !expected_tmp.exists(),
            "tmp file lingered: {expected_tmp:?}"
        );
        // Also no stray sibling under a different naming guess.
        assert!(!tmp_sidecar.exists());
    }

    #[test]
    fn read_tightens_pre_existing_loose_file_perms() {
        // Pre-existing config.toml with 0644 (group/other readable)
        // must be tightened to 0600 the first time the TUI reads
        // it — without having to wait for the next write.
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        fs::write(a.file(), "clipboard_clear_secs = 20\n").unwrap();
        fs::set_permissions(a.file(), fs::Permissions::from_mode(0o644)).unwrap();
        // Sanity: we did set the loose perms.
        let before = fs::metadata(a.file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(before, 0o644);

        let cfg = a.read();
        assert_eq!(cfg.clipboard_clear_secs, 20);

        let after = fs::metadata(a.file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(after, 0o600, "read() must tighten loose file perms");
    }

    #[test]
    fn read_tightens_pre_existing_loose_dir_perms() {
        // Same property for the config DIRECTORY. ensure_dir is
        // called from read(), so the unconditional set_permissions
        // inside it should bring a loose dir back to 0700.
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        // Loosen the dir out of band.
        fs::set_permissions(&a.dir, fs::Permissions::from_mode(0o755)).unwrap();
        let before = fs::metadata(&a.dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(before, 0o755);

        let _ = a.read();
        let after = fs::metadata(&a.dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(after, 0o700, "ensure_dir must tighten loose dir perms");
    }

    #[test]
    fn write_preserves_owner_only_perms() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.write_clipboard_clear_secs(60);
        let mode = fs::metadata(a.file()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config.toml must be owner-only");
    }

    #[test]
    fn write_replaces_pre_existing_contents_completely() {
        // The pre-fix code truncated then rewrote, so a crash mid-
        // write could leave the file shorter than the new content
        // OR shorter than the old content. The atomic variant must
        // produce exactly the new content with no leftover bytes
        // from the previous version.
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        // Seed with a long previous version.
        fs::write(
            a.file(),
            "auto_mark_read = false\n\
             clipboard_clear_secs = 999\n\
             list_inbox_timeout_secs = 999\n\
             download_timeout_secs = 999\n\
             inbox_refresh_secs = 999\n\
             [theme]\naccent = \"#abcdef\"\n",
        )
        .unwrap();
        // Overwrite a single key — the rest must round-trip.
        a.write_clipboard_clear_secs(15);
        let body = fs::read_to_string(a.file()).unwrap();
        assert!(body.contains("clipboard_clear_secs = 15"));
        assert!(body.contains("auto_mark_read = false"));
        assert!(body.contains("[theme]"));
        assert!(body.contains("accent = \"#abcdef\""));
        // No leftover bytes from a truncated mid-write — the file
        // should end with a newline and nothing weird.
        assert!(body.ends_with('\n'), "got: {body:?}");
    }

    #[test]
    fn strip_inline_comment_respects_quoted_hashes() {
        assert_eq!(strip_inline_comment("42 # secs"), "42");
        assert_eq!(strip_inline_comment("42#nope"), "42");
        // `#` inside double-quoted strings stays.
        assert_eq!(strip_inline_comment("\"#abcdef\" # color"), "\"#abcdef\"");
        // No comment — returns input.
        assert_eq!(strip_inline_comment("no hash here"), "no hash here");
    }

    #[test]
    fn write_theme_name_inserts_into_section_preserving_overrides() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        fs::write(
            a.file(),
            "auto_mark_read = true\n[theme]\naccent = \"#ff0000\"\n",
        )
        .unwrap();
        a.write_theme_name("dracula");
        let out = fs::read_to_string(a.file()).unwrap();
        assert!(out.contains("name = \"dracula\""));
        assert!(out.contains("accent = \"#ff0000\""));
        assert!(out.contains("auto_mark_read = true"));
        let theme_at = out.find("[theme]").unwrap();
        let name_at = out.find("name = \"dracula\"").unwrap();
        assert!(name_at > theme_at, "name must sit inside [theme]");
        // A second write replaces, not duplicates.
        a.write_theme_name("nord");
        let out2 = fs::read_to_string(a.file()).unwrap();
        assert_eq!(out2.matches("name = ").count(), 1);
        assert!(out2.contains("name = \"nord\""));
    }

    #[test]
    fn write_theme_name_creates_section_when_absent() {
        let tmp = TempDir::new().unwrap();
        let a = adapter_in(&tmp);
        a.ensure_dir();
        fs::write(a.file(), "auto_mark_read = true\n").unwrap();
        a.write_theme_name("nord");
        let out = fs::read_to_string(a.file()).unwrap();
        assert!(out.contains("[theme]"));
        assert!(out.contains("name = \"nord\""));
        assert!(out.contains("auto_mark_read = true"));
    }
}
