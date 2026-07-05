//! Tests for the chat flow surface.
//!
//! Each test drives the request/response cycle end-to-end:
//!
//! 1. Build an `App` whose worker thread holds a [`MockKeybase`].
//! 2. Call a `request_*` function. The mock records the call and
//!    queues a (pre-baked) response.
//! 3. Call [`pump_until_idle`] (or [`pump_one`] when an intermediate
//!    state assertion is needed). Each `recv` dispatches through
//!    [`apply_response`].
//! 4. Assert app state and / or mock-recorded calls.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use crate::domain::{
    AttachmentInfo, Channel, ChatMember, Conversation, Emoji, IdentityInfo, InboxHit, MemberStatus,
    MembersType, Message, MessageContent, TeamMembership, TeamRole,
};
use crate::ports::KeybaseError;
use crate::ports::keybase::{KeybasePort, ListConversationsOk, ListTeamsOk, ReadChannel};
use crate::ports::{ClipboardPort, SettingsPort, UserSettings};
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::flows::{apply_response, chat::*};
use crate::tui::screens::Screen;
use crate::tui::worker::{InFlight, WorkerHandle, WorkerResponse};

// ── Mocks ─────────────────────────────────────────────────────────────

/// Recording state shared between the test and the
/// worker-thread-owned [`MockKeybase`]. `Arc<Mutex<>>` because the
/// mock has to be `Send` to cross into the worker.
#[derive(Default)]
struct MockState {
    // Pre-baked responses.
    conversations: Vec<Conversation>,
    /// Diagnostics returned alongside `conversations` to simulate a
    /// partial parse failure in the adapter.
    conversations_skipped: Vec<String>,
    messages: Vec<Message>,
    messages_next: Option<String>,
    teams: Vec<TeamMembership>,
    /// Diagnostics returned alongside `teams` (partial parse failure).
    teams_skipped: Vec<String>,
    search_hits: Vec<InboxHit>,
    status: IdentityInfo,
    new_conv_id: String,
    // Call recording.
    sent: Vec<(String, String, Option<u64>)>,
    edits: Vec<(u64, String)>,
    deletes: Vec<u64>,
    reactions: Vec<(u64, String)>,
    new_convs: Vec<String>,
    statuses: Vec<(String, String)>,
    pins: Vec<u64>,
    unpins: usize,
    downloads: Vec<(u64, String)>,
    uploads: Vec<(String, String)>,
    emojis: Vec<Emoji>,
    mark_reads: Vec<u64>,
    read_calls: Vec<Option<String>>,
    /// Channel-browser: pre-baked `listconvsonname` result + recorded
    /// join / leave topic names.
    channels: Vec<Conversation>,
    joined: Vec<String>,
    left: Vec<String>,
    renames: Vec<(String, String, String)>,
    deleted_channels: Vec<String>,
    /// `default-channels`: the get result + recorded set calls.
    default_channels_get: Vec<String>,
    default_channels_set: Vec<Vec<String>>,
    /// Members: pre-baked `listmembers` result + recorded add/remove calls.
    members: Vec<ChatMember>,
    added_members: Vec<Vec<String>>,
    removed_members: Vec<Vec<String>>,
    /// Recorded paper-key login calls: (username, device, paperkey).
    logins: Vec<(String, String, String)>,
    /// Next adapter call returning a Result returns this error then
    /// clears the slot.
    fail_next: Option<KeybaseError>,
}

#[derive(Default, Clone)]
struct MockKeybase(Arc<Mutex<MockState>>);

impl MockKeybase {
    fn st(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.0.lock().unwrap()
    }
}

impl KeybasePort for MockKeybase {
    fn status(&mut self) -> Result<IdentityInfo, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(s.status.clone())
    }
    fn logout(&mut self) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(())
    }
    fn login_paperkey(
        &mut self,
        username: &str,
        device: &str,
        paperkey: &str,
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.logins
            .push((username.into(), device.into(), paperkey.into()));
        Ok(())
    }
    fn list_conversations(&mut self) -> Result<ListConversationsOk, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(ListConversationsOk {
            conversations: s.conversations.clone(),
            skipped: s.conversations_skipped.clone(),
        })
    }
    fn read_messages(
        &mut self,
        _channel: &ReadChannel,
        _num: u32,
        _peek: bool,
        next: Option<&str>,
    ) -> Result<(Vec<Message>, Option<String>), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.read_calls.push(next.map(str::to_string));
        Ok((s.messages.clone(), s.messages_next.clone()))
    }
    fn mark_read(&mut self, _: &ReadChannel, id: u64) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.mark_reads.push(id);
        Ok(())
    }
    fn search_inbox(&mut self, _: &str, _: u32) -> Result<Zeroizing<String>, KeybaseError> {
        Ok(Zeroizing::new(String::new()))
    }
    fn search_inbox_hits(&mut self, _: &str, _: u32) -> Result<Vec<InboxHit>, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(s.search_hits.clone())
    }
    fn search_regexp(
        &mut self,
        _: &ReadChannel,
        _: &str,
        _: u32,
    ) -> Result<Zeroizing<String>, KeybaseError> {
        Ok(Zeroizing::new(r#"{"result":{"hits":[]}}"#.to_string()))
    }
    fn search_regexp_hits(
        &mut self,
        _: &ReadChannel,
        _: &str,
        _: u32,
    ) -> Result<Vec<InboxHit>, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(s.search_hits.clone())
    }
    fn send_message(
        &mut self,
        channel: &ReadChannel,
        body: &str,
        reply_to: Option<u64>,
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.sent
            .push((channel.name.clone(), body.to_string(), reply_to));
        Ok(())
    }
    fn edit_message(&mut self, _: &ReadChannel, id: u64, body: &str) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.edits.push((id, body.to_string()));
        Ok(())
    }
    fn delete_message(&mut self, _: &ReadChannel, id: u64) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.deletes.push(id);
        Ok(())
    }
    fn react(&mut self, _: &ReadChannel, id: u64, body: &str) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.reactions.push((id, body.to_string()));
        Ok(())
    }
    fn upload_attachment(
        &mut self,
        _: &ReadChannel,
        filename: &str,
        title: &str,
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.uploads.push((filename.to_string(), title.to_string()));
        Ok(())
    }
    fn list_emojis(&mut self) -> Result<Vec<Emoji>, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(s.emojis.clone())
    }
    fn new_conversation(&mut self, channel: &ReadChannel) -> Result<String, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.new_convs.push(channel.name.clone());
        Ok(s.new_conv_id.clone())
    }
    fn list_channels_on_name(&mut self, _: &str) -> Result<ListConversationsOk, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(ListConversationsOk {
            conversations: s.channels.clone(),
            skipped: Vec::new(),
        })
    }
    fn join_channel(&mut self, ch: &ReadChannel) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.joined.push(ch.topic_name.clone().unwrap_or_default());
        Ok(())
    }
    fn leave_channel(&mut self, ch: &ReadChannel) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.left.push(ch.topic_name.clone().unwrap_or_default());
        Ok(())
    }
    fn list_members(&mut self, _: &ReadChannel) -> Result<Vec<ChatMember>, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(s.members.clone())
    }
    fn add_to_channel(
        &mut self,
        _: &ReadChannel,
        usernames: &[String],
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.added_members.push(usernames.to_vec());
        Ok(())
    }
    fn remove_from_channel(
        &mut self,
        _: &ReadChannel,
        usernames: &[String],
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.removed_members.push(usernames.to_vec());
        Ok(())
    }
    fn rename_channel(&mut self, team: &str, old: &str, new: &str) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.renames
            .push((team.to_string(), old.to_string(), new.to_string()));
        Ok(())
    }
    fn delete_channel(&mut self, _team: &str, channel: &str) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.deleted_channels.push(channel.to_string());
        Ok(())
    }
    fn default_channels(&mut self, _: &str, set: &[String]) -> Result<Vec<String>, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        if set.is_empty() {
            Ok(s.default_channels_get.clone())
        } else {
            // SET replaces the set, then the CLI prints the new one back.
            s.default_channels_set.push(set.to_vec());
            s.default_channels_get = set.to_vec();
            Ok(set.to_vec())
        }
    }
    fn set_conversation_status(
        &mut self,
        channel: &ReadChannel,
        status: &str,
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.statuses.push((channel.name.clone(), status.to_string()));
        Ok(())
    }
    fn pin_message(&mut self, _: &ReadChannel, id: u64) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.pins.push(id);
        Ok(())
    }
    fn unpin_message(&mut self, _: &ReadChannel) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.unpins += 1;
        Ok(())
    }
    fn download_attachment(
        &mut self,
        _: &ReadChannel,
        id: u64,
        output: &str,
    ) -> Result<(), KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        s.downloads.push((id, output.to_string()));
        Ok(())
    }
    fn list_self_memberships(&mut self, _: &str) -> Result<ListTeamsOk, KeybaseError> {
        let mut s = self.0.lock().unwrap();
        if let Some(e) = s.fail_next.take() {
            return Err(e);
        }
        Ok(ListTeamsOk {
            teams: s.teams.clone(),
            skipped: s.teams_skipped.clone(),
        })
    }
}

