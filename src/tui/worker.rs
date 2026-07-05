// The test module deliberately lives in the middle of the file
// (before `run_worker`) so the synthetic port stub it carries can
// be visually paired with the production loop it exercises. Clippy
// prefers tests last; we make the exception explicit.
#![allow(clippy::items_after_test_module)]

//! Worker thread that owns the [`KeybasePort`] and serves requests
//! serially.
//!
//! ## Why
//!
//! Every call into the `keybase` CLI is a synchronous subprocess that
//! can take up to the configured timeout (30 s for `list` / `read`).
//! Running it on the render thread freezes the TUI for the whole
//! duration — the spinner is static, mouse and keys queue up, even
//! Ctrl+C is delayed. To keep the UI live we move the port behind a
//! single long-lived worker thread:
//!
//! * The render thread sends a [`WorkerRequest`] and immediately
//!   continues redrawing. The spinner ticks while the request runs.
//! * The worker pulls one request at a time, calls into the port,
//!   ships a [`WorkerResponse`] back through the response channel.
//! * The render thread polls the channel between events; when a
//!   response arrives it dispatches to
//!   [`crate::tui::flows::apply_response`] which mutates `App`.
//!
//! Serial-by-construction: only one request is in flight at any time
//! (`App::in_flight` is `Option`, not `Vec`). That preserves the
//! semantics of the original port (`&mut self`) and matches the
//! existing single-shot interaction model.
//!
//! ## No tokio
//!
//! This is `std::thread` + `std::sync::mpsc`, deliberately. The
//! project rule (`CLAUDE.md`) forbids a background async runtime —
//! the worker thread costs ~80 KB of stack and zero runtime overhead.

use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use crate::domain::{ChatMember, Emoji, IdentityInfo, InboxHit, Message};
use crate::ports::keybase::{ListConversationsOk, ListTeamsOk, ReadChannel};
use crate::ports::{KeybaseError, KeybasePort};

/// Wraps a port call in `catch_unwind` so a panic inside the
/// adapter (or a third-party crate it pulls in) does not kill the
/// worker thread.
///
/// `&mut dyn KeybasePort` is not `UnwindSafe` by default — we
/// assert it via [`AssertUnwindSafe`]. The trade-off: if the panic
/// happened mid-mutation, the adapter could be left in a partial
/// state. In practice [`crate::adapters::keybase_cli::KeybaseCliAdapter`]
/// only stores `u64` timeout fields, so there is no mid-call state
/// to corrupt; the next request runs against a healthy receiver.
fn run_caught<T>(f: impl FnOnce() -> Result<T, KeybaseError>) -> Result<T, KeybaseError> {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => Err(KeybaseError::Internal(panic_payload_to_string(payload))),
    }
}

/// Best-effort extraction of a panic payload as a human-readable
/// string. The standard library panics with either `&'static str`
/// (from `panic!("literal")`) or `String` (from
/// `panic!("{var}")`); anything else is an exotic custom type and
/// we fall back to a placeholder.
fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<unknown panic payload>".to_string()
}

