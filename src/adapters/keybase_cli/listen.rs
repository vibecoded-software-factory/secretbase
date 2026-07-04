//! Long-lived `keybase chat api-listen` subprocess.
//!
//! `keybase chat api-listen` streams chat notifications as one JSON
//! object per line for as long as it runs. We spawn it once, drain its
//! stdout on a dedicated thread, parse each line into a [`ChatEvent`],
//! and hand the events to the TUI over a channel — so the inbox and the
//! open conversation update in real time instead of being re-fetched on
//! a timer.
//!
//! This is a **push** stream, distinct from the request/response
//! [`super::session::ApiSession`]: it has no stdin protocol, it only
//! emits. If the process dies (logged out, service restart, crash) a
//! **supervisor thread respawns it with backoff** — real-time updates
//! recover on their own instead of silently stopping for the rest of
//! the session (the periodic inbox resync remains the safety net for
//! anything missed while the stream was down, and a
//! [`ChatEvent::StreamClosed`] tells the UI about the interruption).

use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use zeroize::Zeroize;

use crate::domain::ChatEvent;

use super::parse_message;

/// Maximum respawn backoff. Doubling from 1 s and capping here keeps a
/// logged-out session (where `api-listen` exits immediately) from busy
/// re-spawning, while a post-login retry lands within half a minute.
const RESPAWN_BACKOFF_MAX_SECS: u64 = 30;

/// A child that survived this long counts as a healthy run — the next
/// failure starts the backoff from scratch instead of continuing it.
const HEALTHY_RUN_SECS: u64 = 60;

/// Owns the listener supervisor + the channel its reader feeds. Dropping
/// it stops the supervisor and kills the current child, so there are no
/// orphaned `keybase` processes.
pub struct ChatListener {
    /// Tells the supervisor to stop instead of respawning.
    shutdown: Arc<AtomicBool>,
    /// The currently-running child, shared with the supervisor: killing
    /// it from Drop unblocks the supervisor's blocking read.
    child: Arc<Mutex<Option<Child>>>,
    /// Receive half — parsed events. Moved out once with [`Self::take_rx`]
    /// into `App`; the guard itself stays alive in the run scope so the
    /// supervisor is stopped on exit (Drop).
    rx: Option<Receiver<ChatEvent>>,
}

impl ChatListener {
    /// Hands the event receiver to the caller (the TUI drains it each
    /// frame). Can only be taken once.
    pub fn take_rx(&mut self) -> Option<Receiver<ChatEvent>> {
        self.rx.take()
    }
}

impl Drop for ChatListener {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.child.lock()
            && let Some(mut child) = slot.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Spawns one `keybase chat api-listen` child. `--convs` also reports
/// new/joined conversations; `--hide-exploding` skips ephemeral messages.
fn spawn_child() -> std::io::Result<Child> {
    Command::new("keybase")
        .args(["chat", "api-listen", "--convs", "--hide-exploding"])
        .stdin(Stdio::null())
        // Notices ("Listening for chat notifications…") go to stderr;
        // null it so only JSON reaches us and a chatty service can't
        // wedge a full pipe.
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
}

/// Spawns the listener and its supervisor thread. The first child is
/// spawned synchronously so a missing `keybase` binary still surfaces as
/// an `Err` to the caller; after that the supervisor owns the lifecycle,
/// respawning the stream with backoff whenever it ends.
pub fn spawn_chat_listener() -> std::io::Result<ChatListener> {
    let child = spawn_child()?;
    let slot = Arc::new(Mutex::new(Some(child)));
    let shutdown = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel::<ChatEvent>();
    {
        let slot = Arc::clone(&slot);
        let shutdown = Arc::clone(&shutdown);
        std::thread::spawn(move || supervise(slot, shutdown, tx));
    }
    Ok(ChatListener {
        shutdown,
        child: slot,
        rx: Some(rx),
    })
}

/// The supervisor: reads the current child until its stream ends, then —
/// unless shutting down or the receiver is gone — reports the outage,
/// waits out the backoff and respawns. A run longer than
/// [`HEALTHY_RUN_SECS`] resets the backoff.
fn supervise(slot: Arc<Mutex<Option<Child>>>, shutdown: Arc<AtomicBool>, tx: Sender<ChatEvent>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let stdout = slot
            .lock()
            .ok()
            .and_then(|mut s| s.as_mut().and_then(|c| c.stdout.take()));
        if let Some(stdout) = stdout {
            let started = Instant::now();
            reader_loop(stdout, &tx);
            // Reap the finished child (or the one Drop already killed).
            if let Ok(mut s) = slot.lock()
                && let Some(mut c) = s.take()
            {
                let _ = c.kill();
                let _ = c.wait();
            }
            if started.elapsed() >= Duration::from_secs(HEALTHY_RUN_SECS) {
                backoff = Duration::from_secs(1);
            }
        }
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        // Tell the UI real-time updates were interrupted (send failure ⇒
        // the app is gone — stop).
        if tx.send(ChatEvent::StreamClosed).is_err() {
            return;
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(RESPAWN_BACKOFF_MAX_SECS));
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        // A spawn failure (keybase missing / transient) just loops and
        // retries with backoff.
        if let Ok(child) = spawn_child()
            && let Ok(mut s) = slot.lock()
        {
            *s = Some(child);
        }
    }
}