struct FakeClipboard;
impl ClipboardPort for FakeClipboard {
    fn write(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

struct FakeOpener;
impl crate::ports::OpenerPort for FakeOpener {
    fn open(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

struct FakeSettings;
impl SettingsPort for FakeSettings {
    fn read(&self) -> UserSettings {
        UserSettings::default()
    }
    fn write_setting(&self, _: &str, _: &str) -> bool {
        true
    }
    fn write_theme_name(&self, _: &str) -> bool {
        true
    }
    fn config_dir(&self) -> PathBuf {
        PathBuf::from(".")
    }
}

// ── Test rig ───────────────────────────────────────────────────────────

/// Bundle returned by [`build_app`] — owns the worker handle so it
/// stays alive for the test scope and is dropped (joined) at the end.
struct Rig {
    app: App,
    mock: MockKeybase,
    /// Kept in scope to reap the worker thread on drop.
    _worker: WorkerHandle,
}

/// Builds a fresh [`App`] backed by a worker thread that owns the
/// [`MockKeybase`]. Returns a [`Rig`] holding both ends so tests
/// can poke the mock state and drive the worker.
fn build_rig() -> Rig {
    let mock = MockKeybase::default();
    let port: Box<dyn KeybasePort + Send> = Box::new(mock.clone());
    let mut worker = WorkerHandle::spawn(port);
    let tx = worker.tx();
    let bg_tx = worker.spawn_extra(Box::new(mock.clone()));
    let rx = worker.take_rx();
    let app = App::new(
        tx,
        bg_tx,
        rx,
        None,
        Box::new(FakeClipboard),
        Box::new(FakeOpener),
        Box::new(FakeSettings),
    );
    Rig {
        app,
        mock,
        _worker: worker,
    }
}

/// Drains one worker response (blocks if none has arrived yet) and
/// dispatches it through [`apply_response`]. Use when you need to
/// inspect intermediate state between two chained requests.
fn pump_one(app: &mut App) {
    let resp = app
        .worker_rx
        .recv()
        .expect("worker thread died before response");
    apply_response(app, resp);
}

/// Loops [`pump_one`] until the in-flight slot is empty. Handles
/// chained requests (e.g. send-message → load-messages) by recursing
/// into each follow-up response.
fn pump_until_idle(app: &mut App) {
    while app.in_flight.is_some() {
        pump_one(app);
    }
}

fn conv(id: &str, name: &str, members: MembersType) -> Conversation {
    Conversation {
        id: id.into(),
        channel: Channel {
            name: name.into(),
            members_type: members,
            topic_name: None,
        },
        unread: false,
        active_at: 0,
        active_at_ms: 0,
        member_status: MemberStatus::Active,
        creator_info: None,
    }
}

fn text_msg(id: u64, sender: &str, body: &str) -> Message {
    Message {
        id,
        sender: sender.into(),
        device: "test".into(),
        sent_at: 0,
        sent_at_ms: 0,
        content: MessageContent::Text(body.into()),
        reactions: Vec::new(),
        reply_to: None,
        edited: false,
        mentions: Vec::new(),
    }
}

fn set_identity(app: &mut App, username: &str) {
    let mut info = IdentityInfo::default();
    info.username = username.into();
    info.logged_in = true;
    app.identity = info;
}

/// Pre-loads the inbox synchronously (via `request_load_inbox` +
/// `pump_until_idle`) and points `open_conv_id` at `target`.
/// Expands every tree group and puts the cursor on the first conversation,
/// so selection-based actions (mute / mark / ignore / open) have a target.
/// The tree starts fully collapsed, so tests that act on a selection call this
/// after loading.
fn reveal_first(app: &mut App) {
    let teams: Vec<String> = app
        .conversations
        .iter()
        .filter(|c| c.channel.members_type.is_team())
        .map(|c| c.channel.name.clone())
        .collect();
    app.expanded.insert(App::DMS_KEY.to_string());
    for t in teams {
        app.expanded.insert(t);
    }
    app.rebuild_filter(); // re-seats the cursor on the first conversation row
}

fn preload_inbox(app: &mut App, mock: &MockKeybase, convs: Vec<Conversation>, target: &str) {
    mock.st().conversations = convs;
    request_load_inbox(app);
    pump_until_idle(app);
    app.open_conv_id = Some(target.into());
    // The tree starts fully collapsed; expand every group so tests see the
    // conversations, then put the cursor on `target` (so selected_conversation
    // resolves to it, like the old flat-list default did).
    let teams: Vec<String> = app
        .conversations
        .iter()
        .filter(|c| c.channel.members_type.is_team())
        .map(|c| c.channel.name.clone())
        .collect();
    app.expanded.insert(App::DMS_KEY.to_string());
    for t in teams {
        app.expanded.insert(t);
    }
    app.rebuild_filter();
    if let Some(pos) = app.tree_rows().iter().position(|r| {
        matches!(r, crate::tui::app::TreeRow::Conv { idx } if app.conversations[*idx].id == target)
    }) {
        app.tree_selected = pos;
    }
}

// ── do_load_inbox → request_load_inbox ───────────────────────────────

#[test]
fn load_inbox_populates_conversations_and_rebuilds_filter() {
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![
        conv("c1", "alice,bob", MembersType::ImpTeamNative),
        conv("c2", "team1", MembersType::Team),
    ];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.conversations.len(), 2);
    // Default filter is All → both the DM and the team conversation match.
    assert_eq!(rig.app.filtered_cache.len(), 2);
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
}

#[test]
fn load_inbox_surfaces_errors() {
    let mut rig = build_rig();
    rig.mock.st().fail_next = Some(KeybaseError::api_message("api error"));
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert!(rig.app.conversations.is_empty());
    assert!(
        matches!(&rig.app.action_state, ActionState::Error(s) if s == "api error"),
        "got {:?}",
        rig.app.action_state
    );
}

#[test]
fn silent_load_inbox_leaves_action_state_idle_on_success() {
    // Auto-refresh contract: a successful silent refresh must NOT
    // flash a "Loaded N" banner — the user is reading the inbox and
    // does not need to be interrupted every `inbox_refresh_secs`.
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox_silent(&mut rig.app);
    // Silent refresh runs on the background lane (bg_inflight, not the
    // user's in_flight slot), so drain its single response directly.
    assert!(rig.app.bg_inflight);
    pump_one(&mut rig.app);
    assert!(!rig.app.bg_inflight);
    assert!(matches!(rig.app.action_state, ActionState::Idle));
    assert_eq!(rig.app.conversations.len(), 1);
    // cmd_log entry is written either way — the user can still
    // audit the refresh history if curious.
    assert!(
        rig.app
            .cmd_log
            .iter()
            .any(|e| e.ok && e.cmd == "keybase chat api list"),
        "successful refresh must leave a cmd_log entry"
    );
}

#[test]
fn silent_load_inbox_still_flashes_error_on_failure() {
    // The other half of the contract: silent ≠ swallow. A network
    // blip surfaces an Error banner so the user notices a wedged
    // refresh rather than slowly accumulating a stale inbox.
    let mut rig = build_rig();
    rig.mock.st().fail_next = Some(KeybaseError::api_message("net down"));
    request_load_inbox_silent(&mut rig.app);
    pump_one(&mut rig.app);
    assert!(!rig.app.bg_inflight);
    match &rig.app.action_state {
        ActionState::Error(s) => assert_eq!(s, "net down"),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn load_inbox_surfaces_skipped_rows_as_warnings_but_keeps_good_ones() {
    let mut rig = build_rig();
    // Two healthy rows + a synthetic "skipped" diagnostic — what
    // the adapter would emit if one row failed to decode.
    rig.mock.st().conversations = vec![
        conv("c1", "alice,bob", MembersType::ImpTeamNative),
        conv("c2", "team1", MembersType::Team),
    ];
    rig.mock.st().conversations_skipped = vec!["conversation #2: missing id".to_string()];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Good rows must load — the bad one must not block them.
    assert_eq!(rig.app.conversations.len(), 2);
    // Default filter is All → both good rows are in the filtered view.
    assert_eq!(rig.app.filtered_cache.len(), 2);
    // The feedback strip stays Done (load succeeded) but mentions
    // the count of skipped rows.
    match &rig.app.action_state {
        ActionState::Done(s) => assert!(
            s.contains("1 skipped"),
            "feedback should mention skipped count, got: {s}"
        ),
        other => panic!("expected Done, got {other:?}"),
    }
    // The cmd log must contain the per-row diagnostic so the user
    // can see *what* failed.
    let log_has_diag = rig
        .app
        .cmd_log
        .iter()
        .any(|e| !e.ok && e.detail.contains("conversation #2"));
    assert!(
        log_has_diag,
        "expected diag in cmd_log: {:?}",
        rig.app.cmd_log
    );
}

// ── do_load_messages → request_load_messages ─────────────────────────

#[test]
fn load_messages_reverses_to_chronological_and_pins_to_bottom() {
    let mut rig = build_rig();
    rig.mock.st().messages = vec![
        text_msg(3, "me", "third"),
        text_msg(2, "me", "second"),
        text_msg(1, "me", "first"),
    ];
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    request_load_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.messages.len(), 3);
    assert_eq!(rig.app.messages[0].id, 1);
    assert_eq!(rig.app.messages[2].id, 3);
    assert_eq!(rig.app.messages_scroll, 0);
}

#[test]
fn load_messages_remembers_next_cursor() {
    let mut rig = build_rig();
    rig.mock.st().messages = vec![text_msg(1, "me", "x")];
    rig.mock.st().messages_next = Some("WX==".into());
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    request_load_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.messages_next.as_deref(), Some("WX=="));
}

#[test]
fn control_op_reread_keeps_scroll_position() {
    let mut rig = build_rig();
    rig.mock.st().messages = vec![text_msg(1, "me", "x")];
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    // Reader has scrolled up; a delete/edit/react re-read sets the flag.
    rig.app.messages_scroll = 7;
    rig.app.preserve_msg_scroll = true;
    request_load_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.messages_scroll, 7, "scroll preserved");
    assert!(!rig.app.preserve_msg_scroll, "flag consumed");
}

#[test]
fn fresh_read_snaps_to_latest() {
    let mut rig = build_rig();
    rig.mock.st().messages = vec![text_msg(1, "me", "x")];
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages_scroll = 7;
    // No preserve flag → a plain read snaps to the bottom.
    request_load_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.messages_scroll, 0);
}

#[test]
fn load_messages_without_open_conv_id_errors() {
    let mut rig = build_rig();
    request_load_messages(&mut rig.app);
    // No worker request was queued; nothing to pump.
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

// ── do_load_older_messages → request_load_older_messages ─────────────

#[test]
fn load_older_messages_prepends_in_chronological_order() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(4, "me", "fourth"), text_msg(5, "me", "fifth")];
    rig.app.messages_next = Some("CURSOR-A".into());

    rig.mock.st().messages = vec![
        text_msg(3, "me", "third"),
        text_msg(2, "me", "second"),
        text_msg(1, "me", "first"),
    ];
    rig.mock.st().messages_next = Some("CURSOR-B".into());

    request_load_older_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);

    let ids: Vec<u64> = rig.app.messages.iter().map(|m| m.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5], "got {ids:?}");
    assert_eq!(rig.app.messages_next.as_deref(), Some("CURSOR-B"));
    assert!(!rig.app.messages_loading_older);
    // The adapter must have been called with the original cursor.
    assert_eq!(
        rig.mock.st().read_calls.last().cloned(),
        Some(Some("CURSOR-A".into()))
    );
}

#[test]
fn load_older_messages_no_op_when_cursor_is_none() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(1, "me", "x")];
    rig.app.messages_next = None;
    request_load_older_messages(&mut rig.app);
    // No worker request queued — pump must not block.
    assert!(rig.app.in_flight.is_none());
    assert_eq!(rig.app.messages.len(), 1, "messages must not change");
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
}

// ── do_send_message → request_send_message ───────────────────────────

#[test]
fn send_message_rejects_empty_buffer() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.compose.clear();
    request_send_message(&mut rig.app);
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert!(rig.mock.st().sent.is_empty());
}

#[test]
fn send_message_failure_preserves_draft_and_reply_context() {
    // Regression guard: a transient network failure must not eat the
    // user's typed message. With the optimistic outbox, the compose is
    // cleared at send time and the body is parked in the outbox as a
    // Failed entry (with its reply context) that `Alt+R` can resend.
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.reply_to_id = Some(42);
    rig.app.compose.set("important draft");
    rig.mock.st().fail_next = Some(KeybaseError::api_message("network unreachable"));
    request_send_message(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Compose is cleared at send time; the draft survives in the outbox.
    assert_eq!(rig.app.compose.text(), "");
    assert_eq!(rig.app.outbox.len(), 1);
    assert_eq!(rig.app.outbox[0].state, crate::tui::app::SendState::Failed);
    assert_eq!(rig.app.outbox[0].body, "important draft");
    assert_eq!(rig.app.outbox[0].reply_to, Some(42));
    // Error visible on the feedback strip.
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn resend_failed_message_clears_the_outbox_on_success() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.compose.set("retry me");
    rig.mock.st().fail_next = Some(KeybaseError::api_message("boom"));
    request_send_message(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.outbox.len(), 1);
    assert_eq!(rig.app.outbox[0].state, crate::tui::app::SendState::Failed);
    // Resend — the mock no longer fails, so it goes through and the
    // reconciling re-read prunes the now-Delivered bubble.
    request_resend_message(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert!(rig.app.outbox.is_empty());
}

#[test]
fn upload_attachment_sends_the_file_and_reloads() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    request_upload_attachment(&mut rig.app, std::path::PathBuf::from("/tmp/photo.png"));
    pump_until_idle(&mut rig.app);
    let uploads = rig.mock.st().uploads.clone();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].0, "/tmp/photo.png");
    // Success chains a re-read, which lands as Done (no error).
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
}

#[test]
fn reaction_events_are_dropped_from_the_message_stream() {
    use crate::domain::MessageContent;
    // A standalone reaction event is collapsed onto its target (via the
    // target's `reactions` field), so it shouldn't survive as its own row.
    let mut reaction = text_msg(200, "alice", "");
    reaction.content = MessageContent::Reaction {
        target_id: 91,
        body: ":+1:".into(),
    };
    let kept = project_messages(vec![text_msg(1, "alice", "hi"), reaction]);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].id, 1);
}

#[test]
fn send_message_clears_buffer_records_call_and_queues_reload() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.compose.set("hola");
    request_send_message(&mut rig.app);
    // After the send completes the handler chains a load-messages.
    // pump_one ⇒ send-response applied; in_flight should now hold
    // LoadMessages.
    pump_one(&mut rig.app);
    assert!(rig.app.compose.is_empty());
    assert!(matches!(rig.app.in_flight, Some(InFlight::LoadMessages)));
    let st = rig.mock.st();
    assert_eq!(st.sent.len(), 1);
    assert_eq!(st.sent[0].1, "hola");
}

// ── do_save_edit → request_save_edit ─────────────────────────────────

#[test]
fn save_edit_requires_target_id() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.compose.set("x");
    rig.app.edit_target_id = None;
    request_save_edit(&mut rig.app);
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert!(rig.mock.st().edits.is_empty());
}

#[test]
fn save_edit_failure_preserves_buffer_and_target() {
    // Symmetry with `send_message_failure_preserves_draft_*`:
    // a failed edit must keep `compose_buffer` and `edit_target_id`
    // intact so the user can retry without re-typing the body or
    // re-selecting the target message.
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.compose.set("corrected body");
    rig.app.edit_target_id = Some(99);
    rig.mock.st().fail_next = Some(KeybaseError::api_message("server rejected"));
    request_save_edit(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.compose.text(), "corrected body");
    assert_eq!(rig.app.edit_target_id, Some(99));
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn save_edit_calls_adapter_clears_buffer_and_target() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.compose.set("fixed");
    rig.app.edit_target_id = Some(42);
    request_save_edit(&mut rig.app);
    pump_one(&mut rig.app);
    assert!(rig.app.compose.is_empty());
    assert!(rig.app.edit_target_id.is_none());
    let st = rig.mock.st();
    assert_eq!(st.edits, vec![(42, "fixed".to_string())]);
    drop(st);
    assert!(matches!(rig.app.in_flight, Some(InFlight::LoadMessages)));
}

// ── do_delete_selected_message → request_delete_selected_message ─────

#[test]
fn delete_selected_calls_adapter_with_correct_id() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.identity.username = "me".into();
    rig.app.messages = vec![text_msg(7, "me", "doomed")];
    rig.app.selected_msg_idx = Some(0);
    request_delete_selected_message(&mut rig.app);
    pump_one(&mut rig.app);
    assert_eq!(rig.mock.st().deletes, vec![7]);
    // Stays in Select mode after deleting (you can delete more).
    assert_eq!(rig.app.selected_msg_idx, Some(0));
    assert!(rig.app.pending_batch.is_none());
}