/// One unit of work for the worker. Each variant carries everything
/// the port call needs — parameters are captured at queue time so the
/// worker thread never has to look back into `App`.
pub enum WorkerRequest {
    Status,
    Logout,
    /// Non-interactive paper-key login. The paper key is secret material,
    /// held in a `Zeroizing` buffer so it is wiped when the request is
    /// consumed by the worker.
    LoginPaperkey {
        username: String,
        device: String,
        paperkey: zeroize::Zeroizing<String>,
    },
    ListConversations,
    /// Inbox list issued by the idle auto-refresh on the **background**
    /// lane. Identical to [`Self::ListConversations`] but its response
    /// is routed off the user's in-flight slot so it never gates input.
    ListConversationsSilent,
    /// Background mark-read fired when a pushed message lands in the
    /// conversation being viewed at the bottom — keeps the server-side read
    /// pointer (and the phone badge) in step without touching the user's
    /// in-flight slot. Routed by variant, like the silent list.
    MarkReadSilent {
        channel: ReadChannel,
        message_id: u64,
    },
    ReadMessages {
        channel: ReadChannel,
        num: u32,
        peek: bool,
        next_cursor: Option<String>,
    },
    MarkRead {
        channel: ReadChannel,
        message_id: u64,
    },
    SearchInboxHits {
        query: String,
        max_hits: u32,
    },
    /// GIF search against giphy with the **user's own API key** (no
    /// keybase call — the service's giphy proxy key isn't reachable over
    /// the JSON API). The key rides in the request and is dropped with it.
    GiphySearch {
        api_key: zeroize::Zeroizing<String>,
        query: String,
    },
    SearchRegexp {
        channel: ReadChannel,
        query: String,
        max_hits: u32,
    },
    SendMessage {
        channel: ReadChannel,
        body: String,
        reply_to: Option<u64>,
    },
    EditMessage {
        channel: ReadChannel,
        message_id: u64,
        body: String,
    },
    DeleteMessage {
        channel: ReadChannel,
        message_id: u64,
    },
    React {
        channel: ReadChannel,
        message_id: u64,
        body: String,
    },
    NewConversation {
        channel: ReadChannel,
    },
    /// List every channel of a team (`listconvsonname`) for the channel browser.
    LoadChannels {
        team: String,
    },
    JoinChannel {
        channel: ReadChannel,
    },
    LeaveChannel {
        channel: ReadChannel,
    },
    /// List the members of a conversation / channel (`listmembers`).
    LoadMembers {
        channel: ReadChannel,
    },
    AddToChannel {
        channel: ReadChannel,
        usernames: Vec<String>,
    },
    RemoveFromChannel {
        channel: ReadChannel,
        usernames: Vec<String>,
    },
    RenameChannel {
        team: String,
        old: String,
        new: String,
    },
    DeleteChannel {
        team: String,
        channel: String,
    },
    /// Get (empty `set`) or replace the team's default channels.
    DefaultChannels {
        team: String,
        set: Vec<String>,
    },
    SetConvStatus {
        channel: ReadChannel,
        status: String,
    },
    PinMessage {
        channel: ReadChannel,
        message_id: u64,
    },
    UnpinMessage {
        channel: ReadChannel,
    },
    /// Background fetch of a pinned message whose target is known (from the
    /// session-local pin record) but older than the loaded window — the 📌
    /// header needs its body. Routed by variant on the background lane,
    /// like [`Self::MarkReadSilent`].
    GetPinnedMessage {
        conv_id: String,
        channel: ReadChannel,
        message_id: u64,
    },
    DownloadAttachment {
        channel: ReadChannel,
        message_id: u64,
        output: String,
    },
    /// Background download of an image attachment to a cache path for inline
    /// rendering. Carries `message_id` so the response can be matched back to
    /// the right attachment (many run concurrently).
    PreviewImage {
        channel: ReadChannel,
        message_id: u64,
        output: String,
    },
    UploadAttachment {
        channel: ReadChannel,
        filename: String,
        title: String,
    },
    /// Background fetch of a **public web media** file (giphy GIF) to the
    /// image cache for inline rendering — the giphy path can't go through
    /// keybase (the encrypted re-host isn't reachable over the JSON API).
    /// Responds with [`WorkerResponse::PreviewImage`], so the ready/failed
    /// bookkeeping is shared with attachment previews. Sent on the
    /// background lane: a slow CDN must never queue ahead of user ops.
    FetchWebImage {
        url: String,
        output: String,
    },
    /// Decode an animated GIF's frames off the render thread (ImageMagick
    /// `convert`) so the UI never blocks while a large GIF is "generated".
    /// Not a keybase call — pure image work; carries the downloaded GIF's
    /// cache path, from which the worker derives the frame dir.
    DecodeGif {
        path: String,
    },
    ListEmojis,
    /// The local user's teams. Carries the self `username` because it queries
    /// `list-user-memberships` (one row per team) rather than
    /// `list-self-memberships` (one row per teammate — see the port doc).
    ListSelfMemberships {
        username: String,
    },
    /// Terminates the worker. Sent automatically on drop of
    /// [`WorkerHandle`].
    Shutdown,
}