/// Hard cap on one buffered listener line. A healthy `api-listen` emits one
/// compact JSON object per line, and Keybase's own message-size limits keep
/// real events far below this — a line that exceeds it (misbehaving service,
/// hostile payload) is discarded instead of being allocated whole. Same
/// bounded-buffer discipline as the persistent session's `READER_BUF_CAP`,
/// except one bad line must not end the push stream, so it's dropped and
/// listening continues.
const LINE_CAP: usize = 8 * 1024 * 1024;

/// Outcome of one capped line read.
enum LineRead {
    /// `buf` holds a complete line (delimiter stripped).
    Line,
    /// The line exceeded the cap; it was drained and discarded.
    Oversized,
    /// Stream over (EOF or read error).
    Eof,
}

/// Drains stdout line-by-line, forwarding every parseable event. Exits
/// on EOF / read error (process gone) or once the receiver is dropped.
/// The line buffer is wiped between events (it holds chat plaintext).
fn reader_loop(stdout: ChildStdout, tx: &Sender<ChatEvent>) {
    let mut reader = BufReader::new(stdout);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.zeroize();
        buf.clear();
        match read_line_capped(&mut reader, &mut buf, LINE_CAP) {
            LineRead::Line => {
                let line = String::from_utf8_lossy(&buf);
                if let Some(ev) = parse_listen_event(&line)
                    && tx.send(ev).is_err()
                {
                    break; // receiver dropped (app exiting)
                }
            }
            LineRead::Oversized => {} // dropped; keep listening
            LineRead::Eof => break,   // process gone
        }
    }
    buf.zeroize();
}

/// Reads one `\n`-terminated line into `buf`, refusing to buffer more than
/// `cap` bytes. An over-long line is wiped, drained to its newline in
/// bounded chunks (never held whole), and reported as [`LineRead::Oversized`].
fn read_line_capped<R: BufRead>(r: &mut R, buf: &mut Vec<u8>, cap: usize) -> LineRead {
    loop {
        // `+ 1` so "exactly at the cap" is distinguishable from "ran out
        // of budget mid-line".
        let budget = (cap + 1).saturating_sub(buf.len()) as u64;
        match r.by_ref().take(budget).read_until(b'\n', buf) {
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return LineRead::Eof,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                    return LineRead::Line;
                }
                if buf.len() > cap {
                    buf.zeroize();
                    buf.clear();
                    return if drain_to_newline(r) {
                        LineRead::Oversized
                    } else {
                        LineRead::Eof
                    };
                }
                // No delimiter and under budget → true EOF; a non-empty
                // remainder is still a (final) line.
                return if buf.is_empty() {
                    LineRead::Eof
                } else {
                    LineRead::Line
                };
            }
        }
    }
}

