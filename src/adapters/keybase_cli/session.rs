//! Persistent `keybase <family> api` subprocess.
//!
//! `keybase chat api` / `keybase team api` read a *stream* of JSON
//! method calls from stdin and emit one JSON response per call. The
//! original adapter spawned a fresh `keybase` process for every
//! operation; the binary's fork+exec and its connection to the local
//! `keybased` service is the dominant per-operation cost once the call
//! itself hits the local cache. Keeping one long-lived process per API
//! family amortises that across every request.
//!
//! ## Safety / robustness
//!
//! The worker thread is the single, serial caller, so request↔response
//! ordering is guaranteed (write one line, read one response) — no
//! request IDs needed. A dedicated reader thread drains stdout and
//! ships each complete JSON object over a channel; `run` writes the
//! request then `recv_timeout`s the reply, so a wedged service still
//! honours the per-call wall-clock budget.
//!
//! Crucially, **any** persistent-mode failure (the process died, the
//! pipe broke, the stream protocol misbehaved) transparently falls back
//! to a one-shot `keybase <family> api` spawn — the proven path — for
//! that call, and after a few consecutive failures the persistent mode
//! disables itself for the rest of the process. Net effect: as fast as
//! streaming allows when it works, never less correct than one-shot
//! when it doesn't.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use zeroize::{Zeroize, Zeroizing};

use crate::adapters::keybase_cli::process::{
    keybase_run_with_stdin_timeout, stderr_str, stdout_str,
};
use crate::ports::KeybaseError;

/// Consecutive persistent-mode failures before we give up and route
/// everything through one-shot spawns for the rest of the process.
const MAX_FAILURES: u32 = 3;

/// Hard cap on the un-parsed reader buffer. A healthy `keybase api`
/// emits one compact JSON object per response; if the buffer grows past
/// this without parsing, the stream is misbehaving — bail to one-shot.
const READER_BUF_CAP: usize = 32 * 1024 * 1024;

/// Short probe budget for the *first ever* persistent call. If
/// `keybase api` doesn't actually stream (e.g. it waited for stdin EOF
/// before processing), the first request would otherwise hang for the
/// full per-call timeout. We cap the first wait so a misbehaving stream
/// degrades to one-shot in seconds, not tens of seconds. Once any call
/// has succeeded we know streaming works and use the full budget.
const PROBE_SECS: u64 = 4;

/// One item handed from the reader thread to [`ApiSession::run`].
enum ReadItem {
    /// A complete JSON response object (raw text).
    Json(String),
    /// stdout closed / errored — the process is gone.
    Closed,
}

/// The live half of a persistent session.
struct Live {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<ReadItem>,
}