/// Wraps the typed result of one [`WorkerRequest`]. The kind variant
/// matches the request 1:1 — the dispatcher in
/// [`crate::tui::flows::apply_response`] uses `App::in_flight` (not
/// the variant) to know what caller-side context to apply, so each
/// kind carries only the port's own output.
pub enum WorkerResponse {
    Status(Result<IdentityInfo, KeybaseError>),
    Logout(Result<(), KeybaseError>),
    LoginPaperkey(Result<(), KeybaseError>),
    ListConversations(Result<ListConversationsOk, KeybaseError>),
    /// Response for [`WorkerRequest::MarkReadSilent`] — routed by variant.
    MarkReadSilent(Result<(), KeybaseError>),
    /// Response for [`WorkerRequest::ListConversationsSilent`] — routed
    /// in `apply_response` before the `in_flight` match.
    ListConversationsSilent(Result<ListConversationsOk, KeybaseError>),
    ReadMessages(Result<(Vec<Message>, Option<String>), KeybaseError>),
    MarkRead(Result<(), KeybaseError>),
    SearchInboxHits(Result<Vec<InboxHit>, KeybaseError>),
    GiphySearch(Result<Vec<crate::domain::GiphyHit>, KeybaseError>),
    SearchRegexp(Result<Vec<InboxHit>, KeybaseError>),
    SendMessage(Result<(), KeybaseError>),
    EditMessage(Result<(), KeybaseError>),
    DeleteMessage(Result<(), KeybaseError>),
    React(Result<(), KeybaseError>),
    NewConversation(Result<String, KeybaseError>),
    LoadChannels(Result<ListConversationsOk, KeybaseError>),
    JoinChannel(Result<(), KeybaseError>),
    LeaveChannel(Result<(), KeybaseError>),
    LoadMembers(Result<Vec<ChatMember>, KeybaseError>),
    AddToChannel(Result<(), KeybaseError>),
    RemoveFromChannel(Result<(), KeybaseError>),
    RenameChannel(Result<(), KeybaseError>),
    DeleteChannel(Result<(), KeybaseError>),
    DefaultChannels(Result<Vec<String>, KeybaseError>),
    SetConvStatus(Result<(), KeybaseError>),
    PinMessage(Result<(), KeybaseError>),
    UnpinMessage(Result<(), KeybaseError>),
    /// Response for [`WorkerRequest::GetPinnedMessage`] — routed by variant.
    GetPinnedMessage {
        conv_id: String,
        message_id: u64,
        result: Result<Option<Message>, KeybaseError>,
    },
    DownloadAttachment(Result<(), KeybaseError>),
    /// The cache **path** + the download result (the path is carried even on
    /// error so the failure can be pinned to the right attachment).
    PreviewImage(String, Result<(), KeybaseError>),
    UploadAttachment(Result<(), KeybaseError>),
    /// Decoded GIF frames (or `None` for a still / single-frame GIF, or when
    /// ImageMagick isn't available). Carries the source cache path to key it
    /// back into `App::gif_anims`.
    DecodeGif(String, Option<crate::tui::image::GifFrames>),
    Emojis(Result<Vec<Emoji>, KeybaseError>),
    ListSelfMemberships(Result<ListTeamsOk, KeybaseError>),
}

/// Caller-side context for an in-flight request. Mirrors
/// [`WorkerRequest`] but only carries the fields the response handler
/// needs *after* the port call returns (e.g. which conv to mark read
/// locally, which target_id to log).
///
/// Stored on `App::in_flight` and consumed when the matching
/// [`WorkerResponse`] arrives.
#[derive(Debug, Clone)]
pub enum InFlight {
    /// `keybase status` check. On success: logged in → keep the splash as a
    /// loading screen and load the inbox before entering it; logged out →
    /// Login. Fired once at boot and from the Login screen's retry.
    Status,
    Logout,
    LoginPaperkey,
    LoadInbox,
    LoadMessages,
    LoadOlderMessages,
    MarkRead {
        conv_id: String,
    },
    SearchInboxRemote,
    ConvSearch,
    GiphySearch,
    SendMessage {
        body_len: usize,
        was_reply: bool,
    },
    EditMessage {
        target_id: u64,
    },
    DeleteMessage {
        message_id: u64,
    },
    SendReaction {
        body: String,
    },
    NewConversation,
    /// Create a team channel (`newconv` on a team channel) — reuses the
    /// `NewConversation` request/response but routes to the channel handler.
    /// `topic` is the new channel name (for the toast + browser reload).
    CreateChannel {
        topic: String,
    },
    /// Channel-browser load for a team (`listconvsonname`).
    LoadChannels,
    /// Join a team channel; `topic` is the channel name (for the toast).
    JoinChannel {
        topic: String,
    },
    /// Leave a team channel; `topic` is the channel name (for the toast).
    LeaveChannel {
        topic: String,
    },
    /// Load the members of a channel/conversation for the Members view.
    LoadMembers,
    /// Add `count` members to a channel; count drives the toast.
    AddToChannel {
        count: usize,
    },
    /// Remove a member from a channel; `username` for the toast.
    RemoveFromChannel {
        username: String,
    },
    /// Rename a channel; `topic` is the new name (for the toast).
    RenameChannel {
        topic: String,
    },
    /// Delete a channel; `topic` is the channel name (for the toast).
    DeleteChannel {
        topic: String,
    },
    /// Get / set the team default channels; `setting` distinguishes the two
    /// (the load-time get is quiet, the set toasts).
    DefaultChannels {
        setting: bool,
    },
    /// Any `setstatus` call (mute / unmute / ignore / block / report /
    /// favorite). `done_label` is the feedback shown on success.
    SetConvStatus {
        done_label: String,
    },
    PinMessage {
        message_id: u64,
    },
    UnpinConversation,
    DownloadAttachment {
        message_id: u64,
        path: String,
    },
    UploadAttachment {
        filename: String,
    },
    LoadTeams,
}

