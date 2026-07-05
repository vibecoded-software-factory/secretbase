//! [`crate::ports::ClipboardPort`] implementation that shells out to the
//! native clipboard tool of the running session.
//!
//! Backend selection priority:
//! 1. Wayland (`$WAYLAND_DISPLAY` set) → `wl-copy` / `wl-paste`.
//! 2. X11 (`$DISPLAY` set) → `xclip`, falling back to `xsel`.
//! 3. macOS → `pbcopy` / `pbpaste`.
//!
//! When none of the above can be detected the call returns an error
//! describing the missing tool.

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::ports::ClipboardPort;

/// Default clipboard adapter — picks the right tool at call time and
/// pipes the payload into it via stdin (the payload never appears on a
/// command line, so it stays out of `ps`).
#[derive(Debug, Default)]
pub struct SystemClipboardAdapter;

/// Shape of a clipboard backend pair. Read and write tools are
/// independent so we can mix `wl-copy` with `wl-paste`, `xclip -i` with
/// `xclip -o`, etc.
struct Backend {
    write_argv: Vec<&'static str>,
    read_argv: Vec<&'static str>,
}

impl SystemClipboardAdapter {
    /// Constructs a new adapter. Cheap — selection happens at call-time.
    pub fn new() -> Self {
        Self
    }