impl Drop for Live {
    fn drop(&mut self) {
        // Closing stdin (dropped with the struct) signals EOF; kill is
        // belt-and-suspenders so the child can't linger after the TUI
        // exits or after we decide to respawn.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Persistent `keybase <family> api` session with one-shot fallback.
pub struct ApiSession {
    family: &'static str,
    live: Option<Live>,
    disabled: bool,
    failures: u32,
    /// Whether the persistent stream has ever returned a response. Until
    /// it has, a timeout is treated as "streaming doesn't work here"
    /// (fall back) rather than "this op is slow" (surface the timeout).
    had_success: bool,
}

/// Internal classification of a failed persistent-mode attempt. The
/// distinction that matters for correctness is **whether the request
/// was delivered to keybase** (and may therefore have executed):
///
/// * `PreWrite` — spawn or the stdin write failed, so keybase never got
///   the request. Always safe to retry via one-shot.
/// * `PostWrite*` — the request was written and flushed, but no usable
///   response came back. The operation *may* have already run, so a
///   blind one-shot retry would double-execute it. Only safe to retry
///   for idempotent (read-only) methods.
enum PersistentError {
    /// Failed before the request reached keybase — safe to fall back.
    PreWrite,
    /// Request delivered, response timed out.
    PostWriteTimeout,
    /// Request delivered, the stream closed without a response.
    PostWriteClosed,
}

impl ApiSession {
    /// Builds a lazy session — no process spawns until the first
    /// [`run`](Self::run).
    pub fn new(family: &'static str) -> Self {
        Self {
            family,
            live: None,
            disabled: false,
            failures: 0,
            had_success: false,
        }
    }

    /// Runs one API request and returns the raw JSON stdout (wrapped in
    /// [`Zeroizing`]). Tries the persistent process first.
    ///
    /// `idempotent` MUST be `false` for any state-changing call
    /// (send/react/edit/delete/…): after the request has been delivered
    /// to keybase, a one-shot fallback could re-execute it (a duplicate
    /// message, a toggled-off reaction). For such calls a delivered but
    /// unanswered request surfaces as an error instead of being retried.
    /// Read-only calls (`idempotent = true`) retry freely.
    pub fn run(
        &mut self,
        stdin_json: &str,
        timeout_secs: u64,
        idempotent: bool,
    ) -> Result<Zeroizing<String>, KeybaseError> {
        if !self.disabled {
            match self.run_persistent(stdin_json, timeout_secs) {
                Ok(body) => {
                    self.failures = 0;
                    self.had_success = true;
                    return Ok(body);
                }
                // Never reached keybase → always safe to fall back.
                Err(PersistentError::PreWrite) => {
                    self.live = None;
                    self.note_failure();
                    // fall through to one-shot
                }
                // Delivered but no response. Safe to retry ONLY if the
                // op is idempotent (also covers the first-call probe,
                // which is always a read). Otherwise surface an error so
                // we never double-execute a mutation.
                Err(PersistentError::PostWriteTimeout) => {
                    self.live = None;
                    if idempotent {
                        self.note_failure();
                        // fall through to one-shot
                    } else {
                        return Err(KeybaseError::Timeout {
                            label: format!("keybase {} api", self.family),
                            secs: timeout_secs,
                        });
                    }
                }
                Err(PersistentError::PostWriteClosed) => {
                    self.live = None;
                    if idempotent {
                        self.note_failure();
                        // fall through to one-shot
                    } else {
                        return Err(KeybaseError::Internal(format!(
                            "keybase {} api: response lost; the operation may have completed",
                            self.family
                        )));
                    }
                }
            }
        }
        self.run_oneshot(stdin_json, timeout_secs)
    }

    /// Counts a persistent-mode failure and disables the stream after
    /// too many in a row (e.g. `keybase api` doesn't stream here).
    fn note_failure(&mut self) {
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            self.disabled = true;
        }
    }

    fn run_persistent(
        &mut self,
        stdin_json: &str,
        timeout_secs: u64,
    ) -> Result<Zeroizing<String>, PersistentError> {
        if self.live.is_none() {
            self.live = Some(spawn(self.family).map_err(|_| PersistentError::PreWrite)?);
        }
        let live = self.live.as_mut().expect("just spawned");

        // Write the request as one line. The buffer carries the
        // plaintext body, so zero it after the write.
        let mut line = Zeroizing::new(String::with_capacity(stdin_json.len() + 1));
        line.push_str(stdin_json);
        line.push('\n');
        // A write/flush failure means the request never reached keybase.
        if live.stdin.write_all(line.as_bytes()).is_err() || live.stdin.flush().is_err() {
            return Err(PersistentError::PreWrite);
        }

        // First-ever call gets a short probe budget so a non-streaming
        // `keybase api` degrades to one-shot quickly instead of hanging
        // for the full timeout.
        let recv_secs = if self.had_success {
            timeout_secs
        } else {
            timeout_secs.min(PROBE_SECS)
        };
        match live.rx.recv_timeout(Duration::from_secs(recv_secs)) {
            Ok(ReadItem::Json(s)) => Ok(Zeroizing::new(s)),
            Ok(ReadItem::Closed) => Err(PersistentError::PostWriteClosed),
            Err(RecvTimeoutError::Timeout) => Err(PersistentError::PostWriteTimeout),
            Err(RecvTimeoutError::Disconnected) => Err(PersistentError::PostWriteClosed),
        }
    }

    fn run_oneshot(
        &self,
        stdin_json: &str,
        timeout_secs: u64,
    ) -> Result<Zeroizing<String>, KeybaseError> {
        let mut out =
            keybase_run_with_stdin_timeout(&[self.family, "api"], stdin_json, timeout_secs)?;
        if !out.status.success() {
            let stderr = stderr_str(&out);
            let status = out.status.code().unwrap_or(-1);
            out.stdout.zeroize();
            out.stderr.zeroize();
            return Err(KeybaseError::Exit { stderr, status });
        }
        let body = Zeroizing::new(stdout_str(&out));
        out.stdout.zeroize();
        Ok(body)
    }
}

/// Spawns `keybase <family> api` in streaming mode with piped stdin /
/// stdout and a reader thread draining stdout.
fn spawn(family: &str) -> std::io::Result<Live> {
    let mut child = Command::new("keybase")
        .args([family, "api"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Notices/warnings go to stderr; we only consume the JSON on
        // stdout. Null it so a chatty service can't fill a pipe and
        // wedge the child, and so stdout stays pure JSON.
        .stderr(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let (tx, rx) = channel::<ReadItem>();
    std::thread::spawn(move || reader_loop(stdout, tx));
    Ok(Live { child, stdin, rx })
}

/// Drains stdout, emitting one [`ReadItem::Json`] per complete JSON
/// object. Accumulates across lines so a (rare) pretty-printed reply is
/// still delivered as a single object. Exits on EOF, read error, a
/// runaway buffer, or once the receiver is gone.
fn reader_loop(stdout: ChildStdout, tx: Sender<ReadItem>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut acc = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                let _ = tx.send(ReadItem::Closed);
                return;
            }
            Ok(_) => {
                acc.push_str(&line);
                let trimmed = acc.trim();
                if !trimmed.is_empty() && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
                {
                    let item = ReadItem::Json(std::mem::take(&mut acc));
                    if tx.send(item).is_err() {
                        return; // receiver dropped (session respawned / app exiting)
                    }
                } else if acc.len() > READER_BUF_CAP {
                    let _ = tx.send(ReadItem::Closed);
                    return;
                }
            }
            Err(_) => {
                let _ = tx.send(ReadItem::Closed);
                return;
            }
        }
    }
}