/// Owning handle to the worker thread. Stores the channel halves in
/// [`Option`]s so callers can `take_tx` / `take_rx` to move them into
/// the App without violating Drop. The thread is reaped on drop:
/// closing the request channel triggers a clean exit of the worker
/// loop, and `join` blocks until it returns.
pub struct WorkerHandle {
    tx: Option<Sender<WorkerRequest>>,
    rx: Option<Receiver<WorkerResponse>>,
    join: Option<JoinHandle<()>>,
    /// Held until [`spawn_extra`](Self::spawn_extra) **takes** it for the
    /// background lane (same response channel). After that only the worker
    /// threads hold senders, so a dead-workers state is observable as
    /// `Disconnected` on the receive half.
    resp_tx: Option<Sender<WorkerResponse>>,
    extra_tx: Option<Sender<WorkerRequest>>,
    extra_join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    /// Moves `keybase` into a fresh worker thread. The `Send` bound is
    /// required so the trait object can cross the thread boundary —
    /// `KeybaseCliAdapter` satisfies it (only `u64` fields).
    pub fn spawn(mut keybase: Box<dyn KeybasePort + Send>) -> Self {
        let (req_tx, req_rx) = channel::<WorkerRequest>();
        let (resp_tx, resp_rx) = channel::<WorkerResponse>();
        let resp_tx_main = resp_tx.clone();
        let join = std::thread::spawn(move || {
            run_worker(&mut *keybase, req_rx, resp_tx_main);
        });
        Self {
            tx: Some(req_tx),
            rx: Some(resp_rx),
            join: Some(join),
            resp_tx: Some(resp_tx),
            extra_tx: None,
            extra_join: None,
        }
    }

    /// Spawns a **background** worker owning its own `keybase` port and
    /// shipping onto the shared response channel. Used for the idle
    /// inbox auto-refresh so it never head-of-line-blocks the user's
    /// lane. Returns the request sender for that lane.
    ///
    /// Single-use: it **takes** the handle's response sender, so once both
    /// lanes exist only the worker threads hold senders — if every worker
    /// dies, the run loop's `try_recv` sees `Disconnected` and can unwedge
    /// the UI instead of spinning "busy" forever.
    pub fn spawn_extra(
        &mut self,
        mut keybase: Box<dyn KeybasePort + Send>,
    ) -> Sender<WorkerRequest> {
        let (req_tx, req_rx) = channel::<WorkerRequest>();
        let resp_tx = self.resp_tx.take().expect("spawn_extra is single-use");
        let join = std::thread::spawn(move || {
            // One-shot startup chores on this idle lane, off the render
            // thread: warm the syntect grammar/theme dumps (so the first
            // rendered code block doesn't pay the load hitch) and sweep
            // stale entries from the on-disk image cache.
            crate::tui::syntax::preload();
            crate::tui::image::sweep_disk_cache();
            run_worker(&mut *keybase, req_rx, resp_tx);
        });
        self.extra_join = Some(join);
        self.extra_tx = Some(req_tx.clone());
        req_tx
    }

    /// Hands out a clone of the request sender. The original sender
    /// is kept inside the handle so dropping the handle closes the
    /// channel (which is what tells the worker to exit).
    pub fn tx(&self) -> Sender<WorkerRequest> {
        self.tx.as_ref().expect("worker tx already taken").clone()
    }

