//! Optional debug logging to `~/.secretbase.log` — activated by
//! setting the `SECRETBASE_DEBUG=1` environment variable before
//! launching the TUI.
//!
//! Designed to be cheap when disabled (an `env::var` check + early
//! return), so leaving the call sites in place in release builds has
//! no measurable overhead.
//!
//! The file is created with mode `0600`. Entries may include
//! conversation labels (DM partner names, team channel names) which
//! are not credentials but are private — readable only by the
//! account that ran the TUI.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Owner-only mode for the log file.
const LOG_FILE_MODE: u32 = 0o600;

/// Returns `true` when `SECRETBASE_DEBUG=1` is set in the environment.
fn enabled() -> bool {
    matches!(std::env::var("SECRETBASE_DEBUG").as_deref(), Ok("1"))
}

/// Returns the absolute path of the log file (`$HOME/.secretbase.log`).
fn log_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".secretbase.log"))
}

/// Appends `line` to the debug log (prefixed with a unix-millis
/// timestamp). Silently swallows all errors so a logging hiccup never
/// breaks the TUI.
pub fn log(line: &str) {
    if !enabled() {
        return;
    }
    let Some(path) = log_path() else {
        return;
    };
    log_to(&path, line);
}

/// Append helper that does the actual file write. Separated so it
/// can be unit-tested without manipulating the process-global
/// `HOME` / `SECRETBASE_DEBUG` environment (which would require
/// `unsafe` in modern Rust and is forbidden by the crate's
/// `#![forbid(unsafe_code)]`).
fn log_to(path: &std::path::Path, line: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut f) = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(LOG_FILE_MODE)
        .open(path)
    {
        let _ = writeln!(f, "{ts} {line}");
        // Tighten perms on pre-existing files that were created
        // before the mode arg was added.
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(LOG_FILE_MODE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn log_to_appends_lines_with_unix_millis_prefix() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("debug.log");
        log_to(&path, "first entry");
        log_to(&path, "second entry");
        let body = fs::read_to_string(&path).expect("file written");
        // Two lines, each ending with the message.
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("first entry"), "got {body:?}");
        assert!(lines[1].ends_with("second entry"));
        // The leading token is a unix-millis timestamp (numeric).
        let ts_token = lines[0].split_whitespace().next().unwrap();
        assert!(
            ts_token.parse::<u128>().is_ok(),
            "leading token must be unix millis, got {ts_token:?}"
        );
    }

    #[test]
    fn log_to_creates_file_with_owner_only_perms() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("debug.log");
        log_to(&path, "x");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "log file must be owner-only");
    }

    #[test]
    fn log_to_tightens_perms_on_pre_existing_loose_file() {
        // Simulate a pre-existing file from before the mode-arg
        // was added (e.g. created by an older secretbase build).
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("debug.log");
        fs::write(&path, b"old line\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        log_to(&path, "new line");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "pre-existing loose file must be tightened");
        // The old line is preserved (append mode), the new one is
        // appended.
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("old line"));
        assert!(body.contains("new line"));
    }

    #[test]
    fn log_helper_no_ops_when_env_var_unset() {
        // `log` reads `SECRETBASE_DEBUG` from the process env. We
        // can't set or unset env vars without `unsafe` (forbidden
        // by the crate-level `#![forbid(unsafe_code)]`), so this
        // test only covers the "unset by default in the test
        // harness" branch — which is the most important guarantee:
        // production users without the opt-in are never written to.
        assert!(
            !enabled(),
            "tests must run with SECRETBASE_DEBUG unset; CI or local shell exporting it would invalidate this guarantee"
        );
        // Calling `log` here is a no-op — there's nothing to assert
        // beyond "did not panic". The path-resolution helper is
        // tested implicitly via `log_to` above.
        log("ignored");
    }
}
