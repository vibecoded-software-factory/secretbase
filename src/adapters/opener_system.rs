//! [`crate::ports::OpenerPort`] implementation that hands a URL to the OS's
//! default handler.
//!
//! Backend selection:
//! 1. macOS → `open`.
//! 2. Windows → `cmd /c start`.
//! 3. Otherwise (Linux/BSD) → `xdg-open`.
//!
//! The child is spawned detached (we don't wait on it) so the TUI never
//! blocks on the browser launch.

use std::process::{Command, Stdio};

use crate::ports::OpenerPort;

/// Default opener adapter — resolves the launcher at call time.
#[derive(Debug, Default)]
pub struct SystemOpener;

impl SystemOpener {
    /// Constructs a new adapter (cheap — selection happens at call time).
    pub fn new() -> Self {
        Self
    }
}

impl OpenerPort for SystemOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        // Only open web URLs we recognise — never pass arbitrary message text
        // to a shell launcher.
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("not an http(s) URL".to_string());
        }
        let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
            ("open", vec![url])
        } else if cfg!(target_os = "windows") {
            ("cmd", vec!["/c", "start", "", url])
        } else {
            ("xdg-open", vec![url])
        };
        Command::new(cmd)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_child| ())
            .map_err(|e| format!("{cmd}: {e}"))
    }
}