    /// Moves the response receiver out. Can only be called once.
    pub fn take_rx(&mut self) -> Receiver<WorkerResponse> {
        self.rx.take().expect("worker rx already taken")
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // Signal both workers to exit and DETACH (do not join).
        //
        // Joining would block here until the worker observes Shutdown —
        // but an idle worker is parked inside a long blocking `keybase`
        // call (up to the per-op timeout) and won't reach the `recv`
        // that sees Shutdown until that call returns. Blocking on the
        // join keeps the terminal frozen, so Ctrl+C during an in-flight
        // call felt unresponsive. Detaching lets shutdown / terminal
        // restore happen immediately; when the process exits the OS
        // closes the worker's stdin pipes, so any `keybase` child gets
        // EOF and exits — no orphaned processes.
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(WorkerRequest::Shutdown);
        }
        if let Some(tx) = self.extra_tx.take() {
            let _ = tx.send(WorkerRequest::Shutdown);
        }
        // Drop the join handles without joining (detach the threads).
        self.join.take();
        self.extra_join.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::domain::{Conversation, IdentityInfo, InboxHit, Message, TeamMembership};
    use crate::ports::keybase::{ListConversationsOk, ListTeamsOk};
    use zeroize::Zeroizing;

    /// Port stub whose `status()` panics on the first call and
    /// returns Ok thereafter. Every other method is a no-op success.
    /// We use it to verify that a panic in one request does not kill
    /// the worker thread.
    #[derive(Default)]
    struct PanicOnFirstStatus {
        call_count: Arc<AtomicUsize>,
    }