#[test]
fn input_shift_x_in_select_opens_delete_confirm() {
    // Gradient: in Select mode the destructive delete is Shift+X (Char('X')),
    // matching Shift-remove in the channel browser / members — not bare `d`.
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.identity.username = "me".into();
    rig.app.messages = vec![text_msg(7, "me", "doomed")];
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Chat;
    rig.app.selected_msg_idx = Some(0);
    press(&mut rig.app, KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert_eq!(rig.app.screen, Screen::ConfirmDeleteMessage);
    // A bare `d` must NOT delete anymore (it's inert in Select).
    rig.app.screen = Screen::Inbox;
    press(&mut rig.app, KeyCode::Char('d'), KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Inbox);
}

#[test]
fn delete_acts_on_the_whole_marked_selection_sequentially() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.identity.username = "me".into();
    rig.app.messages = vec![
        text_msg(10, "me", "a"),
        text_msg(11, "me", "b"),
        text_msg(12, "me", "c"),
    ];
    rig.app.rebuild_msg_meta();
    // Mark all three (by id); the batch must delete every one, not just the
    // cursor.
    rig.app.selected_msg_idx = Some(2);
    rig.app.msg_marks = [10u64, 11, 12].into_iter().collect();
    request_delete_selected_message(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let mut deleted = rig.mock.st().deletes.clone();
    deleted.sort_unstable();
    assert_eq!(
        deleted,
        vec![10, 11, 12],
        "all marked deleted, not just cursor"
    );
    // Shading cleared, still in Select mode.
    assert!(rig.app.msg_marks.is_empty());
    assert!(rig.app.pending_batch.is_none());
}

#[test]
fn delete_skips_other_peoples_messages() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.identity.username = "me".into();
    rig.app.messages = vec![text_msg(20, "me", "mine"), text_msg(21, "alice", "theirs")];
    rig.app.rebuild_msg_meta();
    rig.app.selected_msg_idx = Some(0);
    rig.app.msg_marks = [20u64, 21].into_iter().collect();
    request_delete_selected_message(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Only the own message is deleted; alice's is left alone.
    assert_eq!(rig.mock.st().deletes, vec![20]);
}

#[test]
fn marks_survive_a_reprojecting_reload_by_id() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.identity.username = "me".into();
    rig.app.messages = vec![
        text_msg(1, "alice", "old"),
        text_msg(2, "me", "keep-a"),
        text_msg(3, "me", "keep-b"),
    ];
    rig.app.rebuild_msg_meta();
    rig.app.selected_msg_idx = Some(2); // cursor on id 3
    rig.app.msg_marks = [2u64, 3].into_iter().collect();
    // A remote delete removed message 1 — the re-read returns a list where
    // every index shifted down by one (mock stores newest-first; the read
    // handler reverses).
    rig.mock.st().messages = vec![text_msg(3, "me", "keep-b"), text_msg(2, "me", "keep-a")];
    rig.app.preserve_msg_scroll = true;
    request_load_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Marks still denote the same messages (ids, not the shifted indices)…
    let want: std::collections::HashSet<u64> = [2u64, 3].into_iter().collect();
    assert_eq!(rig.app.msg_marks, want);
    // …and the cursor re-found its message (id 3) at its new index.
    assert_eq!(rig.app.selected_msg_idx, Some(1));

    // A mark whose message vanished is pruned on the next reload.
    rig.mock.st().messages = vec![text_msg(2, "me", "keep-a")];
    rig.app.preserve_msg_scroll = true;
    request_load_messages(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let want: std::collections::HashSet<u64> = [2u64].into_iter().collect();
    assert_eq!(rig.app.msg_marks, want);
}

// ── do_send_reaction → request_send_reaction ─────────────────────────

#[test]
fn react_with_empty_query_sends_the_highlighted_emoji() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(5, "me", "x")];
    rig.app.selected_msg_idx = Some(0);
    rig.app.react.clear();
    rig.app.react_selected = 0;
    // The picker always has a highlighted emoji (top of the seeded standard
    // set); stock emojis send their raw glyph, so Enter on an empty query
    // reacts with that glyph (Discord-style) rather than erroring.
    let expected = rig
        .app
        .emojis
        .first()
        .expect("seeded emojis")
        .display
        .clone();
    request_send_reaction(&mut rig.app);
    pump_one(&mut rig.app);
    assert_eq!(rig.mock.st().reactions, vec![(5, expected)]);
}

#[test]
fn react_failure_surfaces_error_and_stays_in_select() {
    // Committing a reaction closes the picker and runs it over the selection;
    // a failure surfaces an error and keeps the reader in Select mode (cursor
    // preserved) so they can retry from the picker.
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(5, "me", "x")];
    rig.mock.st().messages = vec![text_msg(5, "me", "x")]; // reload returns it
    rig.app.selected_msg_idx = Some(0);
    rig.app.react.set(":fire:");
    rig.app.screen = Screen::React;
    rig.mock.st().fail_next = Some(KeybaseError::api_message("reaction not allowed"));
    request_send_reaction(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.selected_msg_idx, Some(0));
    assert!(rig.app.pending_batch.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn react_sends_with_correct_msg_id_and_body() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(5, "me", "x")];
    rig.app.selected_msg_idx = Some(0);
    rig.app.react.set(":+1:");
    request_send_reaction(&mut rig.app);
    pump_one(&mut rig.app);
    let st = rig.mock.st();
    assert_eq!(st.reactions, vec![(5, ":+1:".to_string())]);
}

// ── local mute (no Keybase call) ──────────────────────────────────────

#[test]
fn local_mute_toggles_without_keybase_and_suppresses_unread() {
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app);
    // Mark it unread for the test.
    rig.app.conversations[0].unread = true;
    let id = rig.app.conversations[0].id.clone();
    assert_eq!(rig.app.unread_total(), 1);

    // Toggle mute — purely local, must NOT issue a Keybase setstatus.
    toggle_muted_conversation(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert!(
        rig.mock.st().statuses.is_empty(),
        "local mute must not call setstatus: {:?}",
        rig.mock.st().statuses
    );
    assert!(rig.app.is_muted(&id));
    // Muted → no longer counts as unread.
    assert_eq!(rig.app.unread_total(), 0);

    // Toggle off → unread again.
    toggle_muted_conversation(&mut rig.app);
    assert!(!rig.app.is_muted(&id));
    assert_eq!(rig.app.unread_total(), 1);
}

#[test]
fn failed_empty_inbox_load_records_error_and_success_clears_it() {
    let mut rig = build_rig();
    // A failed load with nothing already shown records the error for the
    // persistent "couldn't load, retry" panel.
    handle_load_inbox_response(
        &mut rig.app,
        Err(KeybaseError::Timeout {
            label: "keybase".into(),
            secs: 30,
        }),
        false,
    );
    assert!(rig.app.inbox_error.is_some());
    // A later successful load clears it.
    handle_load_inbox_response(
        &mut rig.app,
        Ok(ListConversationsOk {
            conversations: vec![],
            skipped: vec![],
        }),
        false,
    );
    assert!(rig.app.inbox_error.is_none());
}

// ── channel browser (listconvsonname / join) ─────────────────────────

#[test]
fn channel_browser_loads_sorts_joined_first_and_joins() {
    let mut rig = build_rig();
    let mut general = conv("c-general", "phoenix", MembersType::Team);
    general.channel.topic_name = Some("general".into());
    general.member_status = MemberStatus::Active; // joined
    let mut random = conv("c-random", "phoenix", MembersType::Team);
    random.channel.topic_name = Some("random".into());
    random.member_status = MemberStatus::Left; // not joined
    // Return them not-joined-first so the sort is observable.
    rig.mock.st().channels = vec![random, general];

    rig.app.channel_browser_team = Some("phoenix".into());
    request_load_channels(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.channels.len(), 2);
    // Joined channel floats to the top.
    assert_eq!(
        rig.app.channels[0].channel.topic_name.as_deref(),
        Some("general")
    );

    // Select the not-joined "random" and activate → join it.
    let ri = rig
        .app
        .channels
        .iter()
        .position(|c| c.channel.topic_name.as_deref() == Some("random"))
        .unwrap();
    rig.app.channel_selected = ri;
    channel_browser_activate(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.mock.st().joined, vec!["random".to_string()]);
}