/// Skips bytes up to and including the next `\n`, holding only the
/// `BufReader`'s own bounded buffer at a time — never the whole line.
/// Returns `false` when the stream ends first.
fn drain_to_newline<R: BufRead>(r: &mut R) -> bool {
    loop {
        let (found, used) = match r.fill_buf() {
            Ok([]) => return false,
            Ok(avail) => match avail.iter().position(|&b| b == b'\n') {
                Some(i) => (true, i + 1),
                None => (false, avail.len()),
            },
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        r.consume(used);
        if found {
            return true;
        }
    }
}

/// Parses one `api-listen` JSON line into a [`ChatEvent`], or `None` for
/// events we don't act on (typing, identify, etc.) and malformed lines.
///
/// Shapes (verified against `keybase/client`
/// `go/client/chat_api_listen_display.go`):
/// - message: `{"type":"chat","source":"remote","msg":{<MsgSummary>}}`
/// - new conv: `{"type":"chat_conv","conv":{<ConvSummary>}}`
pub(crate) fn parse_listen_event(line: &str) -> Option<ChatEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type").and_then(Value::as_str)? {
        "chat" => {
            let msg = v.get("msg")?;
            let conv_id = msg
                .get("conversation_id")
                .and_then(Value::as_str)?
                .to_string();
            let message = parse_message(msg)?;
            Some(ChatEvent::Message { conv_id, message })
        }
        "chat_conv" => Some(ChatEvent::NewConversation),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::MessageContent;

    #[test]
    fn parses_incoming_text_message() {
        let line = r#"{"type":"chat","source":"remote","msg":{
            "id":42,"conversation_id":"abc123",
            "channel":{"name":"alice,bob","members_type":"impteamnative","topic_type":"chat"},
            "sender":{"username":"alice"},
            "sent_at":1700000000,"sent_at_ms":1700000000000,
            "content":{"type":"text","text":{"body":"hello"}}
        }}"#;
        let ev = parse_listen_event(line).expect("event");
        match ev {
            ChatEvent::Message { conv_id, message } => {
                assert_eq!(conv_id, "abc123");
                assert_eq!(message.id, 42);
                assert_eq!(message.sender, "alice");
                assert!(
                    matches!(&message.content, MessageContent::Text(t) if t.as_str() == "hello")
                );
            }
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn parses_new_conversation() {
        let line = r#"{"type":"chat_conv","conv":{"id":"zzz"}}"#;
        assert!(matches!(
            parse_listen_event(line),
            Some(ChatEvent::NewConversation)
        ));
    }

    #[test]
    fn ignores_unknown_and_malformed() {
        assert!(parse_listen_event(r#"{"type":"typing"}"#).is_none());
        assert!(parse_listen_event("not json").is_none());
        assert!(parse_listen_event("").is_none());
        // chat event without a conversation_id is dropped.
        assert!(parse_listen_event(r#"{"type":"chat","msg":{"id":1}}"#).is_none());
    }

    #[test]
    fn capped_reader_returns_normal_lines() {
        let mut r = std::io::BufReader::new(std::io::Cursor::new(b"hello\nworld".to_vec()));
        let mut buf = Vec::new();
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64),
            LineRead::Line
        ));
        assert_eq!(buf, b"hello");
        buf.clear();
        // Final line without a trailing newline still comes through.
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64),
            LineRead::Line
        ));
        assert_eq!(buf, b"world");
        buf.clear();
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64),
            LineRead::Eof
        ));
    }

    #[test]
    fn capped_reader_drops_oversized_line_and_keeps_listening() {
        let mut data = vec![b'x'; 200]; // one 200-byte line, cap of 64
        data.push(b'\n');
        data.extend_from_slice(b"next\n");
        let mut r = std::io::BufReader::new(std::io::Cursor::new(data));
        let mut buf = Vec::new();
        // The oversized line is discarded (never buffered whole)…
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64),
            LineRead::Oversized
        ));
        assert!(buf.is_empty());
        // …and the following line is intact.
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64),
            LineRead::Line
        ));
        assert_eq!(buf, b"next");
    }

    #[test]
    fn capped_reader_line_exactly_at_cap_is_kept() {
        let mut data = vec![b'y'; 64];
        data.push(b'\n');
        let mut r = std::io::BufReader::new(std::io::Cursor::new(data));
        let mut buf = Vec::new();
        assert!(matches!(
            read_line_capped(&mut r, &mut buf, 64),
            LineRead::Line
        ));
        assert_eq!(buf.len(), 64);
    }
}