    impl KeybasePort for PanicOnFirstStatus {
        fn status(&mut self) -> Result<IdentityInfo, KeybaseError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                panic!("synthetic adapter panic");
            }
            Ok(IdentityInfo::default())
        }
        fn logout(&mut self) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn login_paperkey(&mut self, _: &str, _: &str, _: &str) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn list_conversations(&mut self) -> Result<ListConversationsOk, KeybaseError> {
            Ok(ListConversationsOk {
                conversations: Vec::<Conversation>::new(),
                skipped: Vec::new(),
            })
        }
        fn read_messages(
            &mut self,
            _: &ReadChannel,
            _: u32,
            _: bool,
            _: Option<&str>,
        ) -> Result<(Vec<Message>, Option<String>), KeybaseError> {
            Ok((Vec::new(), None))
        }
        fn mark_read(&mut self, _: &ReadChannel, _: u64) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn search_inbox(&mut self, _: &str, _: u32) -> Result<Zeroizing<String>, KeybaseError> {
            Ok(Zeroizing::new(String::new()))
        }
        fn send_message(
            &mut self,
            _: &ReadChannel,
            _: &str,
            _: Option<u64>,
        ) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn edit_message(&mut self, _: &ReadChannel, _: u64, _: &str) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn delete_message(&mut self, _: &ReadChannel, _: u64) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn react(&mut self, _: &ReadChannel, _: u64, _: &str) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn new_conversation(&mut self, _: &ReadChannel) -> Result<String, KeybaseError> {
            Ok(String::new())
        }
        fn list_channels_on_name(&mut self, _: &str) -> Result<ListConversationsOk, KeybaseError> {
            Ok(ListConversationsOk {
                conversations: Vec::new(),
                skipped: Vec::new(),
            })
        }
        fn join_channel(&mut self, _: &ReadChannel) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn leave_channel(&mut self, _: &ReadChannel) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn list_members(&mut self, _: &ReadChannel) -> Result<Vec<ChatMember>, KeybaseError> {
            Ok(Vec::new())
        }
        fn add_to_channel(&mut self, _: &ReadChannel, _: &[String]) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn remove_from_channel(
            &mut self,
            _: &ReadChannel,
            _: &[String],
        ) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn rename_channel(&mut self, _: &str, _: &str, _: &str) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn delete_channel(&mut self, _: &str, _: &str) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn default_channels(&mut self, _: &str, _: &[String]) -> Result<Vec<String>, KeybaseError> {
            Ok(Vec::new())
        }
        fn set_conversation_status(
            &mut self,
            _: &ReadChannel,
            _: &str,
        ) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn pin_message(&mut self, _: &ReadChannel, _: u64) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn get_message(
            &mut self,
            _: &ReadChannel,
            _: u64,
        ) -> Result<Option<Message>, KeybaseError> {
            Ok(None)
        }
        fn unpin_message(&mut self, _: &ReadChannel) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn download_attachment(
            &mut self,
            _: &ReadChannel,
            _: u64,
            _: &str,
        ) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn upload_attachment(
            &mut self,
            _: &ReadChannel,
            _: &str,
            _: &str,
        ) -> Result<(), KeybaseError> {
            Ok(())
        }
        fn list_emojis(&mut self) -> Result<Vec<crate::domain::Emoji>, KeybaseError> {
            Ok(Vec::new())
        }
        fn list_self_memberships(&mut self, _: &str) -> Result<ListTeamsOk, KeybaseError> {
            Ok(ListTeamsOk {
                teams: Vec::<TeamMembership>::new(),
                skipped: Vec::new(),
            })
        }
        fn search_inbox_hits(&mut self, _: &str, _: u32) -> Result<Vec<InboxHit>, KeybaseError> {
            Ok(Vec::new())
        }
        fn search_regexp(
            &mut self,
            _: &ReadChannel,
            _: &str,
            _: u32,
        ) -> Result<Zeroizing<String>, KeybaseError> {
            Ok(Zeroizing::new(r#"{"result":{"hits":[]}}"#.to_string()))
        }
    }

    #[test]
    fn worker_survives_panic_and_serves_next_request() {
        // Silence the default panic hook so the test output isn't
        // polluted by the synthetic panic stack we deliberately
        // trigger below.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let port = PanicOnFirstStatus::default();
        let counter = port.call_count.clone();
        let mut worker = WorkerHandle::spawn(Box::new(port));
        let tx = worker.tx();
        let rx = worker.take_rx();

        // First Status: should produce an Internal error from the
        // synthetic panic.
        tx.send(WorkerRequest::Status).unwrap();
        match rx.recv().unwrap() {
            WorkerResponse::Status(Err(KeybaseError::Internal(msg))) => {
                assert!(
                    msg.contains("synthetic adapter panic"),
                    "panic payload should round-trip into Internal: {msg}"
                );
            }
            other => panic!("expected Internal error, got {other:?}"),
        }

        // Second Status: worker is still alive — counter went past
        // the panicking branch.
        tx.send(WorkerRequest::Status).unwrap();
        match rx.recv().unwrap() {
            WorkerResponse::Status(Ok(_)) => {}
            other => panic!("expected Ok, got {other:?}"),
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "both calls should have reached the port"
        );

        drop(worker);
        std::panic::set_hook(prev_hook);
    }

    impl std::fmt::Debug for WorkerResponse {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // Minimal Debug — enough for `panic!("got {other:?}")`
            // in the test above; we don't expose this widely so a
            // hand-rolled impl is fine.
            match self {
                Self::Status(r) => write!(f, "Status({r:?})"),
                Self::Logout(r) => write!(f, "Logout({r:?})"),
                Self::LoginPaperkey(r) => write!(f, "LoginPaperkey({r:?})"),
                Self::ListConversations(_) => f.write_str("ListConversations(..)"),
                Self::ListConversationsSilent(_) => f.write_str("ListConversationsSilent(..)"),
                Self::MarkReadSilent(r) => write!(f, "MarkReadSilent({r:?})"),
                Self::ReadMessages(_) => f.write_str("ReadMessages(..)"),
                Self::MarkRead(r) => write!(f, "MarkRead({r:?})"),
                Self::SearchInboxHits(_) => f.write_str("SearchInboxHits(..)"),
                Self::GiphySearch(_) => f.write_str("GiphySearch(..)"),
                Self::SearchRegexp(_) => f.write_str("SearchRegexp(..)"),
                Self::SendMessage(r) => write!(f, "SendMessage({r:?})"),
                Self::EditMessage(r) => write!(f, "EditMessage({r:?})"),
                Self::DeleteMessage(r) => write!(f, "DeleteMessage({r:?})"),
                Self::React(r) => write!(f, "React({r:?})"),
                Self::NewConversation(r) => write!(f, "NewConversation({r:?})"),
                Self::LoadChannels(_) => f.write_str("LoadChannels(..)"),
                Self::JoinChannel(r) => write!(f, "JoinChannel({r:?})"),
                Self::LeaveChannel(r) => write!(f, "LeaveChannel({r:?})"),
                Self::LoadMembers(_) => f.write_str("LoadMembers(..)"),
                Self::AddToChannel(r) => write!(f, "AddToChannel({r:?})"),
                Self::RemoveFromChannel(r) => write!(f, "RemoveFromChannel({r:?})"),
                Self::RenameChannel(r) => write!(f, "RenameChannel({r:?})"),
                Self::DeleteChannel(r) => write!(f, "DeleteChannel({r:?})"),
                Self::DefaultChannels(r) => write!(f, "DefaultChannels({r:?})"),
                Self::SetConvStatus(r) => write!(f, "SetConvStatus({r:?})"),
                Self::PinMessage(r) => write!(f, "PinMessage({r:?})"),
                Self::UnpinMessage(r) => write!(f, "UnpinMessage({r:?})"),
                Self::GetPinnedMessage { .. } => f.write_str("GetPinnedMessage(..)"),
                Self::DownloadAttachment(r) => write!(f, "DownloadAttachment({r:?})"),
                Self::PreviewImage(p, r) => write!(f, "PreviewImage({p}, {r:?})"),
                Self::UploadAttachment(r) => write!(f, "UploadAttachment({r:?})"),
                Self::DecodeGif(p, _) => write!(f, "DecodeGif({p}, ..)"),
                Self::Emojis(_) => f.write_str("Emojis(..)"),
                Self::ListSelfMemberships(_) => f.write_str("ListSelfMemberships(..)"),
            }
        }
    }
}