#[test]
fn channel_browser_create_fires_newconv_and_exits_create_mode() {
    let mut rig = build_rig();
    rig.app.channel_browser_team = Some("phoenix".into());
    open_channel_create(&mut rig.app);
    assert!(rig.app.channel_creating);
    rig.app.channel_new_name.set("announcements");
    request_create_channel(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // newconv fired on the team (the channel's team name).
    assert!(rig.mock.st().new_convs.contains(&"phoenix".to_string()));
    // Create mode exits on success.
    assert!(!rig.app.channel_creating);
}

#[test]
fn channel_browser_rename_calls_adapter() {
    let mut rig = build_rig();
    let mut general = conv("c-general", "phoenix", MembersType::Team);
    general.channel.topic_name = Some("general".into());
    general.member_status = MemberStatus::Active;
    rig.mock.st().channels = vec![general];
    rig.app.channel_browser_team = Some("phoenix".into());
    request_load_channels(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Enter rename mode, type a new name, submit.
    open_channel_rename(&mut rig.app);
    assert_eq!(rig.app.channel_renaming.as_deref(), Some("general"));
    rig.app.channel_new_name.set("lobby");
    request_rename_channel(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(
        rig.mock.st().renames,
        vec![(
            "phoenix".to_string(),
            "general".to_string(),
            "lobby".to_string()
        )]
    );
    assert!(rig.app.channel_renaming.is_none());
}

#[test]
fn channel_browser_delete_needs_confirm_then_calls_adapter() {
    let mut rig = build_rig();
    let mut random = conv("c-random", "phoenix", MembersType::Team);
    random.channel.topic_name = Some("random".into());
    random.member_status = MemberStatus::Active;
    rig.mock.st().channels = vec![random];
    rig.app.channel_browser_team = Some("phoenix".into());
    request_load_channels(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // `d` opens the inline confirm — nothing deleted yet.
    open_channel_delete_confirm(&mut rig.app);
    assert_eq!(rig.app.channel_confirm_delete.as_deref(), Some("random"));
    assert!(rig.mock.st().deleted_channels.is_empty());
    // Confirm → delete fires.
    confirm_channel_delete(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.mock.st().deleted_channels, vec!["random".to_string()]);
    assert!(rig.app.channel_confirm_delete.is_none());
}

#[test]
fn channel_browser_toggle_default_sends_full_set() {
    let mut rig = build_rig();
    let mut general = conv("c-general", "phoenix", MembersType::Team);
    general.channel.topic_name = Some("general".into());
    general.member_status = MemberStatus::Active;
    let mut random = conv("c-random", "phoenix", MembersType::Team);
    random.channel.topic_name = Some("random".into());
    random.member_status = MemberStatus::Active;
    rig.mock.st().channels = vec![general, random];
    rig.app.channel_browser_team = Some("phoenix".into());
    request_load_channels(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // The chained default-channels get ran (mock returns none → general only).
    assert!(rig.app.default_channels.is_empty());
    // Toggle "random" ON → SET fires with the full new set [random].
    let ri = rig
        .app
        .channels
        .iter()
        .position(|c| c.channel.topic_name.as_deref() == Some("random"))
        .unwrap();
    rig.app.channel_selected = ri;
    toggle_default_channel(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(
        rig.mock.st().default_channels_set,
        vec![vec!["random".to_string()]]
    );
    assert_eq!(rig.app.default_channels, vec!["random".to_string()]);
}

// ── members (listmembers / addtochannel / removefromchannel) ─────────

fn open_members_rig() -> Rig {
    let mut rig = build_rig();
    rig.mock.st().members = vec![
        ChatMember {
            username: "zoe".into(),
            role: TeamRole::Writer,
        },
        ChatMember {
            username: "alice".into(),
            role: TeamRole::Owner,
        },
    ];
    rig.app.members_channel = Some(ReadChannel {
        name: "phoenix".into(),
        members_type: "team".into(),
        topic_name: Some("general".into()),
    });
    request_load_members(&mut rig.app);
    pump_until_idle(&mut rig.app);
    rig
}

#[test]
fn members_load_sorts_by_role_then_name() {
    let rig = open_members_rig();
    // Owner (alice) before Writer (zoe) regardless of input order.
    assert_eq!(rig.app.members.len(), 2);
    assert_eq!(rig.app.members[0].username, "alice");
    assert_eq!(rig.app.members[0].role, TeamRole::Owner);
    assert_eq!(rig.app.members[1].username, "zoe");
}

#[test]
fn members_add_parses_usernames_and_calls_adapter() {
    let mut rig = open_members_rig();
    open_member_add(&mut rig.app);
    assert!(rig.app.member_adding);
    rig.app.member_add_input.set("Bob, charlie");
    request_add_members(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(
        rig.mock.st().added_members,
        vec![vec!["bob".to_string(), "charlie".to_string()]]
    );
    assert!(!rig.app.member_adding);
}

#[test]
fn members_add_rejects_invalid_username() {
    let mut rig = open_members_rig();
    open_member_add(&mut rig.app);
    rig.app.member_add_input.set("not a valid!!name");
    request_add_members(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Nothing dispatched — the invalid username short-circuits.
    assert!(rig.mock.st().added_members.is_empty());
}

#[test]
fn members_remove_needs_confirm_then_calls_adapter() {
    let mut rig = open_members_rig();
    rig.app.members_selected = 0; // alice
    open_member_remove_confirm(&mut rig.app);
    assert_eq!(rig.app.member_confirm_remove.as_deref(), Some("alice"));
    assert!(rig.mock.st().removed_members.is_empty());
    confirm_remove_member(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(
        rig.mock.st().removed_members,
        vec![vec!["alice".to_string()]]
    );
    assert!(rig.app.member_confirm_remove.is_none());
}

// ── do_create_new_conversation → request_create_new_conversation ─────

#[test]
fn new_conversation_inserts_own_username() {
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.new_conv.set("bob, charlie");
    request_create_new_conversation(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let st = rig.mock.st();
    assert_eq!(st.new_convs, vec!["alice,bob,charlie".to_string()]);
}

#[test]
fn new_conversation_rejects_empty_input() {
    let mut rig = build_rig();
    rig.app.new_conv.set("   ");
    request_create_new_conversation(&mut rig.app);
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert!(rig.mock.st().new_convs.is_empty());
}

#[test]
fn new_conversation_rejects_invalid_usernames_before_dispatch() {
    // Garbage in the username field used to reach the worker and
    // come back with a cryptic server error. Now we filter it
    // client-side and surface a clean message.
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.new_conv.set("bob, not.a.user, ; rm -rf /");
    request_create_new_conversation(&mut rig.app);
    assert!(rig.app.in_flight.is_none(), "must not queue a worker call");
    match &rig.app.action_state {
        ActionState::Error(s) => {
            assert!(s.starts_with("Invalid username(s):"), "got: {s}");
            assert!(s.contains("not.a.user"));
            assert!(s.contains("; rm -rf /"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
    assert!(rig.mock.st().new_convs.is_empty());
}

#[test]
fn new_conversation_accepts_proof_identity() {
    // `alice@twitter`-style proof identities must pass validation
    // without round-tripping a Keybase error.
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.new_conv.set("bob@twitter, charlie@reddit");
    request_create_new_conversation(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let st = rig.mock.st();
    assert_eq!(st.new_convs.len(), 1);
    assert_eq!(st.new_convs[0], "alice,bob@twitter,charlie@reddit");
}

// ── do_search_inbox_remote → request_search_inbox_remote ─────────────

#[test]
fn search_remote_rejects_empty_query() {
    let mut rig = build_rig();
    rig.app.search_global_input.clear();
    request_search_inbox_remote(&mut rig.app);
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert!(rig.app.search_global_results.is_empty());
}

#[test]
fn search_remote_populates_results() {
    let mut rig = build_rig();
    rig.mock.st().search_hits = vec![
        InboxHit {
            conv_id: "c1".into(),
            conv_name: "alice".into(),
            message_id: 1,
            sender: "alice".into(),
            body_summary: "hola".into(),
            sent_at: 0,
        },
        InboxHit {
            conv_id: "c2".into(),
            conv_name: "bob".into(),
            message_id: 2,
            sender: "bob".into(),
            body_summary: "chau".into(),
            sent_at: 0,
        },
    ];
    rig.app.search_global_input.set("hola");
    request_search_inbox_remote(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.search_global_results.len(), 2);
    assert_eq!(rig.app.search_global_selected, 0);
}

// ── open_selected_search_result ───────────────────────────────────────

#[test]
fn open_search_result_jumps_to_conversation() {
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    rig.app.search_global_results = vec![InboxHit {
        conv_id: "c1".into(),
        conv_name: "alice".into(),
        message_id: 1,
        sender: "alice".into(),
        body_summary: "x".into(),
        sent_at: 0,
    }];
    rig.app.search_global_selected = 0;
    open_selected_search_result(&mut rig.app);
    // The unified Home opens the chat in-place (stays on the inbox screen).
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert_eq!(rig.app.open_conv_id.as_deref(), Some("c1"));
    // The open chains a request_load_messages call.
    assert!(matches!(rig.app.in_flight, Some(InFlight::LoadMessages)));
}

#[test]
fn open_search_result_errors_when_conv_not_in_cache() {
    let mut rig = build_rig();
    rig.app.search_global_results = vec![InboxHit {
        conv_id: "nope".into(),
        conv_name: "?".into(),
        message_id: 0,
        sender: "?".into(),
        body_summary: "?".into(),
        sent_at: 0,
    }];
    open_selected_search_result(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    // The conversation isn't in the cached inbox, so nothing was opened.
    assert!(rig.app.open_conv_id.is_none());
}

// ── pin / unpin ───────────────────────────────────────────────────────

#[test]
fn pin_uses_selected_message_id() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(99, "me", "important")];
    rig.app.selected_msg_idx = Some(0);
    request_pin_selected_message(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.mock.st().pins, vec![99]);
}

#[test]
fn unpin_calls_adapter_once() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    request_unpin_conversation(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.mock.st().unpins, 1);
}

#[test]
fn tree_groups_dms_then_teams_with_conversations_nested() {
    use crate::tui::app::TreeRow;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![
            conv("d", "alice", MembersType::ImpTeamNative),
            conv("t1", "globex", MembersType::Team),
            conv("t2", "acme", MembersType::Team),
        ],
        "d",
    );
    let labels: Vec<String> = rig
        .app
        .tree_rows()
        .iter()
        .map(|r| match r {
            TreeRow::Group { label, .. } => format!("[{label}]"),
            TreeRow::Conv { idx } => rig.app.conversations[*idx].id.clone(),
        })
        .collect();
    // DMs group + its conv, then teams (alpha) each with their conv.
    assert_eq!(
        labels,
        vec!["[Direct messages]", "d", "[acme]", "t2", "[globex]", "t1"]
    );
}

#[test]
fn collapsed_group_hides_its_conversations() {
    use crate::tui::app::TreeRow;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![
            conv("d", "alice", MembersType::ImpTeamNative),
            conv("t", "acme", MembersType::Team),
        ],
        "d",
    );
    // Collapse the DMs group → its conversation disappears from the rows.
    rig.app.toggle_collapsed(crate::tui::app::App::DMS_KEY);
    let has_dm_conv = rig
        .app
        .tree_rows()
        .iter()
        .any(|r| matches!(r, TreeRow::Conv { idx } if rig.app.conversations[*idx].id == "d"));
    assert!(!has_dm_conv);
    // Toggling back reveals it again.
    rig.app.toggle_collapsed(crate::tui::app::App::DMS_KEY);
    let has_dm_conv = rig
        .app
        .tree_rows()
        .iter()
        .any(|r| matches!(r, TreeRow::Conv { idx } if rig.app.conversations[*idx].id == "d"));
    assert!(has_dm_conv);
}

#[test]
fn tree_forward_expands_but_never_collapses() {
    use crate::tui::app::TreeRow;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("t1", "acme", MembersType::Team)],
        "t1",
    );
    // Cursor on the team group header (collapse it first: preload expands).
    rig.app.toggle_collapsed("acme");
    let group_pos = rig
        .app
        .tree_rows()
        .iter()
        .position(|r| matches!(r, TreeRow::Group { key, .. } if key == "acme"))
        .expect("group row");
    rig.app.tree_selected = group_pos;
    let collapsed_len = rig.app.tree_rows().len();
    // → on a collapsed group expands it…
    tree_forward(&mut rig.app);
    assert!(rig.app.tree_rows().len() > collapsed_len, "expanded");
    // …and → on an expanded group is a no-op (never collapses — the
    // documented "l can't loop open↔closed" invariant).
    let expanded_len = rig.app.tree_rows().len();
    rig.app.tree_selected = group_pos;
    tree_forward(&mut rig.app);
    assert_eq!(rig.app.tree_rows().len(), expanded_len, "no collapse on →");
}

#[test]
fn tree_back_collapses_expanded_group() {
    use crate::tui::app::TreeRow;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("t1", "acme", MembersType::Team)],
        "t1",
    );
    let group_pos = rig
        .app
        .tree_rows()
        .iter()
        .position(|r| matches!(r, TreeRow::Group { key, .. } if key == "acme"))
        .expect("group row");
    rig.app.tree_selected = group_pos;
    let expanded_len = rig.app.tree_rows().len();
    tree_back(&mut rig.app);
    assert!(rig.app.tree_rows().len() < expanded_len, "collapsed");
}

#[test]
fn incoming_push_keeps_tree_cursor_on_selected_conversation() {
    use crate::tui::app::TreeRow;
    let mut rig = build_rig();
    let mut bumped = conv("bumped", "alice", MembersType::ImpTeamNative);
    bumped.active_at_ms = 1000;
    let mut selected = conv("selected", "bob", MembersType::ImpTeamNative);
    selected.active_at_ms = 2000; // most recent → first row before the push
    preload_inbox(&mut rig.app, &rig.mock, vec![bumped, selected], "selected");
    assert_eq!(
        rig.app.selected_conversation().map(|c| c.id.clone()),
        Some("selected".to_string())
    );
    // A push for the *other* conversation re-sorts it to the top…
    let mut m = text_msg(9, "alice", "bump");
    m.sent_at_ms = 5000;
    handle_incoming_message(&mut rig.app, "bumped".into(), m);
    // …but the cursor stays on the conversation the user had selected
    // (a recency re-sort must not yank the tree selection).
    assert_eq!(
        rig.app.selected_conversation().map(|c| c.id.clone()),
        Some("selected".to_string())
    );
    // And the bumped conversation did move to the first conversation row.
    let first_conv = rig.app.tree_rows().iter().find_map(|r| match r {
        TreeRow::Conv { idx } => Some(rig.app.conversations[*idx].id.clone()),
        _ => None,
    });
    assert_eq!(first_conv, Some("bumped".to_string()));
}

// ── select mode ───────────────────────────────────────────────────────

#[test]
fn enter_select_anchors_to_last_message() {
    let mut rig = build_rig();
    rig.app.messages = vec![
        text_msg(1, "me", "a"),
        text_msg(2, "me", "b"),
        text_msg(3, "me", "c"),
    ];
    enter_select_mode(&mut rig.app);
    assert_eq!(rig.app.selected_msg_idx, Some(2));
    assert!(!rig.app.compose_open);
}

#[test]
fn enter_select_errors_on_empty_history() {
    let mut rig = build_rig();
    rig.app.messages.clear();
    enter_select_mode(&mut rig.app);
    assert!(rig.app.selected_msg_idx.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn select_move_up_clamps_at_zero() {
    let mut rig = build_rig();
    rig.app.messages = vec![text_msg(1, "me", "a"), text_msg(2, "me", "b")];
    rig.app.selected_msg_idx = Some(0);
    select_move_up(&mut rig.app);
    assert_eq!(rig.app.selected_msg_idx, Some(0));
}

#[test]
fn select_move_down_clamps_at_last() {
    let mut rig = build_rig();
    rig.app.messages = vec![text_msg(1, "me", "a"), text_msg(2, "me", "b")];
    rig.app.selected_msg_idx = Some(1);
    select_move_down(&mut rig.app);
    assert_eq!(rig.app.selected_msg_idx, Some(1));
}

#[test]
fn leave_select_restores_compose_focus() {
    let mut rig = build_rig();
    rig.app.messages = vec![text_msg(1, "me", "x")];
    enter_select_mode(&mut rig.app);
    leave_select_mode(&mut rig.app);
    assert!(rig.app.selected_msg_idx.is_none());
    assert!(rig.app.compose_open);
}

// ── message-action permission gates ───────────────────────────────────

#[test]
fn open_edit_refuses_non_own_message() {
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.messages = vec![text_msg(1, "bob", "yours")];
    rig.app.selected_msg_idx = Some(0);
    open_edit_for_selected(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert!(rig.app.edit_target_id.is_none());
}

#[test]
fn open_edit_loads_body_into_compose_buffer() {
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.messages = vec![text_msg(1, "alice", "hola mundo")];
    rig.app.selected_msg_idx = Some(0);
    open_edit_for_selected(&mut rig.app);
    assert_eq!(rig.app.edit_target_id, Some(1));
    assert_eq!(rig.app.compose.text(), "hola mundo");
    assert!(rig.app.compose_open);
    assert!(rig.app.selected_msg_idx.is_none());
}

#[test]
fn open_delete_refuses_non_own_message() {
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.messages = vec![text_msg(1, "bob", "yours")];
    rig.app.selected_msg_idx = Some(0);
    open_delete_for_selected(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert_ne!(rig.app.screen, Screen::ConfirmDeleteMessage);
}

#[test]
fn cancel_edit_clears_target_and_buffer() {
    let mut rig = build_rig();
    rig.app.compose.set("draft");
    rig.app.edit_target_id = Some(7);
    cancel_edit(&mut rig.app);
    assert!(rig.app.compose.is_empty());
    assert!(rig.app.edit_target_id.is_none());
}

// ── escape_conversation semantics ─────────────────────────────────────

#[test]
fn escape_clears_non_empty_draft_first() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.compose.set("draft");
    escape_conversation(&mut rig.app);
    assert!(rig.app.compose.is_empty());
    assert_eq!(rig.app.screen, Screen::Inbox);
}

#[test]
fn escape_on_empty_draft_closes_conversation() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.compose.clear();
    escape_conversation(&mut rig.app);
    assert_eq!(rig.app.screen, Screen::Inbox);
}

// ── pin tracking ──────────────────────────────────────────────────────

#[test]
fn rebuild_pinned_finds_latest_pin_in_history() {
    let mut rig = build_rig();
    rig.app.messages = vec![
        text_msg(1, "alice", "first"),
        Message {
            id: 2,
            sender: "bob".into(),
            device: "d".into(),
            sent_at: 0,
            sent_at_ms: 0,
            content: MessageContent::Pin { target_id: 1 },
            reactions: Vec::new(),
            reply_to: None,
            edited: false,
            mentions: Vec::new(),
        },
        text_msg(3, "alice", "later"),
    ];
    rig.app.rebuild_msg_meta();
    assert_eq!(rig.app.pinned_msg_id, Some(1));
}

#[test]
fn rebuild_pinned_clears_when_target_zero() {
    let mut rig = build_rig();
    rig.app.messages = vec![
        Message {
            id: 1,
            sender: "alice".into(),
            device: "d".into(),
            sent_at: 0,
            sent_at_ms: 0,
            content: MessageContent::Pin { target_id: 42 },
            reactions: Vec::new(),
            reply_to: None,
            edited: false,
            mentions: Vec::new(),
        },
        Message {
            id: 2,
            sender: "alice".into(),
            device: "d".into(),
            sent_at: 0,
            sent_at_ms: 0,
            content: MessageContent::Pin { target_id: 0 },
            reactions: Vec::new(),
            reply_to: None,
            edited: false,
            mentions: Vec::new(),
        },
    ];
    rig.app.rebuild_msg_meta();
    assert_eq!(rig.app.pinned_msg_id, None);
}

#[test]
fn rebuild_pinned_yields_none_when_no_pin_in_history() {
    let mut rig = build_rig();
    rig.app.messages = vec![text_msg(1, "alice", "x")];
    rig.app.rebuild_msg_meta();
    assert_eq!(rig.app.pinned_msg_id, None);
}

// ── teams ─────────────────────────────────────────────────────────────

#[test]
fn do_load_teams_populates_and_clamps_selection() {
    use crate::tui::flows::teams::{open_teams, request_load_teams};
    let mut rig = build_rig();
    rig.app.identity.username = "me".into(); // list-user-memberships needs it
    rig.mock.st().teams = vec![
        TeamMembership {
            name: "phoenix".into(),
            is_implicit_team: false,
            member_count: 3,
            role: crate::domain::TeamRole::Admin,
        },
        TeamMembership {
            name: "phoenix.bots".into(),
            is_implicit_team: false,
            member_count: 1,
            role: crate::domain::TeamRole::Owner,
        },
    ];
    rig.app.teams_selected = 99;
    request_load_teams(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.teams.len(), 2);
    assert_eq!(rig.app.teams_selected, 1, "must clamp to last loaded row");
    open_teams(&mut rig.app); // sanity — toggles screen and queues a fresh load
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.screen, Screen::Teams);
    assert_eq!(rig.app.teams_selected, 0);
}

#[test]
fn load_teams_collapses_duplicate_rows() {
    // Regression: list-self-memberships returned one row per teammate, so a
    // team appeared many times. The handler must collapse to one row per team.
    use crate::tui::flows::teams::request_load_teams;
    let mut rig = build_rig();
    rig.app.identity.username = "me".into();
    let dup = |name: &str, role| TeamMembership {
        name: name.into(),
        is_implicit_team: false,
        member_count: 0,
        role,
    };
    rig.mock.st().teams = vec![
        dup("acme", crate::domain::TeamRole::Writer),
        dup("acme", crate::domain::TeamRole::Admin),
        dup("acme", crate::domain::TeamRole::Writer),
        dup("globex", crate::domain::TeamRole::Owner),
        dup("globex", crate::domain::TeamRole::Writer),
    ];
    request_load_teams(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.teams.len(), 2, "one row per team");
    // Sorted by name.
    assert_eq!(rig.app.teams[0].name, "acme");
    assert_eq!(rig.app.teams[1].name, "globex");
}

#[test]
fn load_teams_surfaces_skipped_rows_as_warnings_but_keeps_good_ones() {
    use crate::tui::flows::teams::request_load_teams;
    let mut rig = build_rig();
    rig.app.identity.username = "me".into();
    rig.mock.st().teams = vec![TeamMembership {
        name: "phoenix".into(),
        is_implicit_team: false,
        member_count: 3,
        role: crate::domain::TeamRole::Admin,
    }];
    rig.mock.st().teams_skipped = vec!["team #4: unknown role".to_string()];
    request_load_teams(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.teams.len(), 1);
    match &rig.app.action_state {
        ActionState::Done(s) => assert!(s.contains("1 skipped"), "got: {s}"),
        other => panic!("expected Done, got {other:?}"),
    }
    let log_has_diag = rig
        .app
        .cmd_log
        .iter()
        .any(|e| !e.ok && e.detail.contains("team #4"));
    assert!(log_has_diag);
}

// ── Threaded replies ──────────────────────────────────────────────────

#[test]
fn start_reply_sets_reply_to_id_and_clears_buffer() {
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.messages = vec![text_msg(42, "bob", "original")];
    rig.app.selected_msg_idx = Some(0);
    rig.app.compose.set("old draft");
    start_reply_for_selected(&mut rig.app);
    assert_eq!(rig.app.reply_to_id, Some(42));
    assert!(rig.app.compose.is_empty());
    assert!(rig.app.selected_msg_idx.is_none());
    assert!(rig.app.compose_open);
}

#[test]
fn start_reply_cancels_pending_edit() {
    let mut rig = build_rig();
    rig.app.messages = vec![text_msg(10, "alice", "x")];
    rig.app.selected_msg_idx = Some(0);
    rig.app.edit_target_id = Some(99);
    start_reply_for_selected(&mut rig.app);
    assert!(rig.app.edit_target_id.is_none(), "edit must be cancelled");
    assert_eq!(rig.app.reply_to_id, Some(10));
}

#[test]
fn send_passes_reply_to_through_to_adapter() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.reply_to_id = Some(7);
    rig.app.compose.set("replying");
    request_send_message(&mut rig.app);
    pump_one(&mut rig.app);
    let st = rig.mock.st();
    assert_eq!(st.sent.len(), 1);
    assert_eq!(st.sent[0].2, Some(7), "reply_to should be threaded");
    drop(st);
    // After a successful send the reply context must clear — the
    // `compose_clear` inside `request_send_message` already nulls it
    // before the worker even sees the call.
    assert!(rig.app.reply_to_id.is_none());
}

#[test]
fn compose_clear_also_clears_reply_context() {
    let mut rig = build_rig();
    rig.app.reply_to_id = Some(1);
    rig.app.compose.set("x");
    rig.app.compose_clear();
    assert!(rig.app.compose.is_empty());
    assert!(rig.app.reply_to_id.is_none());
}

// ── Attachment download ───────────────────────────────────────────────

fn attachment_msg(id: u64, sender: &str, filename: &str, size: u64) -> Message {
    Message {
        id,
        sender: sender.into(),
        device: "test".into(),
        sent_at: 0,
        sent_at_ms: 0,
        content: MessageContent::Attachment(AttachmentInfo {
            title: String::new(),
            filename: filename.into(),
            size,
            mime_type: "application/octet-stream".into(),
            uploaded: true,
        }),
        reactions: Vec::new(),
        reply_to: None,
        edited: false,
        mentions: Vec::new(),
    }
}

#[test]
fn open_download_requires_attachment_message() {
    let mut rig = build_rig();
    rig.app.messages = vec![text_msg(1, "alice", "not an attachment")];
    rig.app.selected_msg_idx = Some(0);
    open_download_for_selected(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert!(rig.app.file_picker.is_none());
}

#[test]
fn open_download_opens_dir_picker_with_a_download_action() {
    use crate::tui::app::PickerAction;
    let mut rig = build_rig();
    rig.app.messages = vec![attachment_msg(99, "bob", "secret.txt", 1024)];
    rig.app.selected_msg_idx = Some(0);
    open_download_for_selected(&mut rig.app);
    assert!(rig.app.file_picker.is_some(), "directory picker opened");
    match &rig.app.picker_action {
        PickerAction::Download {
            message_id,
            filename,
        } => {
            assert_eq!(*message_id, 99);
            assert_eq!(filename, "secret.txt");
        }
        _ => panic!("expected a Download action"),
    }
}

#[test]
fn request_download_to_joins_dir_and_filename_and_invokes_adapter() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    request_download_to(
        &mut rig.app,
        42,
        std::path::PathBuf::from("/tmp"),
        "out.bin".to_string(),
    );
    pump_until_idle(&mut rig.app);
    assert_eq!(
        rig.mock.st().downloads,
        vec![(42, "/tmp/out.bin".to_string())]
    );
}

// ── Quick switcher ────────────────────────────────────────────────────

#[test]
fn quick_switcher_lists_all_then_filters_by_name() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![
            conv("a", "alice", MembersType::ImpTeamNative),
            conv("b", "bob", MembersType::ImpTeamNative),
            conv("c", "carol", MembersType::Team),
        ],
        "a",
    );
    // Empty query → every conversation is a candidate.
    rig.app.switcher.clear();
    assert_eq!(rig.app.switcher_results().len(), 3);
    // A query narrows + ranks; the matching conversation comes first.
    rig.app.switcher.set("bob");
    let r = rig.app.switcher_results();
    assert!(!r.is_empty());
    assert_eq!(rig.app.conversations[r[0]].id, "b");
}

#[test]
fn search_jump_selects_matched_message_when_in_loaded_page() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("a", "alice", MembersType::ImpTeamNative)],
        "a",
    );
    rig.app.open_conv_id = Some("a".into());
    rig.app.pending_search_jump = Some(42);
    // The read handler reverses (keybase returns newest-first).
    let page = vec![
        text_msg(45, "alice", "newest"),
        text_msg(42, "alice", "match"),
        text_msg(40, "alice", "older"),
    ];
    handle_load_messages_response(&mut rig.app, Ok((page, None)));
    let idx = rig.app.selected_msg_idx.expect("a message is selected");
    assert_eq!(rig.app.messages[idx].id, 42);
    assert_eq!(rig.app.pending_search_jump, None);
    assert!(!rig.app.compose_open);
}

#[test]
fn search_jump_gives_up_when_message_absent_and_no_more_history() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("a", "alice", MembersType::ImpTeamNative)],
        "a",
    );
    rig.app.open_conv_id = Some("a".into());
    rig.app.pending_search_jump = Some(999);
    handle_load_messages_response(&mut rig.app, Ok((vec![text_msg(1, "alice", "hi")], None)));
    assert_eq!(rig.app.pending_search_jump, None);
    assert_eq!(rig.app.selected_msg_idx, None);
}