    /// Picks the clipboard read+write commands for the current session.
    ///
    /// Returns `None` when no backend is detectable so the caller can
    /// surface a clear error to the user instead of guessing.
    fn choose_backend() -> Option<Backend> {
        /// Whether `name` resolves to an executable anywhere on `PATH` —
        /// not just the two historical hardcoded locations, so a Nix /
        /// ~/.local install is found before falling through to `xsel`
        /// (which may not be installed at all).
        fn binary_on_path(name: &str) -> bool {
            let Some(paths) = std::env::var_os("PATH") else {
                return false;
            };
            std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
        }

        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return Some(Backend {
                write_argv: vec!["wl-copy"],
                read_argv: vec!["wl-paste", "--no-newline"],
            });
        }
        if std::env::var("DISPLAY").is_ok() {
            if binary_on_path("xclip") {
                return Some(Backend {
                    write_argv: vec!["xclip", "-selection", "clipboard"],
                    read_argv: vec!["xclip", "-selection", "clipboard", "-o"],
                });
            }
            return Some(Backend {
                write_argv: vec!["xsel", "--clipboard", "--input"],
                read_argv: vec!["xsel", "--clipboard", "--output"],
            });
        }
        if cfg!(target_os = "macos") {
            return Some(Backend {
                write_argv: vec!["pbcopy"],
                read_argv: vec!["pbpaste"],
            });
        }
        None
    }

    /// Pipes `text` into the configured write tool via stdin and
    /// surfaces real failures up the call stack.
    ///
    /// What we used to swallow (and why each was wrong):
    ///
    /// * `write_all` errors — a broken pipe meant the backend
    ///   exited early; user saw "Label copied" but nothing was on
    ///   the clipboard.
    /// * `child.wait()` exit status — `wl-copy` (and others) exit
    ///   non-zero when they cannot reach the display server; same
    ///   false-positive.
    ///
    /// We deliberately route stderr to `/dev/null` and avoid
    /// `wait_with_output`. The reason is subtle: every backend in
    /// our set (`wl-copy`, `xclip`, `xsel`) **daemonises** —
    /// `fork()`s, the parent exits, the child keeps the clipboard
    /// payload in memory until ownership is lost. The daemon
    /// inherits the parent's stderr file descriptor; if that
    /// descriptor is a pipe held by us, `read_to_end` on the
    /// pipe never sees EOF (the daemon keeps the write-end open
    /// for its full lifetime) and `wait_with_output` hangs.
    /// Pointing stderr at `/dev/null` makes the inherited FD
    /// harmless, at the cost of losing the backend's stderr text
    /// on failure — we still return the exit status, which is
    /// enough for the user to know the copy failed.
    fn write_via(argv: &[&str], text: &str) -> Result<(), String> {
        let tool = argv[0];
        let mut cmd = Command::new(tool);
        for a in &argv[1..] {
            cmd.arg(a);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("clipboard `{tool}`: spawn failed ({e})"))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A `BrokenPipe` here is NOT conclusive: a backend that read
            // what it wanted and exited (or forked its daemon) closes its
            // read end before our write finishes — the exit status below is
            // the real verdict. Any other write error is fatal as before.
            if let Err(e) = stdin.write_all(text.as_bytes())
                && e.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(format!("clipboard `{tool}`: write to stdin failed ({e})"));
            }
            // Drop `stdin` here so the child sees EOF on its read
            // side and is free to fork its daemon / exit.
        }
        let status = child
            .wait()
            .map_err(|e| format!("clipboard `{tool}`: wait failed ({e})"))?;
        if !status.success() {
            return Err(format!("clipboard `{tool}`: exited with {status}"));
        }
        Ok(())
    }

    /// Reads the current clipboard contents through the configured read
    /// tool. Returns `None` when the tool fails to spawn or exits with
    /// an error — we treat both as "couldn't read", which makes the
    /// caller skip the clear (safer than blindly clobbering whatever
    /// the user has).
    fn read_via(argv: &[&str]) -> Option<String> {
        let out = Command::new(argv[0])
            .args(&argv[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Copies `text` via the OSC 52 terminal escape — the headless / SSH
    /// fallback used when no `wl-copy`/`xclip`/`xsel`/`pbcopy` backend is
    /// available. The **terminal** applies it (not a display server), so the
    /// copy lands in the *local* terminal's clipboard over SSH/tmux with no X /
    /// Wayland. Written to stdout on the render thread — sequential with
    /// ratatui's own output, so it can't interleave mid-frame. Terminal support
    /// can't be detected (most modern terminals support it; tmux needs
    /// `set-clipboard on`), so a terminal that ignores the escape is a silent
    /// no-op rather than an error.
    fn write_osc52(text: &str) -> Result<(), String> {
        let seq = Zeroizing::new(osc52_sequence(text));
        let mut out = std::io::stdout().lock();
        out.write_all(seq.as_bytes())
            .and_then(|()| out.flush())
            .map_err(|e| format!("clipboard OSC 52: {e}"))
    }
}

/// Base64-encodes `bytes` (standard alphabet, `=` padding). A tiny hand-rolled
/// encoder so the adapter needs no base64 crate for its one use — the OSC 52
/// clipboard escape.
fn base64_encode(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(A[(b0 >> 2) as usize] as char);
        out.push(A[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            A[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[(b2 & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The OSC 52 "set clipboard" escape for `text`: `ESC ] 52 ; c ; <base64> BEL`.
/// `c` selects the clipboard (vs the primary selection); an empty `text` yields
/// a well-formed clearing escape.
fn osc52_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()))
}

impl ClipboardPort for SystemClipboardAdapter {
    fn write(&self, text: &str) -> Result<(), String> {
        match Self::choose_backend() {
            Some(backend) => Self::write_via(&backend.write_argv, text),
            // No display / tool (the headless / SSH case) → OSC 52, which copies
            // via the terminal itself with no display server.
            None => Self::write_osc52(text),
        }
    }

    fn write_with_clear(&self, text: &str, clear_after_secs: u64) -> Result<(), String> {
        let backend = match Self::choose_backend() {
            Some(b) => b,
            // Headless / SSH: OSC 52 has no reliable read-back for the
            // compare-and-clear, and a timed write from a background thread would
            // race the render thread's stdout, so we just copy (no auto-clear).
            None => return Self::write_osc52(text),
        };
        Self::write_via(&backend.write_argv, text)?;

        if clear_after_secs == 0 {
            return Ok(());
        }

        // The payload is wrapped in `Zeroizing` so the heap copy that
        // lives inside the spawned thread is overwritten with zeroes
        // when the thread exits — closes the window where the secret
        // would otherwise sit unscrubbed waiting for the timer to fire.
        let payload = Zeroizing::new(text.to_string());
        let write_argv = backend.write_argv.clone();
        let read_argv = backend.read_argv.clone();

        // Detached background thread. If secretbase exits before the
        // timer fires the thread is killed with the process and the
        // clipboard is left as-is.
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(clear_after_secs));
            // Compare-and-clear: only wipe the clipboard if it still
            // holds the secret we wrote. Anything else means the user
            // moved on and we'd be stomping on their selection.
            let Some(current) = Self::read_via(&read_argv) else {
                return;
            };
            if current.as_str() == payload.as_str() {
                let _ = Self::write_via(&write_argv, "");
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encode_matches_known_vectors() {
        // RFC 4648 test vectors — the padding boundaries are what matter.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode("hi".as_bytes()), "aGk=");
    }

    #[test]
    fn osc52_sequence_wraps_the_base64_payload() {
        // ESC ] 52 ; c ; <base64> BEL
        assert_eq!(osc52_sequence("hi"), "\x1b]52;c;aGk=\x07");
        // Empty text is a well-formed *clearing* escape (no panic, no payload).
        assert_eq!(osc52_sequence(""), "\x1b]52;c;\x07");
    }

    #[test]
    fn write_with_clear_zero_disables_timer() {
        if SystemClipboardAdapter::choose_backend().is_none() {
            return;
        }
        let a = SystemClipboardAdapter::new();
        let started = std::time::Instant::now();
        let _ = a.write_with_clear("ignored", 0);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "write_with_clear(0) should not block on a timer"
        );
    }

    #[test]
    fn write_via_returns_quickly_for_fast_command() {
        // `true` accepts stdin, ignores it, and exits 0 immediately
        // — a portable stand-in for a healthy clipboard backend.
        // We're verifying the post-EOF wait does not hang.
        let started = std::time::Instant::now();
        SystemClipboardAdapter::write_via(&["true"], "ignored")
            .expect("write_via succeeds for `true`");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "write_via should not block — wait() of `true` is instant"
        );
    }

    #[test]
    fn write_via_returns_err_when_backend_exits_nonzero() {
        // `false` exits 1 — the previous code would have happily
        // returned Ok(()) and the user would have seen "copied"
        // with nothing on the clipboard. Reading the exit status also
        // proves `write_via` `wait()`s on (reaps) its child — so it can't
        // leak a zombie. That's why there's no separate /proc-scanning
        // reaping test: such a test was PID-reuse racy (it flaked under
        // parallel `cargo test`) and reimplemented the logic instead of
        // calling it, while this deterministic test covers the property.
        let err = SystemClipboardAdapter::write_via(&["false"], "x")
            .expect_err("write_via must surface non-zero exit");
        assert!(
            err.starts_with("clipboard `false`:"),
            "error should be prefixed with the backend name: {err}"
        );
    }

    #[test]
    fn write_via_surfaces_spawn_failure_with_tool_name() {
        // A non-existent binary surfaces the spawn error AND the
        // tool name (so the user knows which clipboard helper to
        // install).
        let err =
            SystemClipboardAdapter::write_via(&["definitely-not-a-real-clipboard-tool-9999"], "x")
                .expect_err("spawn of non-existent binary must Err");
        assert!(
            err.contains("definitely-not-a-real-clipboard-tool-9999"),
            "got: {err}"
        );
        assert!(err.contains("spawn"), "got: {err}");
    }
}
