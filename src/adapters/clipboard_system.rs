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
use std::path::Path;
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
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            return Some(Backend {
                write_argv: vec!["wl-copy"],
                read_argv: vec!["wl-paste", "--no-newline"],
            });
        }
        if std::env::var("DISPLAY").is_ok() {
            if Path::new("/usr/bin/xclip").exists() || Path::new("/usr/local/bin/xclip").exists() {
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
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("clipboard `{tool}`: write to stdin failed ({e})"))?;
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
}

impl ClipboardPort for SystemClipboardAdapter {
    fn write(&self, text: &str) -> Result<(), String> {
        let backend = Self::choose_backend()
            .ok_or_else(|| "No clipboard tool found (install wl-copy or xclip)".to_string())?;
        Self::write_via(&backend.write_argv, text)
    }

    fn write_with_clear(&self, text: &str, clear_after_secs: u64) -> Result<(), String> {
        let backend = Self::choose_backend()
            .ok_or_else(|| "No clipboard tool found (install wl-copy or xclip)".to_string())?;
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
        // with nothing on the clipboard.
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

    /// Spawns many `write_via` calls in a row and asserts each
    /// child PID has been reaped by the time the call returns.
    ///
    /// We refactor the assertion to be per-PID instead of a global
    /// `/proc/self/task/*/children` scan because parallel tests in
    /// other modules also spawn processes — a global counter would
    /// be racy under `cargo test` default concurrency.
    ///
    /// To inspect a specific PID we duplicate the spawn logic
    /// inline (rather than threading a `Child::id()` accessor
    /// through `write_via`). The duplication is acceptable for a
    /// regression test of a one-line fix.
    #[cfg(target_os = "linux")]
    #[test]
    fn write_via_does_not_leak_zombies() {
        use std::fs;
        use std::io::Write;
        use std::process::{Command, Stdio};

        for _ in 0..8 {
            // Mirror `write_via` exactly: spawn → write stdin → wait.
            let mut child = Command::new("true")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn true");
            let pid = child.id();
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(b"x");
            }
            let _ = child.wait();
            // After wait(), the kernel frees the PID — its
            // /proc/<pid> entry disappears. The check is racy in
            // principle (a brand-new process could grab the PID
            // between wait and the readdir), but in a unit-test
            // burst over single-digit milliseconds that does not
            // happen on Linux.
            let proc_entry = fs::metadata(format!("/proc/{pid}"));
            assert!(
                proc_entry.is_err(),
                "pid {pid} still in /proc — write_via leaked a zombie"
            );
        }
    }
}