#[test]
fn conv_search_runs_searchregexp_then_jumps_to_match() {
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    open_conversation_by_id(&mut rig.app, "c1".into());
    pump_until_idle(&mut rig.app);
    // Loaded history contains the match (#7).
    rig.app.messages = vec![
        text_msg(1, "alice", "hi"),
        text_msg(7, "alice", "needle"),
        text_msg(9, "alice", "bye"),
    ];
    // searchregexp returns one hit at #7.
    rig.mock.st().search_hits = vec![InboxHit {
        conv_id: String::new(),
        conv_name: String::new(),
        message_id: 7,
        sender: "alice".into(),
        body_summary: "needle".into(),
        sent_at: 0,
    }];
    open_conv_search(&mut rig.app);
    rig.app.conv_search.set("needle");
    request_conv_search(&mut rig.app);
    pump_until_idle(&mut rig.app);
    assert_eq!(rig.app.conv_search_results.len(), 1);
    // Enter on the hit jumps to + selects the message and closes the modal.
    conv_search_jump_selected(&mut rig.app);
    let idx = rig.app.selected_msg_idx.expect("a message is selected");
    assert_eq!(rig.app.messages[idx].id, 7);
    assert_ne!(rig.app.screen, Screen::ConvSearch);
    assert!(rig.app.conv_search_results.is_empty());
}

#[test]
fn conv_search_rejects_empty_query() {
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    open_conversation_by_id(&mut rig.app, "c1".into());
    pump_until_idle(&mut rig.app);
    rig.app.conv_search.clear();
    request_conv_search(&mut rig.app);
    assert!(rig.app.conv_search_results.is_empty());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn switcher_sections_group_drafts_unread_recent_without_repeats() {
    use crate::tui::app::SwitcherRow;
    let mut rig = build_rig();
    let mut unread_conv = conv("u", "uconv", MembersType::ImpTeamNative);
    unread_conv.unread = true;
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![
            conv("a", "alice", MembersType::ImpTeamNative),
            conv("b", "bob", MembersType::ImpTeamNative),
            unread_conv,
        ],
        "a",
    );
    rig.app.drafts.insert("a".into(), "wip".into());
    rig.app.switcher.clear();
    let rows = rig.app.switcher_rows();
    let headers: Vec<&str> = rows
        .iter()
        .filter_map(|r| match r {
            SwitcherRow::Header(h) => Some(*h),
            _ => None,
        })
        .collect();
    assert!(headers.contains(&"Drafts"));
    assert!(headers.contains(&"Unread"));
    assert!(headers.contains(&"Recent"));
    // Every conversation is listed exactly once across the sections.
    let sel = rig.app.switcher_selectable();
    let unique: std::collections::HashSet<_> = sel.iter().collect();
    assert_eq!(sel.len(), unique.len());
    assert_eq!(sel.len(), 3);
}

#[test]
fn drafts_are_stashed_and_restored_per_conversation() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![
            conv("a", "alice", MembersType::ImpTeamNative),
            conv("b", "bob", MembersType::ImpTeamNative),
        ],
        "a",
    );
    open_conversation_by_id(&mut rig.app, "a".into());
    rig.app.compose.set("hi from a");
    // Switch away → 'a' stashes its draft; 'b' has none.
    open_conversation_by_id(&mut rig.app, "b".into());
    assert_eq!(rig.app.compose.text(), "");
    // Back to 'a' → the draft is restored.
    open_conversation_by_id(&mut rig.app, "a".into());
    assert_eq!(rig.app.compose.text(), "hi from a");
}

// ── Unread counter ────────────────────────────────────────────────────

#[test]
fn unread_total_counts_only_unread_conversations() {
    let mut rig = build_rig();
    let mut c1 = conv("a", "alice", MembersType::ImpTeamNative);
    c1.unread = true;
    let c2 = conv("b", "bob", MembersType::ImpTeamNative);
    let mut c3 = conv("c", "carol", MembersType::Team);
    c3.unread = true;
    rig.app.conversations = vec![c1, c2, c3];
    assert_eq!(rig.app.unread_total(), 2);
}

// ── Mark-as-read up-to-latest ────────────────────────────────────────

#[test]
fn mark_read_uses_latest_loaded_message_id_when_conv_is_open() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(10, "alice", "old"), text_msg(11, "alice", "newer")];
    request_mark_read(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let st = rig.mock.st();
    assert_eq!(st.mark_reads, vec![11]);
}

#[test]
fn mark_read_falls_back_to_zero_when_no_messages_loaded() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.open_conv_id = None;
    request_mark_read(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let st = rig.mock.st();
    assert_eq!(st.mark_reads, vec![0]);
}

// ── Incoming push messages (api-listen → handle_incoming_message) ─────

#[test]
fn incoming_text_appends_to_the_open_conversation() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(1, "alice", "hi")];
    handle_incoming_message(&mut rig.app, "c1".into(), text_msg(2, "alice", "there"));
    assert_eq!(rig.app.messages.len(), 2);
    assert_eq!(rig.app.messages.last().unwrap().id, 2);
}

#[test]
fn incoming_message_deduplicates_by_id() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.messages = vec![text_msg(5, "alice", "once")];
    handle_incoming_message(&mut rig.app, "c1".into(), text_msg(5, "alice", "again"));
    assert_eq!(
        rig.app.messages.len(),
        1,
        "a message already in the stream must not be re-appended"
    );
}

#[test]
fn incoming_edit_triggers_a_reread_of_the_open_conversation() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    let mut edit = text_msg(9, "alice", "");
    edit.content = MessageContent::Edit {
        target_id: 1,
        body: "fixed".into(),
    };
    handle_incoming_message(&mut rig.app, "c1".into(), edit);
    // Edits modify an existing message → re-read for correct projection,
    // rather than appending the edit envelope as a new line.
    assert!(
        matches!(rig.app.in_flight, Some(InFlight::LoadMessages)),
        "an edit push should queue a re-read, not append"
    );
}

#[test]
fn incoming_from_another_user_marks_unread_when_not_viewing() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.open_conv_id = None; // not viewing it
    handle_incoming_message(&mut rig.app, "c1".into(), text_msg(2, "alice", "ping"));
    assert!(
        rig.app
            .conversations
            .iter()
            .find(|c| c.id == "c1")
            .unwrap()
            .unread
    );
}

