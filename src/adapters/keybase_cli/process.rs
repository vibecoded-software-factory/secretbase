//! Thin wrappers around `std::process::Command` for `keybase`
//! invocations.

use std::io::{Read, Write};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::ports::KeybaseError;

/// Defense-in-depth timeout for *local-only* `keybase` invocations
/// (`status`, `chat api {"method":"list"}` against the local cache).
/// These should finish in milliseconds, but a wedged `keybased`
/// service — or a future bug that adds an unexpected hidden prompt —
/// would otherwise freeze the TUI indefinitely. 10 s is many orders of
/// magnitude above the expected runtime, so it never fires in
/// practice; it's purely a panic-prevention floor.
const LOCAL_OP_FALLBACK_TIMEOUT: u64 = 10;

/// Polls a spawned child until it exits or the wall-clock deadline is
/// reached. On timeout the child is killed and an error is returned.
///
/// ## Why two reader threads
///
/// stdout and stderr are drained concurrently in dedicated threads
/// **while** the child is still running, not after it exits. The
/// post-exit drain pattern (read_to_end after try_wait returns Some)
/// deadlocks on any output larger than the pipe buffer (typically
/// 64 KB on Linux): the child blocks on `write(2)` waiting for the
/// pipe to be drained, we block in `try_wait` waiting for the child
/// to exit, neither side makes progress and the only escape is the
/// timeout. A large Keybase inbox easily exceeds that threshold.
///
/// `std::process::Child::wait_with_output` does this internally; we
/// reimplement the same pattern here because we also need a wall-clock
/// deadline, which `wait_with_output` does not expose.
fn wait_with_timeout(mut child: Child, secs: u64, label: &str) -> Result<Output, KeybaseError> {
    let deadline = Instant::now() + Duration::from_secs(secs);

    // Adaptive poll backoff. A fixed interval is pure added latency on
    // the happy path: most `keybase` calls finish in single-digit
    // milliseconds, so a coarse interval makes *every* operation feel
    // that slow. We start tight (1 ms) so quick ops (mark-read, mute,
    // react, status, the local `list`) return almost immediately, then
    // back off to a relaxed cap so a genuinely slow call (a big network
    // `read`) doesn't busy-spin the CPU.
    const MIN_POLL: Duration = Duration::from_millis(1);
    const MAX_POLL: Duration = Duration::from_millis(20);
    let mut poll = MIN_POLL;

    let stdout_thread = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_thread = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });

    loop {
        match child
            .try_wait()
            .map_err(|e| KeybaseError::Spawn(format!("wait error: {e}")))?
        {
            Some(status) => {
                let stdout = stdout_thread
                    .and_then(|t| t.join().ok())
                    .unwrap_or_default();
                let stderr = stderr_thread
                    .and_then(|t| t.join().ok())
                    .unwrap_or_default();
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.and_then(|t| t.join().ok());
                    let _ = stderr_thread.and_then(|t| t.join().ok());
                    return Err(KeybaseError::Timeout {
                        label: label.to_string(),
                        secs,
                    });
                }
                std::thread::sleep(poll);
                poll = (poll * 2).min(MAX_POLL);
            }
        }
    }
}

/// Runs `keybase <args>` and returns the raw [`Output`] with the
/// default local-op timeout.
pub fn keybase_run(args: &[&str]) -> Result<Output, KeybaseError> {
    keybase_run_timeout(args, LOCAL_OP_FALLBACK_TIMEOUT)
}

/// Runs `keybase <args>` with a wall-clock timeout.
pub fn keybase_run_timeout(args: &[&str], secs: u64) -> Result<Output, KeybaseError> {
    let child = Command::new("keybase")
        .args(args)
        // Null out stdin explicitly. The parent process is a TUI in
        // raw mode — an inherited terminal fd would let any stray
        // keybase read steal the user's keystrokes (or block waiting
        // for them). Closing it at spawn time is the cheapest possible
        // defense.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| KeybaseError::Spawn(e.to_string()))?;
    wait_with_timeout(child, secs, "keybase")
}

/// Runs `keybase <args>` after writing `stdin_input` to the child's
/// stdin. Used by every JSON-API call: `keybase chat api` and
/// `keybase team api` read their request body from stdin.
pub fn keybase_run_with_stdin(args: &[&str], stdin_input: &str) -> Result<Output, KeybaseError> {
    keybase_run_with_stdin_timeout(args, stdin_input, LOCAL_OP_FALLBACK_TIMEOUT)
}

/// Like [`keybase_run_with_stdin`] but with a wall-clock timeout.
pub fn keybase_run_with_stdin_timeout(
    args: &[&str],
    stdin_input: &str,
    secs: u64,
) -> Result<Output, KeybaseError> {
    let mut child = Command::new("keybase")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| KeybaseError::Spawn(e.to_string()))?;

    if let Some(mut sin) = child.stdin.take() {
        // Errors writing to stdin (broken pipe if keybase exits early)
        // are intentionally ignored — the wait below will surface the
        // real failure via the child's exit status / stderr.
        let _ = sin.write_all(stdin_input.as_bytes());
        // Dropping `sin` here closes the write end of the pipe, which
        // signals EOF to keybase so it stops waiting for more input.
    }

    wait_with_timeout(child, secs, "keybase")
}

/// Returns the trimmed stdout of an [`Output`] as a [`String`].
pub fn stdout_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Returns the trimmed stderr of an [`Output`] as a [`String`].
pub fn stderr_str(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_sh(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sh")
    }

    #[test]
    fn timeout_returns_output_when_command_finishes_quickly() {
        let child = spawn_sh("printf hello; exit 0");
        let out = wait_with_timeout(child, 5, "test").expect("should not time out");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello");
    }

    #[test]
    fn timeout_returns_err_when_command_runs_too_long() {
        let child = spawn_sh("exec sleep 5");
        let started = Instant::now();
        let res = wait_with_timeout(child, 1, "test");
        let elapsed = started.elapsed();
        assert!(res.is_err());
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
    }

    #[test]
    fn timeout_drains_large_stdout_without_deadlocking() {
        let child = spawn_sh("exec head -c 262144 /dev/zero");
        let started = Instant::now();
        let res = wait_with_timeout(child, 5, "test").expect("must not time out");
        let elapsed = started.elapsed();
        assert!(res.status.success());
        assert_eq!(res.stdout.len(), 262144);
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
    }
}