fn run_worker(
    keybase: &mut dyn KeybasePort,
    req_rx: Receiver<WorkerRequest>,
    resp_tx: Sender<WorkerResponse>,
) {
    while let Ok(req) = req_rx.recv() {
        let resp = match req {
            WorkerRequest::Shutdown => break,
            WorkerRequest::Status => WorkerResponse::Status(run_caught(|| keybase.status())),
            WorkerRequest::Logout => WorkerResponse::Logout(run_caught(|| keybase.logout())),
            WorkerRequest::LoginPaperkey {
                username,
                device,
                paperkey,
            } => WorkerResponse::LoginPaperkey(run_caught(|| {
                keybase.login_paperkey(&username, &device, &paperkey)
            })),
            WorkerRequest::ListConversations => {
                WorkerResponse::ListConversations(run_caught(|| keybase.list_conversations()))
            }
            WorkerRequest::ListConversationsSilent => {
                WorkerResponse::ListConversationsSilent(run_caught(|| keybase.list_conversations()))
            }
            WorkerRequest::MarkReadSilent {
                channel,
                message_id,
            } => WorkerResponse::MarkReadSilent(run_caught(|| {
                keybase.mark_read(&channel, message_id)
            })),
            WorkerRequest::ReadMessages {
                channel,
                num,
                peek,
                next_cursor,
            } => WorkerResponse::ReadMessages(run_caught(|| {
                keybase.read_messages(&channel, num, peek, next_cursor.as_deref())
            })),
            WorkerRequest::MarkRead {
                channel,
                message_id,
            } => WorkerResponse::MarkRead(run_caught(|| keybase.mark_read(&channel, message_id))),
            WorkerRequest::SearchInboxHits { query, max_hits } => {
                WorkerResponse::SearchInboxHits(run_caught(|| {
                    keybase.search_inbox_hits(&query, max_hits)
                }))
            }
            WorkerRequest::GiphySearch { api_key, query } => {
                WorkerResponse::GiphySearch(run_caught(|| {
                    crate::adapters::web_fetch::giphy_search(&api_key, &query)
                }))
            }
            WorkerRequest::SearchRegexp {
                channel,
                query,
                max_hits,
            } => WorkerResponse::SearchRegexp(run_caught(|| {
                keybase.search_regexp_hits(&channel, &query, max_hits)
            })),
            WorkerRequest::SendMessage {
                channel,
                body,
                reply_to,
            } => WorkerResponse::SendMessage(run_caught(|| {
                keybase.send_message(&channel, &body, reply_to)
            })),
            WorkerRequest::EditMessage {
                channel,
                message_id,
                body,
            } => WorkerResponse::EditMessage(run_caught(|| {
                keybase.edit_message(&channel, message_id, &body)
            })),
            WorkerRequest::DeleteMessage {
                channel,
                message_id,
            } => WorkerResponse::DeleteMessage(run_caught(|| {
                keybase.delete_message(&channel, message_id)
            })),
            WorkerRequest::React {
                channel,
                message_id,
                body,
            } => WorkerResponse::React(run_caught(|| keybase.react(&channel, message_id, &body))),
            WorkerRequest::NewConversation { channel } => {
                WorkerResponse::NewConversation(run_caught(|| keybase.new_conversation(&channel)))
            }
            WorkerRequest::LoadChannels { team } => {
                WorkerResponse::LoadChannels(run_caught(|| keybase.list_channels_on_name(&team)))
            }
            WorkerRequest::JoinChannel { channel } => {
                WorkerResponse::JoinChannel(run_caught(|| keybase.join_channel(&channel)))
            }
            WorkerRequest::LeaveChannel { channel } => {
                WorkerResponse::LeaveChannel(run_caught(|| keybase.leave_channel(&channel)))
            }
            WorkerRequest::LoadMembers { channel } => {
                WorkerResponse::LoadMembers(run_caught(|| keybase.list_members(&channel)))
            }
            WorkerRequest::AddToChannel { channel, usernames } => {
                WorkerResponse::AddToChannel(run_caught(|| {
                    keybase.add_to_channel(&channel, &usernames)
                }))
            }
            WorkerRequest::RemoveFromChannel { channel, usernames } => {
                WorkerResponse::RemoveFromChannel(run_caught(|| {
                    keybase.remove_from_channel(&channel, &usernames)
                }))
            }
            WorkerRequest::RenameChannel { team, old, new } => {
                WorkerResponse::RenameChannel(run_caught(|| {
                    keybase.rename_channel(&team, &old, &new)
                }))
            }
            WorkerRequest::DeleteChannel { team, channel } => {
                WorkerResponse::DeleteChannel(run_caught(|| {
                    keybase.delete_channel(&team, &channel)
                }))
            }
            WorkerRequest::DefaultChannels { team, set } => {
                WorkerResponse::DefaultChannels(run_caught(|| {
                    keybase.default_channels(&team, &set)
                }))
            }
            WorkerRequest::SetConvStatus { channel, status } => {
                WorkerResponse::SetConvStatus(run_caught(|| {
                    keybase.set_conversation_status(&channel, &status)
                }))
            }
            WorkerRequest::PinMessage {
                channel,
                message_id,
            } => {
                WorkerResponse::PinMessage(run_caught(|| keybase.pin_message(&channel, message_id)))
            }
            WorkerRequest::UnpinMessage { channel } => {
                WorkerResponse::UnpinMessage(run_caught(|| keybase.unpin_message(&channel)))
            }
            WorkerRequest::GetPinnedMessage {
                conv_id,
                channel,
                message_id,
            } => WorkerResponse::GetPinnedMessage {
                conv_id,
                message_id,
                result: run_caught(|| keybase.get_message(&channel, message_id)),
            },
            WorkerRequest::DownloadAttachment {
                channel,
                message_id,
                output,
            } => WorkerResponse::DownloadAttachment(run_caught(|| {
                keybase.download_attachment(&channel, message_id, &output)
            })),
            WorkerRequest::PreviewImage {
                channel,
                message_id,
                output,
            } => WorkerResponse::PreviewImage(
                output.clone(),
                run_caught(|| keybase.download_attachment(&channel, message_id, &output)),
            ),
            WorkerRequest::FetchWebImage { url, output } => WorkerResponse::PreviewImage(
                output.clone(),
                run_caught(|| crate::adapters::web_fetch::fetch_giphy_media(&url, &output)),
            ),
            WorkerRequest::UploadAttachment {
                channel,
                filename,
                title,
            } => WorkerResponse::UploadAttachment(run_caught(|| {
                keybase.upload_attachment(&channel, &filename, &title)
            })),
            WorkerRequest::DecodeGif { path } => {
                // Pure image work (not a keybase call). Wrap in catch_unwind for
                // the same panic isolation as the port calls.
                let frames = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    crate::tui::image::extract_gif_frames(
                        &path,
                        &crate::tui::image::gif_frame_dir(&path),
                    )
                }))
                .unwrap_or(None);
                WorkerResponse::DecodeGif(path, frames)
            }
            WorkerRequest::ListEmojis => {
                WorkerResponse::Emojis(run_caught(|| keybase.list_emojis()))
            }
            WorkerRequest::ListSelfMemberships { username } => {
                WorkerResponse::ListSelfMemberships(run_caught(|| {
                    keybase.list_self_memberships(&username)
                }))
            }
        };
        if resp_tx.send(resp).is_err() {
            break;
        }
    }
}