#[test]
fn incoming_from_me_does_not_mark_unread() {
    let mut rig = build_rig();
    rig.app.identity.username = "me".into();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.open_conv_id = None;
    handle_incoming_message(&mut rig.app, "c1".into(), text_msg(2, "me", "self"));
    assert!(
        !rig.app
            .conversations
            .iter()
            .find(|c| c.id == "c1")
            .unwrap()
            .unread,
        "an echo of our own message must not mark the conversation unread"
    );
}

#[test]
fn incoming_while_viewing_does_not_mark_unread() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    // open_conv_id == c1 (set by preload) → we're viewing it.
    handle_incoming_message(&mut rig.app, "c1".into(), text_msg(2, "alice", "seen"));
    assert!(
        !rig.app
            .conversations
            .iter()
            .find(|c| c.id == "c1")
            .unwrap()
            .unread,
        "a message in the open conversation is already read"
    );
}

#[test]
fn incoming_message_bumps_conversation_recency() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.open_conv_id = None;
    let mut m = text_msg(2, "alice", "newer");
    m.sent_at = 1_700;
    m.sent_at_ms = 1_700_000;
    handle_incoming_message(&mut rig.app, "c1".into(), m);
    let c = rig.app.conversations.iter().find(|c| c.id == "c1").unwrap();
    assert_eq!(c.active_at_ms, 1_700_000);
    assert_eq!(c.active_at, 1_700);
}

#[test]
fn incoming_for_unknown_conversation_triggers_silent_refresh() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    handle_incoming_message(
        &mut rig.app,
        "zzz-unknown".into(),
        text_msg(2, "alice", "first"),
    );
    assert!(
        rig.app.bg_inflight,
        "a message for a conversation not yet in the inbox must queue a silent resync"
    );
}

#[test]
fn background_silent_refresh_does_not_consume_the_user_in_flight_slot() {
    // The idle auto-refresh ships on the same response channel as the user
    // lane; apply_response must route it by variant *before* the in_flight
    // match so it can't steal the slot a user request is occupying.
    let mut rig = build_rig();
    rig.app.in_flight = Some(InFlight::LoadMessages);
    rig.app.bg_inflight = true;
    apply_response(
        &mut rig.app,
        WorkerResponse::ListConversationsSilent(Ok(ListConversationsOk {
            conversations: vec![conv("c1", "alice", MembersType::ImpTeamNative)],
            skipped: Vec::new(),
        })),
    );
    // The background lane clears its own flag and applies the inbox…
    assert!(
        !rig.app.bg_inflight,
        "silent refresh must clear bg_inflight"
    );
    assert_eq!(rig.app.conversations.len(), 1);
    // …without touching the user's in-flight slot.
    assert!(
        matches!(rig.app.in_flight, Some(InFlight::LoadMessages)),
        "a background response must not consume the user's in-flight slot"
    );
}

#[test]
fn inbox_refresh_preserves_the_tree_cursor_by_id() {
    // A safety-net resync must not yank the tree cursor to the top.
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![
            conv("c1", "alice", MembersType::ImpTeamNative),
            conv("c2", "bob", MembersType::ImpTeamNative),
        ],
        "c2",
    );
    // Cursor starts on the *second* conversation.
    let id_of = |app: &App| app.selected_conversation().map(|c| c.id.clone());
    assert_eq!(id_of(&rig.app).as_deref(), Some("c2"));
    // Refresh the inbox (same data, as the 180s resync would).
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // The cursor must still be on c2, not jumped back to the first row.
    assert_eq!(
        id_of(&rig.app).as_deref(),
        Some("c2"),
        "refresh should keep the cursor on the same conversation"
    );
}

#[test]
fn mark_read_clears_the_tree_selected_conversation() {
    // Regression: mark-read used to flip `unread` on
    // `filtered_cache[list_selected]`, but the tree cursor
    // (`tree_selected`) — what `selected_conversation` reads — was a
    // different, untracked index, so the wrong row's badge cleared (and
    // an oversized index could panic). It must clear the row that's
    // actually selected, found by id.
    let mut rig = build_rig();
    let mut c1 = conv("c1", "alice", MembersType::ImpTeamNative);
    c1.unread = true;
    let mut c2 = conv("c2", "bob", MembersType::ImpTeamNative);
    c2.unread = true;
    // Seats the tree cursor on c2 (the second row, not the first).
    preload_inbox(&mut rig.app, &rig.mock, vec![c1, c2], "c2");
    request_mark_read(&mut rig.app);
    pump_until_idle(&mut rig.app);
    let unread = |id: &str| {
        rig.app
            .conversations
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .unread
    };
    assert!(!unread("c2"), "tree-selected conversation should be read");
    assert!(unread("c1"), "the other conversation must stay unread");
}

// ── Render perf smoke (manual: `cargo test -- --ignored`) ──────────

/// Runs the full TUI render `iterations` times against an in-memory
/// `TestBackend` and returns the wall-clock duration of each frame.
///
/// Used by the `render_perf_smoke_*` tests below. Lives behind
/// `#[ignore]` so `cargo test` stays under a second on CI; run
/// explicitly with `cargo test --release -- --ignored render_perf
/// --nocapture` to see the timings.
fn run_render_iterations(app: &mut App, iterations: usize) -> Vec<std::time::Duration> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend init");
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = std::time::Instant::now();
        terminal
            .draw(|frame| crate::tui::view::draw(frame, app))
            .expect("draw");
        times.push(start.elapsed());
    }
    times
}

fn print_timings(label: &str, times: &[std::time::Duration]) {
    let total: std::time::Duration = times.iter().sum();
    let avg = total / times.len() as u32;
    let mut sorted = times.to_vec();
    sorted.sort();
    let p50 = sorted[sorted.len() / 2];
    let p99 = sorted[sorted.len() * 99 / 100];
    let max = sorted.last().copied().unwrap_or_default();
    println!("{label}: avg={avg:?} p50={p50:?} p99={p99:?} max={max:?}");
}

#[test]
fn settings_screen_renders_every_section_without_panicking() {
    use crate::tui::app::{SettingsFocus, SettingsSection};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut rig = build_rig();
    rig.app.identity.username = "alice".into();
    rig.app.identity.device_name = "Debian desktop".into();
    rig.app.identity.device_type = "desktop".into();
    rig.app.screen = Screen::Inbox;
    rig.app.open_settings();
    // A long value to exercise the panel's value-trimming path.
    rig.app.settings_cache.image_symbols = "octant+sextant+block+space".into();

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("backend");
    for section in 0..SettingsSection::ALL.len() {
        rig.app.settings_section = section;
        rig.app.settings_item = 0;
        for focus in [SettingsFocus::Sidebar, SettingsFocus::Panel] {
            rig.app.settings_focus = focus;
            terminal
                .draw(|f| crate::tui::view::draw(f, &mut rig.app))
                .expect("draw must not panic");
        }
    }
}

/// Flattens a `TestBackend` buffer to one string (rows joined by `\n`) so a
/// test can assert that expected text actually rendered somewhere on screen.
fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let area = *buf.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn login_form_renders_at_every_supported_width() {
    // The login screen is a form. It must
    // render its fields + buttons without panicking or clipping the labels at
    // every width the app draws it (≥ 70×18, the too-small guard; below that
    // the resize notice shows instead of the login).
    use crate::domain::LineEditor;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    rig.app.login_username = LineEditor::from_text("alice");
    rig.app.login_device = LineEditor::from_text("secretbase");

    for (w, h) in [(70u16, 18u16), (80, 24), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("backend");
        terminal
            .draw(|f| crate::tui::view::draw(f, &mut rig.app))
            .expect("draw must not panic");
        let text = buffer_text(&terminal);
        for needle in ["Login", "Username", "Device", "Paper key", "Log in"] {
            assert!(
                text.contains(needle),
                "login form missing {needle:?} at {w}x{h}:\n{text}"
            );
        }
    }
}

#[test]
#[ignore = "perf smoke — run manually with `--ignored --nocapture`"]
fn render_perf_smoke_inbox_500_conversations() {
    let mut rig = build_rig();
    let convs: Vec<Conversation> = (0..500)
        .map(|i| {
            // Two-letter id to anchor inbox lookups; the long-form
            // name exercises the label rendering that does most of
            // the per-row allocation.
            let mut c = conv(
                &format!("conv-{i:04}"),
                &format!("user_{i}_with_a_longish_name"),
                MembersType::ImpTeamNative,
            );
            c.unread = i % 7 == 0;
            c
        })
        .collect();
    rig.mock.st().conversations = convs;
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    rig.app.screen = Screen::Inbox;

    let times = run_render_iterations(&mut rig.app, 100);
    print_timings("inbox 500-conv", &times);

    // Generous upper bound — debug builds, single 120x40 frame
    // touching 500 conversations should be well under 100ms even
    // on slow hardware. A regression that breaks this implies
    // something quadratic crept into the render path.
    let worst = times.iter().max().copied().unwrap_or_default();
    assert!(
        worst < std::time::Duration::from_millis(100),
        "worst inbox frame: {worst:?}"
    );
}

#[test]
#[ignore = "perf smoke — run manually with `--ignored --nocapture`"]
fn render_perf_smoke_conversation_50_messages() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice,bob", MembersType::ImpTeamNative)],
        "c1",
    );
    let msgs: Vec<Message> = (0..50)
        .map(|i| {
            text_msg(
                i,
                if i % 2 == 0 { "alice" } else { "bob" },
                "a message body of reasonable length to exercise the wrapper",
            )
        })
        .collect();
    rig.app.messages = msgs;
    rig.app.screen = Screen::Inbox;

    let times = run_render_iterations(&mut rig.app, 100);
    print_timings("conv 50-msg", &times);

    let worst = times.iter().max().copied().unwrap_or_default();
    assert!(
        worst < std::time::Duration::from_millis(100),
        "worst conv frame: {worst:?}"
    );
}

// ── apply_response defensive branches ────────────────────────────────

// ── Input handler bindings ────────────────────────────────────────────
//
// Anchor the most frequently-used keybindings so a stray rebind shows
// up as a failing test rather than as a confused user. We exercise the
// public `handle_events` entry point with a synthetic `KeyEvent` and
// assert the observable state mutation (focus, screen, in_flight, …).
// Bindings that just call into already-tested flows
// (`request_load_inbox`, `request_save_edit`, …) are anchored by their
// resulting `in_flight` slot rather than re-asserting the downstream
// effects.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// Builds a `KeyEvent` with a `Press` kind — the only kind the
/// dispatcher honours.
fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    crate::tui::input::handle_events(app, Event::Key(key(code, mods)));
}

#[test]
fn input_ctrl_c_quits_from_inbox() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    press(&mut rig.app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(rig.app.should_quit);
}

#[test]
fn input_ctrl_c_quits_from_conversation() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    press(&mut rig.app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(rig.app.should_quit);
}

#[test]
fn input_f1_toggles_help_overlay() {
    let mut rig = build_rig();
    set_identity(&mut rig.app, "alice");
    rig.app.screen = Screen::Inbox;
    press(&mut rig.app, KeyCode::F(1), KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Help);
    press(&mut rig.app, KeyCode::F(1), KeyModifiers::NONE);
    // post_help_screen routes back to Inbox when logged-in + no
    // conversation open.
    assert_eq!(rig.app.screen, Screen::Inbox);
}

#[test]
fn input_tab_steps_forward_through_non_search_focuses() {
    // Tab advances FOCUS_ORDER one step except when focus is
    // Search — there Tab is swallowed by the search-input handler
    // (the user must Esc / Enter out of search before tabbing on).
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;

    // FOCUS_ORDER = [Search, Tree, Chat, CmdLog]; Chat is skipped with no
    // open conversation.
    rig.app.focus = Focus::Tree;
    press(&mut rig.app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(rig.app.focus, Focus::CmdLog); // Chat skipped (no conv open)

    rig.app.focus = Focus::CmdLog;
    press(&mut rig.app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(rig.app.focus, Focus::Search, "wraps around");
}

#[test]
fn input_tab_from_search_cycles_focus() {
    // Tab is a global focus-cycle and works even from the search box
    // (it can't be confused with text input), so the user can leave
    // search with one keystroke. FOCUS_ORDER = [Search, Filters, …].
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Search;
    press(&mut rig.app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(rig.app.focus, Focus::Tree);
}

#[test]
fn input_alt_f_jumps_to_filter_focus() {
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Tree;
    // Alt+F focuses the chat Filter (the left search box).
    press(&mut rig.app, KeyCode::Char('f'), KeyModifiers::ALT);
    assert_eq!(rig.app.focus, Focus::Search);
}

#[test]
fn input_f5_on_inbox_queues_load_inbox() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    press(&mut rig.app, KeyCode::F(5), KeyModifiers::NONE);
    assert!(matches!(rig.app.in_flight, Some(InFlight::LoadInbox)));
}

#[test]
fn input_r_on_inbox_tree_queues_load_inbox() {
    // Gradient: bare `r` refreshes the focused inbox tree (Alt is reserved
    // for panel jumps; the tree isn't a text field, so its letters act).
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Tree;
    press(&mut rig.app, KeyCode::Char('r'), KeyModifiers::NONE);
    assert!(matches!(rig.app.in_flight, Some(InFlight::LoadInbox)));
}

#[test]
fn input_e_on_inbox_tree_queues_mark_read() {
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app);
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Tree;
    // Bare `e` = mark as sEEn/read.
    press(&mut rig.app, KeyCode::Char('e'), KeyModifiers::NONE);
    assert!(matches!(rig.app.in_flight, Some(InFlight::MarkRead { .. })));
}

#[test]
fn input_n_on_inbox_tree_opens_new_conversation() {
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Tree;
    press(&mut rig.app, KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::NewConversation);
}

#[test]
fn input_shift_i_on_inbox_tree_opens_ignore_confirm() {
    // Destructive tier: Shift+I (Char('I')) opens the ignore confirmation —
    // moved off the old Alt+I so Alt stays purely panel-jumps.
    use crate::tui::app::ConvAction;
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app);
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Tree;
    press(&mut rig.app, KeyCode::Char('I'), KeyModifiers::SHIFT);
    assert_eq!(rig.app.screen, Screen::ConfirmConvAction);
    assert!(matches!(
        rig.app.pending_conv_action,
        Some(ConvAction::Ignore)
    ));
}

#[test]
fn input_bare_i_on_inbox_tree_is_inert() {
    // Only Shift+I ignores; a bare `i` must do nothing destructive (it isn't
    // a tree action), so the screen stays put.
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app);
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Tree;
    press(&mut rig.app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert!(rig.app.pending_conv_action.is_none());
}

#[test]
fn input_shift_l_opens_confirm_logout() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    // Capital L: most terminals report Char('L') with SHIFT mod —
    // the handler matches on the Char so the mod doesn't matter.
    press(&mut rig.app, KeyCode::Char('L'), KeyModifiers::SHIFT);
    assert_eq!(rig.app.screen, Screen::ConfirmLogout);
}

#[test]
fn input_confirm_logout_y_queues_logout_from_inbox() {
    let mut rig = build_rig();
    rig.app.screen = Screen::ConfirmLogout;
    press(&mut rig.app, KeyCode::Char('y'), KeyModifiers::NONE);
    // The confirm queues the logout but returns to the inbox; only
    // `handle_logout_response` moves to Login, and only on success, so a
    // failed logout can't strand the user on Login while still signed in.
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert!(matches!(rig.app.in_flight, Some(InFlight::Logout)));
}

#[test]
fn input_confirm_logout_n_cancels() {
    let mut rig = build_rig();
    rig.app.screen = Screen::ConfirmLogout;
    press(&mut rig.app, KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert!(rig.app.in_flight.is_none());
}

#[test]
fn ignore_conversation_confirm_flow() {
    use crate::tui::app::ConvAction;
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app);

    // Opening the action stages it behind the confirm popup.
    open_conv_action(&mut rig.app, ConvAction::Ignore);
    assert_eq!(rig.app.screen, Screen::ConfirmConvAction);
    assert_eq!(rig.app.pending_conv_action, Some(ConvAction::Ignore));
    assert!(rig.app.in_flight.is_none(), "must not fire before confirm");

    // Confirming issues the setstatus call and returns to the inbox.
    confirm_conv_action(&mut rig.app);
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert!(rig.app.pending_conv_action.is_none());
    assert!(matches!(
        rig.app.in_flight,
        Some(InFlight::SetConvStatus { .. })
    ));
    pump_until_idle(&mut rig.app);
    match &rig.app.action_state {
        ActionState::Done(s) => assert_eq!(s, "Ignored"),
        other => panic!("expected Done(Ignored), got {other:?}"),
    }
}

#[test]
fn ignore_conversation_cancel_does_nothing() {
    use crate::tui::app::ConvAction;
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app);
    open_conv_action(&mut rig.app, ConvAction::Ignore);
    cancel_conv_action(&mut rig.app);
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert!(rig.app.pending_conv_action.is_none());
    assert!(rig.app.in_flight.is_none());
}

#[test]
fn input_enter_on_list_opens_selected_conversation() {
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "alice", MembersType::ImpTeamNative)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    reveal_first(&mut rig.app); // cursor on the conversation, not its group header
    rig.app.screen = Screen::Inbox;
    rig.app.focus = crate::tui::screens::Focus::Tree;
    press(&mut rig.app, KeyCode::Enter, KeyModifiers::NONE);
    // Opens the chat in-place on the unified Home (no screen switch).
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert_eq!(rig.app.open_conv_id.as_deref(), Some("c1"));
}

#[test]
fn tree_back_closes_open_chat() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    // l/→ opens the selected conversation; h/← closes it (master-detail back).
    tree_activate(&mut rig.app);
    assert_eq!(rig.app.open_conv_id.as_deref(), Some("c1"));
    tree_back(&mut rig.app);
    assert!(rig.app.open_conv_id.is_none());
}

#[test]
fn ctrl_f_opens_and_closes_the_conv_search_modal() {
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Chat;
    // Ctrl+F opens the in-conversation search modal (not a focusable panel).
    press(&mut rig.app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.screen, Screen::ConvSearch);
    // Esc closes it back to the inbox.
    press(&mut rig.app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Inbox);
}

#[test]
fn cmdlog_multiselect_copies_marked_lines() {
    let mut rig = build_rig();
    rig.app.push_cmd("a", true, "one");
    rig.app.push_cmd("b", true, "two");
    rig.app.push_cmd("c", false, "three");
    rig.app.enter_cmdlog(); // cursor on the newest (index 2), no marks
    rig.app.cmdlog_toggle_mark(); // mark idx 2
    rig.app.cmdlog_move(isize::MIN); // cursor -> oldest (0)
    rig.app.cmdlog_toggle_mark(); // mark idx 0
    assert_eq!(rig.app.cmdlog_marks.len(), 2);

    do_copy_cmd_log(&mut rig.app, true);
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
    // Selection is kept (so it can be copied full + detail), and the copy
    // itself is logged.
    assert_eq!(rig.app.cmdlog_marks.len(), 2);
    assert_eq!(rig.app.cmd_log.last().unwrap().cmd, "clipboard write");
}

#[test]
fn go_to_keys_focus_each_panel() {
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.screen = Screen::Inbox;
    // Go-to works from any focus (here from the chat compose): Alt+L log,
    // Alt+C chats, Alt+M chat, Ctrl+F in-chat find.
    rig.app.focus = Focus::Chat;
    press(&mut rig.app, KeyCode::Char('l'), KeyModifiers::ALT);
    assert_eq!(rig.app.focus, Focus::CmdLog);
    press(&mut rig.app, KeyCode::Char('c'), KeyModifiers::ALT);
    assert_eq!(rig.app.focus, Focus::Tree);
    press(&mut rig.app, KeyCode::Char('m'), KeyModifiers::ALT);
    assert_eq!(rig.app.focus, Focus::Chat);
    press(&mut rig.app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(rig.app.screen, Screen::ConvSearch);
}

#[test]
fn ctrl_w_window_nav_moves_between_panels() {
    use crate::tui::screens::Focus;
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.screen = Screen::Inbox;
    rig.app.focus = Focus::Chat;

    // Ctrl+W arms the leader; it stays armed across consecutive directions so
    // two keys chain: k (up) chat → filter, then j (down) filter → chats.
    press(&mut rig.app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert!(rig.app.pending_pane_nav);
    press(&mut rig.app, KeyCode::Char('k'), KeyModifiers::NONE); // up → filter
    assert_eq!(rig.app.focus, Focus::Search);
    assert!(
        rig.app.pending_pane_nav,
        "still armed for the next direction"
    );
    press(&mut rig.app, KeyCode::Char('j'), KeyModifiers::NONE); // down → chats
    assert_eq!(rig.app.focus, Focus::Tree);

    // Esc leaves window-nav mode.
    press(&mut rig.app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!rig.app.pending_pane_nav);

    // A single move then a non-direction key exits and is processed normally.
    rig.app.focus = Focus::Tree;
    press(&mut rig.app, KeyCode::Char('w'), KeyModifiers::CONTROL);
    press(&mut rig.app, KeyCode::Char('j'), KeyModifiers::NONE); // down → log
    assert_eq!(rig.app.focus, Focus::CmdLog);
    assert!(rig.app.pending_pane_nav);
    press(&mut rig.app, KeyCode::Char('g'), KeyModifiers::NONE); // exits nav mode
    assert!(!rig.app.pending_pane_nav);
}

#[test]
fn mention_autocomplete_filters_and_accepts() {
    let mut rig = build_rig();
    rig.app.conv_members = vec!["alice".into(), "alfredo".into(), "bob".into()];
    rig.app.compose.set("hey @al");
    // Prefix-filtered (conv_members order preserved).
    assert_eq!(rig.app.mention_matches(), vec!["alice", "alfredo"]);
    // Accepting replaces "@al" with "@alice " (trailing space).
    accept_mention(&mut rig.app, "alice");
    assert_eq!(rig.app.compose.text(), "hey @alice ");
    // A completed mention is no longer "active".
    assert!(rig.app.mention_matches().is_empty());
}

#[test]
fn open_url_opens_first_link_or_reports_none() {
    use crate::domain::{Message, MessageContent};
    let text = |body: &str| {
        let mut m = Message::default();
        m.id = 1;
        m.content = MessageContent::Text(body.into());
        m
    };
    let mut rig = build_rig();

    rig.app.messages = vec![text("docs at https://keybase.io/x and more")];
    rig.app.selected_msg_idx = Some(0);
    do_open_url(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
    assert_eq!(rig.app.cmd_log.last().unwrap().cmd, "open url");
    // Copy the same link.
    do_copy_url(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
    assert_eq!(rig.app.cmd_log.last().unwrap().cmd, "clipboard write");

    rig.app.messages = vec![text("no link here")];
    rig.app.selected_msg_idx = Some(0);
    do_open_url(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    do_copy_url(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn chat_multiselect_copies_messages() {
    use crate::domain::{Message, MessageContent};
    let mk = |id: u64, sender: &str, body: &str| {
        let mut m = Message::default();
        m.id = id;
        m.sender = sender.into();
        m.content = MessageContent::Text(body.into());
        m
    };
    let mut rig = build_rig();
    rig.app.messages = vec![
        mk(1, "ana", "hola"),
        mk(2, "beto", "chau"),
        mk(3, "ana", "ok"),
    ];
    rig.app.rebuild_msg_meta();
    enter_select_mode(&mut rig.app); // cursor on the last message, marks cleared
    rig.app.selected_msg_idx = Some(0);
    msg_toggle_mark(&mut rig.app);
    rig.app.selected_msg_idx = Some(2);
    msg_toggle_mark(&mut rig.app);
    assert_eq!(rig.app.msg_marks.len(), 2);

    // Full copy (author + time + body) and content-only copy both work and
    // keep the selection.
    do_copy_messages(&mut rig.app, true);
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
    do_copy_messages(&mut rig.app, false);
    assert!(matches!(rig.app.action_state, ActionState::Done(_)));
    assert_eq!(rig.app.msg_marks.len(), 2);
}

#[test]
fn input_q_on_inbox_does_not_quit() {
    // Only Ctrl+C quits (per UX.md). Bare 'q' must be free for
    // type-to-search and must NOT exit the app.
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    press(&mut rig.app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(!rig.app.should_quit);
}

#[test]
fn input_conversation_enter_with_buffer_sends() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.screen = Screen::Inbox;
    rig.app.focus = crate::tui::screens::Focus::Chat;
    rig.app.compose_open = true;
    rig.app.compose.set("hello");
    press(&mut rig.app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(
        rig.app.in_flight,
        Some(InFlight::SendMessage { .. })
    ));
}

#[test]
fn input_conversation_enter_with_empty_buffer_errors() {
    let mut rig = build_rig();
    preload_inbox(
        &mut rig.app,
        &rig.mock,
        vec![conv("c1", "alice", MembersType::ImpTeamNative)],
        "c1",
    );
    rig.app.screen = Screen::Inbox;
    rig.app.focus = crate::tui::screens::Focus::Chat;
    rig.app.compose_open = true;
    rig.app.compose.clear();
    press(&mut rig.app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
}

#[test]
fn input_login_f5_retries_status_check() {
    // The login screen is now a form, so bare letters type into fields —
    // retry-status moved to F5 (`r` would land in the Username field).
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    press(&mut rig.app, KeyCode::F(5), KeyModifiers::NONE);
    assert!(matches!(rig.app.in_flight, Some(InFlight::Status)));
}

#[test]
fn input_login_esc_quits() {
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    press(&mut rig.app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(rig.app.should_quit);
}

#[test]
fn input_login_letters_type_into_focused_field() {
    // A bare letter on the login form is text, not an action.
    use crate::tui::app::LoginField;
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    rig.app.login_focus = LoginField::Username;
    for c in "alice".chars() {
        press(&mut rig.app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    assert_eq!(rig.app.login_username.text(), "alice");
    assert!(!rig.app.should_quit);
}

#[test]
fn input_login_enter_submits_paperkey_and_records_call() {
    // Filled form + Enter → non-interactive paper-key login.
    use crate::domain::LineEditor;
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    rig.app.login_username = LineEditor::from_text("alice");
    rig.app.login_device = LineEditor::from_text("secretbase");
    rig.app.login_paperkey = LineEditor::from_text("word ".repeat(9).trim());
    press(&mut rig.app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(rig.app.in_flight, Some(InFlight::LoginPaperkey)));
    pump_one(&mut rig.app);
    let logins = &rig.mock.st().logins;
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].0, "alice");
    assert_eq!(logins[0].1, "secretbase");
}

#[test]
fn login_paperkey_requires_all_fields() {
    // Empty username → validation error, no worker call, focus moves to it.
    use crate::domain::LineEditor;
    use crate::tui::app::LoginField;
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    rig.app.login_device = LineEditor::from_text("secretbase");
    rig.app.login_paperkey = LineEditor::from_text("word word word");
    crate::tui::flows::auth::request_login_paperkey(&mut rig.app);
    assert!(rig.app.in_flight.is_none());
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert_eq!(rig.app.login_focus, LoginField::Username);
}

#[test]
fn login_paperkey_failure_surfaces_error_and_stays_on_login() {
    use crate::domain::LineEditor;
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    rig.app.login_username = LineEditor::from_text("alice");
    rig.app.login_device = LineEditor::from_text("secretbase");
    rig.app.login_paperkey = LineEditor::from_text("word word word");
    rig.mock.st().fail_next = Some(KeybaseError::Exit {
        stderr: "already provisioned this device".into(),
        status: 1,
    });
    crate::tui::flows::auth::request_login_paperkey(&mut rig.app);
    pump_one(&mut rig.app);
    assert!(matches!(rig.app.action_state, ActionState::Error(_)));
    assert_eq!(rig.app.screen, Screen::Login);
    // The paper-key field is kept so the user can switch to native login.
    assert!(!rig.app.login_paperkey.is_empty());
}

#[test]
fn input_login_native_button_sets_pending_native_login() {
    use crate::domain::LineEditor;
    use crate::tui::app::LoginField;
    let mut rig = build_rig();
    rig.app.screen = Screen::Login;
    rig.app.login_username = LineEditor::from_text("alice");
    rig.app.login_focus = LoginField::SubmitNative;
    press(&mut rig.app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.pending_native_login.as_deref(), Some("alice"));
}

#[test]
fn unhide_by_name_sends_setstatus_unfiled_on_built_channel() {
    let mut rig = build_rig();
    rig.app.identity.username = "me".into();
    open_unhide(&mut rig.app);
    assert_eq!(rig.app.screen, Screen::UnhideConversation);
    rig.app.unhide_input.set("alice");
    request_unhide_conversation(&mut rig.app);
    // Popup closes and a setstatus call is in flight.
    assert_eq!(rig.app.screen, Screen::Inbox);
    assert!(matches!(
        rig.app.in_flight,
        Some(InFlight::SetConvStatus { .. })
    ));
    pump_until_idle(&mut rig.app);
    // The adapter saw `unfiled` on the rebuilt "me,alice" impteam channel.
    let statuses = rig.mock.st().statuses.clone();
    assert!(
        statuses
            .iter()
            .any(|(name, st)| name == "me,alice" && st == "unfiled"),
        "statuses: {statuses:?}"
    );
}

#[test]
fn conv_action_block_report_map_to_setstatus_values() {
    use crate::tui::app::ConvAction;
    assert_eq!(ConvAction::Block.status(), "blocked");
    assert_eq!(ConvAction::Report.status(), "reported");
    assert_eq!(ConvAction::Ignore.status(), "ignored");
}

#[test]
fn handle_decode_gif_stores_frames_and_clears_pending() {
    use crate::tui::image::GifFrames;
    let mut rig = build_rig();
    let path = "/cache/x-1.gif".to_string();
    rig.app.gif_pending.insert(path.clone());
    let frames = GifFrames {
        frames: vec!["a.png".into(), "b.png".into()],
        delays_ms: vec![80, 80],
        total_ms: 160,
    };
    handle_decode_gif_response(&mut rig.app, path.clone(), Some(frames));
    // Pending cleared, frames stored, a repaint flagged.
    assert!(!rig.app.gif_pending.contains(&path));
    assert!(matches!(rig.app.gif_anims.get(&path), Some(Some(_))));
    assert!(rig.app.image_dirty);
}

#[test]
fn settings_esc_steps_panel_to_sidebar_then_closes() {
    use crate::tui::app::SettingsFocus;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.open_settings(); // Settings overlay, focus on the section sidebar
    // Enter a section's panel.
    press(&mut rig.app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(rig.app.settings_focus, SettingsFocus::Panel);
    // First Esc steps back to the sidebar (does NOT close the overlay).
    press(&mut rig.app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(rig.app.settings_focus, SettingsFocus::Sidebar);
    assert_eq!(rig.app.screen, Screen::Settings);
    // Second Esc (on the sidebar) closes, returning to where it was opened.
    press(&mut rig.app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Inbox);
}

#[test]
fn input_esc_on_new_conv_popup_closes_it() {
    let mut rig = build_rig();
    rig.app.screen = Screen::NewConversation;
    press(&mut rig.app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(rig.app.screen, Screen::Inbox);
}

#[test]
fn input_key_release_events_are_ignored() {
    // The dispatcher honours only `Press` events. A Release for
    // Ctrl+C must NOT trip the quit handler.
    use crossterm::event::KeyEventKind;
    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    let release = KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Release,
        state: crossterm::event::KeyEventState::NONE,
    };
    crate::tui::input::handle_events(&mut rig.app, Event::Key(release));
    assert!(!rig.app.should_quit);
}

// ── Mouse hit-test staleness ──────────────────────────────────────────

#[test]
fn mouse_handler_rejects_clicks_against_stale_rects_after_resize() {
    // Race: user clicks on the inbox List panel, terminal is resized
    // before the run loop processes the click, and the in-buffer
    // Mouse event arrives before the Resize event. The rects in
    // `app.mouse_areas` reflect the OLD frame; the click coordinates
    // are in the NEW grid. Without the staleness check, the click
    // would land in the wrong panel.
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.focus = crate::tui::screens::Focus::Tree;
    // Frame was drawn against a 120x40 terminal — the List panel
    // sat at (40..120, 5..30).
    rig.app.mouse_areas.frame_size = (120, 40);
    rig.app.mouse_areas.list = Rect {
        x: 40,
        y: 5,
        width: 80,
        height: 25,
    };
    // But the terminal has just been resized to 80x30 — the click
    // we're about to inject was emitted at coords valid in the NEW
    // grid only.
    rig.app.last_terminal_size = (80, 30);

    crate::tui::input::mouse::handle(
        &mut rig.app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    );

    // Focus must NOT have flipped to List based on stale rects.
    assert_eq!(rig.app.focus, crate::tui::screens::Focus::Tree);
}

#[test]
fn mouse_handler_honors_clicks_when_rects_are_fresh() {
    // Sanity contrast: same click, same rect, but the frame_size
    // matches last_terminal_size — the click is honored.
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    let mut rig = build_rig();
    rig.app.screen = Screen::Inbox;
    rig.app.focus = crate::tui::screens::Focus::Tree;
    rig.app.mouse_areas.frame_size = (120, 40);
    rig.app.mouse_areas.list = Rect {
        x: 40,
        y: 5,
        width: 80,
        height: 25,
    };
    rig.app.last_terminal_size = (120, 40);

    crate::tui::input::mouse::handle(
        &mut rig.app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    );

    // Clicking the (chat) pane rect focuses the chat.
    assert_eq!(rig.app.focus, crate::tui::screens::Focus::Chat);
}

#[test]
fn apply_response_drops_message_when_no_in_flight_slot() {
    // A response arriving without a matching in-flight context is
    // a worker/main-thread disagreement that should NEVER happen
    // in normal operation — but the dispatcher must not panic, and
    // must leave a breadcrumb the user can audit.
    let mut rig = build_rig();
    assert!(rig.app.in_flight.is_none());
    apply_response(
        &mut rig.app,
        crate::tui::worker::WorkerResponse::Logout(Ok(())),
    );
    // No state change beyond the cmd_log warning.
    assert!(rig.app.in_flight.is_none());
    let logged = rig.app.cmd_log.iter().any(|e| {
        !e.ok && e.cmd == "worker response" && e.detail.contains("without an in-flight slot")
    });
    assert!(logged, "cmd_log: {:?}", rig.app.cmd_log);
}

#[test]
fn apply_response_surfaces_dispatch_mismatch_to_user() {
    // Slot says Status, response says Logout — disagreement
    // that should never happen but we surface as a hard error
    // rather than silently dropping or hanging on the spinner
    // forever.
    let mut rig = build_rig();
    rig.app.in_flight = Some(InFlight::Status);
    apply_response(
        &mut rig.app,
        crate::tui::worker::WorkerResponse::Logout(Ok(())),
    );
    match &rig.app.action_state {
        ActionState::Error(s) => assert_eq!(s, "internal worker dispatch mismatch"),
        other => panic!("expected Error, got {other:?}"),
    }
    // The slot must be cleared so the next request can run.
    assert!(rig.app.in_flight.is_none());
    let logged = rig
        .app
        .cmd_log
        .iter()
        .any(|e| !e.ok && e.detail.contains("dispatch mismatch"));
    assert!(logged);
}

// ── Path-traversal hardening of attachment filenames ─────────────────

use super::safe_attachment_basename;

#[test]
fn safe_basename_preserves_simple_filename() {
    assert_eq!(safe_attachment_basename("photo.png", 1), "photo.png");
    assert_eq!(safe_attachment_basename("a b c.txt", 2), "a b c.txt");
    // Dots inside the filename are fine — only the path-traversal
    // segments are rejected.
    assert_eq!(
        safe_attachment_basename("archive.tar.gz", 3),
        "archive.tar.gz"
    );
}

#[test]
fn safe_basename_strips_unix_path_components() {
    assert_eq!(safe_attachment_basename("/etc/passwd", 1), "passwd");
    assert_eq!(
        safe_attachment_basename("../../.ssh/authorized_keys", 1),
        "authorized_keys"
    );
    assert_eq!(safe_attachment_basename("a/b/c/d.bin", 1), "d.bin");
}

#[test]
fn safe_basename_strips_windows_path_components() {
    assert_eq!(
        safe_attachment_basename("C:\\Windows\\System32\\drivers\\etc\\hosts", 1),
        "hosts"
    );
    assert_eq!(
        safe_attachment_basename("..\\..\\Windows\\notepad.exe", 1),
        "notepad.exe"
    );
    // Mixed separators — the rsplit picks whichever comes last.
    assert_eq!(safe_attachment_basename("a/b\\c.txt", 1), "c.txt");
}

#[test]
fn safe_basename_falls_back_for_empty_or_dot_inputs() {
    assert_eq!(safe_attachment_basename("", 7), "attachment-7.bin");
    assert_eq!(safe_attachment_basename(".", 7), "attachment-7.bin");
    assert_eq!(safe_attachment_basename("..", 7), "attachment-7.bin");
    // Trailing-slash inputs reduce to "" after rsplit → fallback.
    assert_eq!(safe_attachment_basename("../", 9), "attachment-9.bin");
    assert_eq!(safe_attachment_basename("foo/", 9), "attachment-9.bin");
}

#[test]
fn safe_basename_strips_control_chars_and_nuls() {
    // NUL inside the filename is removed; what remains is the visible
    // basename.
    assert_eq!(safe_attachment_basename("foo\0bar.txt", 1), "foobar.txt");
    // Newline injection (could otherwise confuse logs) is dropped.
    assert_eq!(
        safe_attachment_basename("rogue\nname.txt", 1),
        "roguename.txt"
    );
    // Carriage return + tab — both stripped.
    assert_eq!(safe_attachment_basename("a\r\tb.png", 1), "ab.png");
}

#[test]
fn safe_basename_handles_only_control_chars() {
    // A filename made entirely of control chars boils down to empty
    // → fallback.
    assert_eq!(
        safe_attachment_basename("\0\n\t\r", 42),
        "attachment-42.bin"
    );
}

#[test]
fn open_download_uses_safe_basename_for_malicious_filename() {
    use crate::tui::app::PickerAction;
    let mut rig = build_rig();
    // Simulate a malicious sender embedding a path traversal in the
    // attachment filename.
    rig.app.messages = vec![attachment_msg(
        77,
        "mallory",
        "../../.ssh/authorized_keys",
        16,
    )];
    rig.app.selected_msg_idx = Some(0);
    open_download_for_selected(&mut rig.app);
    // The download filename is sanitised to a bare basename, so joining it
    // onto the chosen directory can't escape with a `..` traversal.
    match &rig.app.picker_action {
        PickerAction::Download { filename, .. } => assert_eq!(filename, "authorized_keys"),
        _ => panic!("expected a Download action"),
    }
}

// ── read_channel_from_conv ────────────────────────────────────────────

#[test]
fn read_channel_maps_members_type_to_keybase_string() {
    let team = conv("c1", "phoenix", MembersType::Team);
    let mut team = team;
    team.channel.topic_name = Some("general".into());
    let ch = read_channel_from_conv(&team).expect("known variant");
    assert_eq!(ch.name, "phoenix");
    assert_eq!(ch.members_type, "team");
    assert_eq!(ch.topic_name.as_deref(), Some("general"));

    let dm = conv("c2", "alice,bob", MembersType::ImpTeamNative);
    let ch = read_channel_from_conv(&dm).expect("known variant");
    assert_eq!(ch.members_type, "impteamnative");
    assert!(ch.topic_name.is_none());
}

#[test]
fn read_channel_covers_every_known_members_type() {
    // Anchor the mapping so a future enum variant added without a
    // matching arm here trips this test rather than silently
    // routing to an unrelated channel.
    for (variant, expected) in [
        (MembersType::ImpTeamNative, "impteamnative"),
        (MembersType::ImpTeamUpgrade, "impteamupgrade"),
        (MembersType::Team, "team"),
        (MembersType::Kbfs, "kbfs"),
    ] {
        let c = conv("x", "name", variant);
        let ch = read_channel_from_conv(&c).expect("known variant should map");
        assert_eq!(ch.members_type, expected, "variant {variant:?}");
    }
}

#[test]
fn read_channel_rejects_unknown_members_type() {
    use crate::tui::flows::chat::UNKNOWN_MEMBERS_TYPE_ERR;
    let mystery = conv("xx", "futureconv", MembersType::Unknown);
    let err = read_channel_from_conv(&mystery).expect_err("Unknown must error");
    assert_eq!(err, UNKNOWN_MEMBERS_TYPE_ERR);
}

#[test]
fn request_mark_read_surfaces_unknown_members_type() {
    // Integration check: a request_* path receiving an Unknown
    // conversation must NOT route to "impteamnative" and must NOT
    // dispatch a worker call. The error lands on the feedback
    // strip and command log.
    use crate::tui::flows::chat::UNKNOWN_MEMBERS_TYPE_ERR;
    let mut rig = build_rig();
    rig.mock.st().conversations = vec![conv("c1", "futureconv", MembersType::Unknown)];
    request_load_inbox(&mut rig.app);
    pump_until_idle(&mut rig.app);
    // Now try to mark it read.
    reveal_first(&mut rig.app);
    request_mark_read(&mut rig.app);
    // No worker call was queued.
    assert!(rig.app.in_flight.is_none());
    match &rig.app.action_state {
        ActionState::Error(s) => assert_eq!(s, UNKNOWN_MEMBERS_TYPE_ERR),
        other => panic!("expected Error, got {other:?}"),
    }
    // Mock must NOT have received a mark_read.
    assert!(rig.mock.st().mark_reads.is_empty());
}
