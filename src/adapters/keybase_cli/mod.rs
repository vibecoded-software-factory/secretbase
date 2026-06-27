//! Concrete [`KeybasePort`] implementation that shells out to the
//! `keybase` CLI binary.
//!
//! ## Subprocess pattern
//!
//! Every JSON-API call follows the same recipe:
//!
//! 1. Build a JSON request via [`codec`].
//! 2. Spawn `keybase chat api` / `keybase team api` with the request
//!    on stdin.
//! 3. Parse the JSON reply from stdout.
//! 4. Surface a non-zero exit / `error` field as `Err(String)`.
//!
//! ## Status command
//!
//! `keybase status --json` is the only call that does not go through
//! the chat / team API. It targets the local `keybased` service and
//! emits a large blob whose useful fields we project into
//! [`IdentityInfo`].
//!
//! ## Timeouts
//!
//! Each call category gets its own timeout budget. The default
//! `list_inbox_timeout_secs` is exposed in [`UserSettings`] so users
//! with very large inboxes can lift the cap without touching the
//! source.
//!
//! ## Memory-hygiene policy ("Zeroize")
//!
//! Chat bodies and search queries travel through this layer in
//! plaintext. The threat we mitigate is "stale plaintext sitting in
//! deallocated heap pages that a later allocation in the same
//! process can read" — not a privileged attacker (root, kernel,
//! debugger) for whom zeroize is theatre.
//!
//! What this module **does** zero:
//!
//! * The JSON request body written to the child's stdin
//!   ([`Zeroizing::new(encode_request(...))`]).
//! * The intermediate `String` holding the child's stdout before
//!   parsing.
//! * The raw `Vec<u8>` returned by [`std::process::Command`] (via
//!   an explicit [`Zeroize::zeroize`] before drop).
//! * Specific typed domain values: [`Conversation`], [`Message`],
//!   [`IdentityInfo`], and friends carry `#[derive(Zeroize,
//!   ZeroizeOnDrop)]` so their heap-owned fields clear on drop.
//!
//! What this module deliberately **does not** zero (and why):
//!
//! * [`serde_json::Value`] trees produced by parsing — `Value`
//!   owns its inner `String`s, but they are not zeroized. Wrapping
//!   `Value` would require either patching serde or rewriting the
//!   parser. We instead minimise the window: parse, project into
//!   the typed (zeroized) domain types as fast as possible, then
//!   drop the `Value`.
//! * The owned `String` allocated by `String::from_utf8_lossy` when
//!   the input contains invalid UTF-8 (rare for JSON output).
//! * Error strings returned upward as `Err(String)`. The TUI logs
//!   them in `cmd_log` (kept in-memory only) and `set_action`. To
//!   keep these short and non-leaky we truncate any embedded reply
//!   body to a short head before formatting.
//!
//! The net effect is "best-effort hygiene": no plaintext lingers
//! past its last legitimate use in the buffers this module owns
//! directly. Anything beyond that — kernel buffer cache for the
//! pipe, terminal scrollback, swap pages — is out of scope.

pub mod codec;
pub mod json;
pub mod listen;
pub mod process;
pub mod session;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::domain::{
    AttachmentInfo, Conversation, IdentityInfo, Message, MessageContent, Reaction, SystemInfo,
    SystemKind, TeamMembership, team_role_name,
};
use crate::ports::KeybaseError;
use crate::ports::keybase::{
    KeybasePort, ListConversationsOk, ListTeamsOk, ParallelSessionData, ReadChannel,
};

use codec::{channel_object, encode_request, request_no_params, request_with_options};
use json::extract_error;
use process::{keybase_run_timeout, stderr_str, stdout_str};
use session::ApiSession;

/// Timeout for `keybase status --json`. Local-only — should return in
/// milliseconds.
const STATUS_TIMEOUT: u64 = 5;

/// Timeout for short write-only chat/team calls (mark, send, react, …).
const QUICK_OP_TIMEOUT: u64 = 15;

/// Timeout for a chat `read` (potentially fetches up to 200 messages).
const READ_TIMEOUT: u64 = 30;

/// Default upper bound on the inbox list call — overridable via
/// [`UserSettings::list_inbox_timeout_secs`].
const DEFAULT_LIST_INBOX_TIMEOUT: u64 = 30;

/// Default wall-clock budget for `keybase chat api {"method":"download"}`.
/// Attachments can be many MB; the per-read 30 s budget is wrong
/// for this path. Overridable via
/// [`UserSettings::download_timeout_secs`].
const DEFAULT_DOWNLOAD_TIMEOUT: u64 = 300;

/// Adapter implementing [`KeybasePort`] by shelling out to the
/// `keybase` binary.
pub struct KeybaseCliAdapter {
    /// Wall-clock budget for `keybase chat api {"method":"list"}`.
    list_inbox_timeout: u64,
    /// Wall-clock budget for `keybase chat api {"method":"download"}`.
    download_timeout: u64,
    /// Long-lived `keybase chat api` stream (one-shot fallback inside).
    chat: ApiSession,
    /// Long-lived `keybase team api` stream (one-shot fallback inside).
    team: ApiSession,
}

impl KeybaseCliAdapter {
    /// Construct with default timeouts.
    pub fn new() -> Self {
        Self {
            list_inbox_timeout: DEFAULT_LIST_INBOX_TIMEOUT,
            download_timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            chat: ApiSession::new("chat"),
            team: ApiSession::new("team"),
        }
    }

    /// Returns the persistent session for an API family.
    fn session_for(&mut self, family: &str) -> &mut ApiSession {
        match family {
            "team" => &mut self.team,
            _ => &mut self.chat,
        }
    }

    /// Override the inbox-list timeout — used by the composition root
    /// to apply the `list_inbox_timeout_secs` setting before the first
    /// call.
    pub fn with_list_inbox_timeout(mut self, secs: u64) -> Self {
        if secs > 0 {
            self.list_inbox_timeout = secs;
        }
        self
    }

    /// Override the attachment-download timeout — used by the
    /// composition root to apply the `download_timeout_secs` setting.
    /// Passing `0` keeps the default.
    pub fn with_download_timeout(mut self, secs: u64) -> Self {
        if secs > 0 {
            self.download_timeout = secs;
        }
        self
    }

    /// Runs a `keybase chat api` request, returning the parsed JSON
    /// reply on success or a typed [`KeybaseError`].
    fn chat_api(&mut self, request: &Value, timeout: u64) -> Result<Value, KeybaseError> {
        self.run_api("chat", request, timeout)
    }

    /// Runs a `keybase team api` request.
    fn team_api(&mut self, request: &Value, timeout: u64) -> Result<Value, KeybaseError> {
        self.run_api("team", request, timeout)
    }

    fn run_api(
        &mut self,
        family: &str,
        request: &Value,
        timeout: u64,
    ) -> Result<Value, KeybaseError> {
        let idempotent = is_idempotent_method(request.get("method").and_then(Value::as_str));
        let stdin = Zeroizing::new(encode_request(request));
        let stdout = self.session_for(family).run(&stdin, timeout, idempotent)?;
        let parsed: Value =
            serde_json::from_str(&stdout).map_err(|e| KeybaseError::InvalidJson {
                family: family.to_string(),
                detail: e.to_string(),
            })?;
        if let Some(err) = extract_error(&parsed) {
            return Err(err);
        }
        Ok(parsed)
    }

    /// Variant of [`Self::run_api`] for the few methods that need to
    /// return the *raw* JSON payload (wrapped in [`Zeroizing`]) rather
    /// than a parsed [`Value`]. Used by `read_conversation_json` and
    /// `search_inbox`, where downstream code prefers to parse on its
    /// own schedule.
    ///
    /// Crucially, this still runs [`extract_error`] on the parsed
    /// output: Keybase sometimes exits 0 with an error body
    /// (`{"error":{"message":"…"}}`) and the earlier "check only the
    /// exit status" path silently passed that through as if it were
    /// a real result. Now any embedded `error` field becomes `Err`.
    fn run_api_raw(
        &mut self,
        family: &str,
        request: &Value,
        timeout: u64,
    ) -> Result<Zeroizing<String>, KeybaseError> {
        let idempotent = is_idempotent_method(request.get("method").and_then(Value::as_str));
        let stdin = Zeroizing::new(encode_request(request));
        let body = self.session_for(family).run(&stdin, timeout, idempotent)?;
        validate_api_json(body, family)
    }
}

impl Default for KeybaseCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether an API method is safe to **retry** (re-execute) after a
/// delivered-but-unanswered persistent call. Only read-only methods
/// qualify; every mutation (send/react/edit/delete/pin/mark/…) and any
/// unknown method defaults to `false` so the persistent→one-shot
/// fallback can never double-execute a state change.
fn is_idempotent_method(method: Option<&str>) -> bool {
    matches!(
        method,
        Some("list" | "read" | "searchinbox" | "list-self-memberships")
    )
}

impl KeybasePort for KeybaseCliAdapter {
    // ── Identity / session ────────────────────────────────────────────────

    fn status(&mut self) -> Result<IdentityInfo, KeybaseError> {
        // `keybase status --json` prints a service start-up notice on
        // stderr the first time the service spins up. We deliberately
        // ignore stderr unless the call fails — the JSON we care about
        // is always on stdout.
        let out = keybase_run_timeout(&["status", "--json"], STATUS_TIMEOUT)?;
        if !out.status.success() {
            return Err(KeybaseError::Exit {
                stderr: stderr_str(&out),
                status: out.status.code().unwrap_or(-1),
            });
        }
        let body = stdout_str(&out);
        let v: Value = serde_json::from_str(&body).map_err(|e| KeybaseError::InvalidJson {
            family: "status".into(),
            detail: e.to_string(),
        })?;
        Ok(IdentityInfo {
            username: v
                .get("Username")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            user_id: v
                .get("UserID")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            default_username: v
                .get("DefaultUsername")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            logged_in: v.get("LoggedIn").and_then(Value::as_bool).unwrap_or(false),
            session_is_valid: v
                .get("SessionIsValid")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            device_name: v
                .pointer("/Device/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            device_id: v
                .pointer("/Device/deviceID")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            device_type: v
                .pointer("/Device/type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    fn logout(&mut self) -> Result<(), KeybaseError> {
        let out = keybase_run_timeout(&["logout"], QUICK_OP_TIMEOUT)?;
        if !out.status.success() {
            return Err(KeybaseError::Exit {
                stderr: stderr_str(&out),
                status: out.status.code().unwrap_or(-1),
            });
        }
        Ok(())
    }

    // ── Chat — inbox / read ───────────────────────────────────────────────

    fn list_conversations(&mut self) -> Result<ListConversationsOk, KeybaseError> {
        let reply = self.chat_api(&request_no_params("list"), self.list_inbox_timeout)?;
        let arr = reply
            .pointer("/result/conversations")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                KeybaseError::shape("keybase chat api list: missing result.conversations")
            })?;
        Ok(parse_conversations_array(arr))
    }

    fn read_messages(
        &mut self,
        channel: &ReadChannel,
        num: u32,
        peek: bool,
        next_cursor: Option<&str>,
    ) -> Result<(Vec<Message>, Option<String>), KeybaseError> {
        let mut pagination = json!({ "num": num });
        if let Some(cursor) = next_cursor {
            pagination["next"] = json!(cursor);
        }
        let req = request_with_options(
            "read",
            json!({
                "channel": channel_object(channel),
                "pagination": pagination,
                "peek": peek,
            }),
        );
        let reply = self.chat_api(&req, READ_TIMEOUT)?;
        let arr = reply
            .pointer("/result/messages")
            .and_then(Value::as_array)
            .ok_or_else(|| KeybaseError::shape("keybase chat api read: missing result.messages"))?;
        let mut out: Vec<Message> = Vec::with_capacity(arr.len());
        for entry in arr {
            let Some(msg) = entry.get("msg") else {
                continue;
            };
            if let Some(m) = parse_message(msg) {
                out.push(m);
            }
        }

        // Surface the cursor for the next older page. Keybase signals
        // "no more history" via either `pagination.last == true` or by
        // omitting the cursor altogether; we treat both as `None`.
        let pag = reply.pointer("/result/pagination").unwrap_or(&Value::Null);
        let last = pag.get("last").and_then(Value::as_bool).unwrap_or(false);
        let next = if last {
            None
        } else {
            pag.get("next")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        };
        Ok((out, next))
    }

    fn read_conversation_json(
        &mut self,
        channel: &ReadChannel,
        num: u32,
    ) -> Result<Zeroizing<String>, KeybaseError> {
        let req = request_with_options(
            "read",
            json!({
                "channel": channel_object(channel),
                "pagination": { "num": num },
            }),
        );
        self.run_api_raw("chat", &req, READ_TIMEOUT)
    }

    fn mark_read(&mut self, channel: &ReadChannel, message_id: u64) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "mark",
            json!({
                "channel": channel_object(channel),
                "message_id": message_id,
            }),
        );
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn search_inbox(
        &mut self,
        query: &str,
        max_hits: u32,
    ) -> Result<Zeroizing<String>, KeybaseError> {
        let req = request_with_options(
            "searchinbox",
            json!({
                "query": query,
                "max_hits": max_hits,
            }),
        );
        self.run_api_raw("chat", &req, READ_TIMEOUT)
    }

    // ── Chat — write ──────────────────────────────────────────────────────

    fn send_message(
        &mut self,
        channel: &ReadChannel,
        body: &str,
        reply_to: Option<u64>,
    ) -> Result<(), KeybaseError> {
        let mut options = json!({
            "channel": channel_object(channel),
            "message": { "body": body },
        });
        if let Some(reply_id) = reply_to {
            options["reply_to"] = json!(reply_id);
        }
        let req = request_with_options("send", options);
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn edit_message(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
        body: &str,
    ) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "edit",
            json!({
                "channel": channel_object(channel),
                "message_id": message_id,
                "message": { "body": body },
            }),
        );
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn delete_message(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
    ) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "delete",
            json!({
                "channel": channel_object(channel),
                "message_id": message_id,
            }),
        );
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn react(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
        body: &str,
    ) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "reaction",
            json!({
                "channel": channel_object(channel),
                "message_id": message_id,
                "message": { "body": body },
            }),
        );
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn new_conversation(&mut self, channel: &ReadChannel) -> Result<String, KeybaseError> {
        let req = request_with_options("newconv", json!({ "channel": channel_object(channel) }));
        let reply = self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        let id = reply
            .pointer("/result/id")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .unwrap_or_default();
        Ok(id)
    }

    fn set_conversation_status(
        &mut self,
        channel: &ReadChannel,
        status: &str,
    ) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "setstatus",
            json!({
                "channel": channel_object(channel),
                "status": status,
            }),
        );
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn pin_message(&mut self, channel: &ReadChannel, message_id: u64) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "pin",
            json!({
                "channel": channel_object(channel),
                "message_id": message_id,
            }),
        );
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn unpin_message(&mut self, channel: &ReadChannel) -> Result<(), KeybaseError> {
        let req = request_with_options("unpin", json!({ "channel": channel_object(channel) }));
        self.chat_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn download_attachment(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
        output: &str,
    ) -> Result<(), KeybaseError> {
        // Downloads can be many MB over a slow uplink — way past
        // the per-`read` 30 s budget. Use the dedicated download
        // timeout, configurable via `UserSettings::download_timeout_secs`.
        let req = request_with_options(
            "download",
            json!({
                "channel": channel_object(channel),
                "message_id": message_id,
                "output": output,
            }),
        );
        self.chat_api(&req, self.download_timeout)?;
        Ok(())
    }

    // ── Teams ─────────────────────────────────────────────────────────────

    fn list_self_memberships(&mut self) -> Result<ListTeamsOk, KeybaseError> {
        let reply = self.team_api(&request_no_params("list-self-memberships"), READ_TIMEOUT)?;
        // The `team list-self-memberships` reply lives under
        // `result.teams` — but keybase has been emitting it under
        // `result.teams` for years, so we treat the missing key as a
        // hard error.
        let arr = reply
            .pointer("/result/teams")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                KeybaseError::shape("keybase team api list-self-memberships: missing result.teams")
            })?;
        Ok(parse_teams_array(arr))
    }

    fn create_team(&mut self, name: &str) -> Result<(), KeybaseError> {
        let req = request_with_options("create-team", json!({"team": name}));
        self.team_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn leave_team(&mut self, name: &str, permanent: bool) -> Result<(), KeybaseError> {
        let req = request_with_options(
            "leave-team",
            json!({
                "team": name,
                "permanent": permanent,
            }),
        );
        self.team_api(&req, QUICK_OP_TIMEOUT)?;
        Ok(())
    }

    fn parallel_session_data(&mut self) -> ParallelSessionData {
        // Sequential by default — parallelism only pays off when each
        // call carries a heavy per-spawn cost (e.g. a Node cold-start).
        // `keybase` is a single Go binary talking to a long-running
        // service so the spawn overhead is tiny; the sequential
        // baseline is fast enough.
        ParallelSessionData {
            teams: self.list_self_memberships(),
        }
    }
}

/// Parses one `msg` envelope from the `result.messages` array.
///
/// Returns `None` when the envelope is unusable — currently only one
/// case: a missing `id`. A message without an id cannot be addressed
/// for edits/replies/pins/marks-read, so dropping it is strictly
/// better than synthesizing a fake id. The caller skips silently and
/// keeps the rest of the page intact (philosophy lifted from the
/// inbox-tolerance fix in #4).
///
/// Unknown content types are still kept and wrapped in
/// [`MessageContent::Unknown`] so the view layer can render a useful
/// placeholder with the original type name — only structural
/// invariants (id) gate the `None` return.
pub(crate) fn parse_message(msg: &Value) -> Option<Message> {
    let id = msg.pointer("/id").and_then(Value::as_u64)?;
    let sender = msg
        .pointer("/sender/username")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let device = msg
        .pointer("/sender/device_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let sent_at = msg.pointer("/sent_at").and_then(Value::as_u64).unwrap_or(0);
    let sent_at_ms = msg
        .pointer("/sent_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let content = parse_content(msg.get("content").unwrap_or(&Value::Null));
    let reactions = parse_reactions(msg.get("reactions").unwrap_or(&Value::Null));
    Some(Message {
        id,
        sender,
        device,
        sent_at,
        sent_at_ms,
        content,
        reactions,
    })
}

/// Extracts the `reactions.reactions[emoji].users{}` map into a flat
/// `Vec<Reaction>` so the view layer doesn't have to navigate two
/// levels of JSON.
fn parse_reactions(reactions: &Value) -> Vec<Reaction> {
    let Some(map) = reactions.pointer("/reactions").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<Reaction> = Vec::with_capacity(map.len());
    for (emoji, body) in map {
        let users = body
            .get("users")
            .and_then(Value::as_object)
            .map(|u| u.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if users.is_empty() {
            continue;
        }
        out.push(Reaction {
            emoji: emoji.clone(),
            usernames: users,
        });
    }
    // Deterministic order — emoji string ascending so the view doesn't
    // jiggle between renders.
    out.sort_by(|a, b| a.emoji.cmp(&b.emoji));
    out
}

/// Translates a `content` JSON blob into the typed
/// [`MessageContent`] enum.
fn parse_content(content: &Value) -> MessageContent {
    let type_name = content
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match type_name.as_str() {
        "text" => MessageContent::Text(
            content
                .pointer("/text/body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        "edit" => MessageContent::Edit {
            target_id: content
                .pointer("/edit/messageID")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            body: content
                .pointer("/edit/body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "delete" => {
            let target_ids = content
                .pointer("/delete/messageIDs")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            MessageContent::Delete { target_ids }
        }
        "reaction" => MessageContent::Reaction {
            target_id: content
                .pointer("/reaction/m")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            body: content
                .pointer("/reaction/b")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "attachment" => MessageContent::Attachment(parse_attachment(content)),
        "system" => MessageContent::System(parse_system(content)),
        "metadata" => MessageContent::Metadata {
            title: content
                .pointer("/metadata/conversationTitle")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "headline" => MessageContent::Headline {
            headline: content
                .pointer("/headline/headline")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "pin" => MessageContent::Pin {
            target_id: content
                .pointer("/pin/messageID")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        "join" => MessageContent::Join {
            joiner: content
                .pointer("/join/joiner")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "leave" => MessageContent::Leave {
            leaver: content
                .pointer("/leave/leaver")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "sendpayment" => MessageContent::SendPayment {
            text: content
                .pointer("/sendpayment/paymentText")
                .and_then(Value::as_str)
                .unwrap_or("Stellar payment")
                .to_string(),
        },
        "requestpayment" => MessageContent::RequestPayment {
            text: content
                .pointer("/requestpayment/requestText")
                .and_then(Value::as_str)
                .unwrap_or("Stellar payment request")
                .to_string(),
        },
        "" => MessageContent::Unknown {
            type_name: "(empty)".into(),
        },
        other => MessageContent::Unknown {
            type_name: other.to_string(),
        },
    }
}

/// Extracts an [`AttachmentInfo`] from a `content` blob whose `type`
/// is `"attachment"`. Missing fields fall back to sensible defaults
/// so a malformed reply does not poison the whole conversation.
fn parse_attachment(content: &Value) -> AttachmentInfo {
    let obj = content
        .pointer("/attachment/object")
        .unwrap_or(&Value::Null);
    AttachmentInfo {
        title: obj
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        filename: obj
            .get("filename")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        size: obj.get("size").and_then(Value::as_u64).unwrap_or(0),
        mime_type: obj
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        uploaded: content
            .pointer("/attachment/uploaded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// Extracts a [`SystemInfo`] from a `content` blob whose `type` is
/// `"system"`. Computes a human-readable description from the inner
/// systemType-specific payload so the view does not have to switch on
/// every kind itself.
fn parse_system(content: &Value) -> SystemInfo {
    let sys = content.pointer("/system").unwrap_or(&Value::Null);
    let kind_int = sys.get("systemType").and_then(Value::as_i64).unwrap_or(-1);
    let kind = SystemKind::from_int(kind_int);

    let description = match kind {
        SystemKind::AddedToTeam | SystemKind::InviteAddedToTeam => {
            let key = if kind == SystemKind::AddedToTeam {
                "addedtoteam"
            } else {
                "inviteaddedtoteam"
            };
            let inner = sys.get(key).unwrap_or(&Value::Null);
            let adder = inner
                .get("adder")
                .and_then(Value::as_str)
                .unwrap_or("someone");
            let addee = inner
                .get("addee")
                .and_then(Value::as_str)
                .unwrap_or("someone");
            let role = team_role_name(inner.get("role").and_then(Value::as_i64).unwrap_or(-1));
            format!("{adder} added {addee} as {role}")
        }
        SystemKind::CreateTeam => {
            let team = sys
                .pointer("/createteam/team")
                .and_then(Value::as_str)
                .unwrap_or("");
            if team.is_empty() {
                "team created".to_string()
            } else {
                format!("team {team} created")
            }
        }
        SystemKind::ComplexTeam => {
            let team = sys
                .pointer("/complexteam/team")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("team {team} upgraded to complex team")
        }
        SystemKind::GitPush => {
            let pusher = sys
                .pointer("/gitpush/pusher")
                .and_then(Value::as_str)
                .unwrap_or("someone");
            let repo = sys
                .pointer("/gitpush/repoName")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("{pusher} pushed to git {repo}")
        }
        SystemKind::ChangeAvatar => {
            let user = sys
                .pointer("/changeavatar/user")
                .and_then(Value::as_str)
                .unwrap_or("someone");
            format!("{user} changed the team avatar")
        }
        SystemKind::ChangeRetention => "retention policy changed".to_string(),
        SystemKind::BulkAddToConv => {
            let usernames = sys
                .pointer("/bulkaddtoconv/usernames")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if usernames.is_empty() {
                "members added".to_string()
            } else {
                format!("added {usernames}")
            }
        }
        SystemKind::SbsResolve => "social-based invite resolved".to_string(),
        SystemKind::NewChannel => {
            let creator = sys
                .pointer("/newchannel/creator")
                .and_then(Value::as_str)
                .unwrap_or("someone");
            let channel = sys
                .pointer("/newchannel/nameAtCreation")
                .and_then(Value::as_str)
                .unwrap_or("a channel");
            format!("{creator} created channel {channel}")
        }
        SystemKind::Other => format!("system event (type {kind_int})"),
    };

    SystemInfo { kind, description }
}

/// Parses the raw `keybase {family} api` reply, checks for an
/// embedded `error` field, and either returns the bytes unchanged
/// (wrapped in [`Zeroizing`]) or the error message.
///
/// `family` is the API family name (`"chat"` or `"team"`) — used
/// only for the error message when the JSON itself is malformed.
fn validate_api_json(
    stdout: Zeroizing<String>,
    family: &str,
) -> Result<Zeroizing<String>, KeybaseError> {
    let parsed: Value = serde_json::from_str(&stdout).map_err(|e| KeybaseError::InvalidJson {
        family: family.to_string(),
        detail: e.to_string(),
    })?;
    if let Some(err) = extract_error(&parsed) {
        return Err(err);
    }
    Ok(stdout)
}

/// Decodes `result.conversations[]` into typed [`Conversation`]s,
/// tolerating per-row decode failures.
///
/// One malformed row used to abort the whole call (a single field
/// schema change locked the user out of the entire inbox). Now we
/// keep the good rows and surface the bad ones as warnings on
/// [`ListConversationsOk::skipped`] so the caller can show them in
/// the command log without losing access to the rest of the inbox.
fn parse_conversations_array(arr: &[Value]) -> ListConversationsOk {
    let mut conversations: Vec<Conversation> = Vec::with_capacity(arr.len());
    let mut skipped: Vec<String> = Vec::new();
    for (idx, item) in arr.iter().enumerate() {
        match serde_json::from_value::<Conversation>(item.clone()) {
            Ok(c) => conversations.push(c),
            Err(e) => {
                let id_hint = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|s| format!(" (id={})", s.chars().take(16).collect::<String>()))
                    .unwrap_or_default();
                skipped.push(format!("conversation #{idx}{id_hint}: {e}"));
            }
        }
    }
    ListConversationsOk {
        conversations,
        skipped,
    }
}

/// Same shape as [`parse_conversations_array`] but for team
/// memberships.
fn parse_teams_array(arr: &[Value]) -> ListTeamsOk {
    let mut teams: Vec<TeamMembership> = Vec::with_capacity(arr.len());
    let mut skipped: Vec<String> = Vec::new();
    for (idx, item) in arr.iter().enumerate() {
        match serde_json::from_value::<TeamMembership>(item.clone()) {
            Ok(t) => teams.push(t),
            Err(e) => {
                let name_hint = item
                    .get("fq_name")
                    .and_then(Value::as_str)
                    .map(|s| format!(" (fq_name={s})"))
                    .unwrap_or_default();
                skipped.push(format!("team #{idx}{name_hint}: {e}"));
            }
        }
    }
    ListTeamsOk { teams, skipped }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;

    #[test]
    fn new_uses_documented_default_timeouts() {
        let a = KeybaseCliAdapter::new();
        assert_eq!(a.list_inbox_timeout, DEFAULT_LIST_INBOX_TIMEOUT);
        assert_eq!(a.download_timeout, DEFAULT_DOWNLOAD_TIMEOUT);
    }

    #[test]
    fn with_download_timeout_overrides_when_nonzero() {
        let a = KeybaseCliAdapter::new().with_download_timeout(900);
        assert_eq!(a.download_timeout, 900);
    }

    #[test]
    fn with_download_timeout_zero_keeps_default() {
        let a = KeybaseCliAdapter::new().with_download_timeout(0);
        assert_eq!(a.download_timeout, DEFAULT_DOWNLOAD_TIMEOUT);
    }

    #[test]
    fn with_list_inbox_timeout_zero_keeps_default() {
        // Existing builder semantics — anchor here so future
        // changes don't drift the override-on-zero contract.
        let a = KeybaseCliAdapter::new().with_list_inbox_timeout(0);
        assert_eq!(a.list_inbox_timeout, DEFAULT_LIST_INBOX_TIMEOUT);
    }
}

#[cfg(test)]
mod parse_array_tests {
    use super::*;
    use serde_json::json;

    fn good_conv_json(id: &str) -> Value {
        json!({
            "id": id,
            "channel": {
                "name": "alice,bob",
                "members_type": "impteamnative",
                "topic_type": "chat",
            },
            "is_default_conv": true,
            "unread": false,
            "active_at": 0,
            "active_at_ms": 0,
            "member_status": "active",
        })
    }

    #[test]
    fn parse_conversations_keeps_good_rows_when_one_is_malformed() {
        let arr = vec![
            good_conv_json("aa"),
            // Bad row: `id` is the wrong shape (object instead of string).
            json!({
                "id": {"oops": true},
                "channel": {"name": "x", "members_type": "team", "topic_type": "chat"},
            }),
            good_conv_json("bb"),
        ];
        let out = parse_conversations_array(&arr);
        assert_eq!(out.conversations.len(), 2);
        assert_eq!(out.conversations[0].id, "aa");
        assert_eq!(out.conversations[1].id, "bb");
        assert_eq!(out.skipped.len(), 1);
        assert!(
            out.skipped[0].contains("conversation #1"),
            "diag should locate the bad row: {}",
            out.skipped[0]
        );
    }

    #[test]
    fn parse_conversations_all_good_yields_no_skipped() {
        let arr = vec![good_conv_json("aa"), good_conv_json("bb")];
        let out = parse_conversations_array(&arr);
        assert_eq!(out.conversations.len(), 2);
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn parse_conversations_all_bad_yields_empty_with_warnings() {
        let arr = vec![
            json!({"id": {}}),
            json!({"id": 42}),
            json!({"not": "a conv"}),
        ];
        let out = parse_conversations_array(&arr);
        assert!(out.conversations.is_empty());
        assert_eq!(out.skipped.len(), 3);
    }

    fn good_team_json(name: &str) -> Value {
        json!({
            "fq_name": name,
            "is_implicit_team": false,
            "member_count": 1,
            "role": "admin",
        })
    }

    // ── parse_message ───────────────────────────────────────────

    #[test]
    fn parse_message_returns_none_when_id_is_missing() {
        let msg = json!({
            "sender": {"username": "alice", "device_name": "laptop"},
            "content": {"type": "text", "text": {"body": "hi"}},
        });
        assert!(parse_message(&msg).is_none());
    }

    #[test]
    fn parse_message_returns_none_when_id_is_wrong_type() {
        // `id` present but not a u64 — fail-soft like missing.
        let msg = json!({"id": "not-a-number"});
        assert!(parse_message(&msg).is_none());
    }

    #[test]
    fn parse_message_fills_defaults_for_missing_optional_fields() {
        // Minimal valid envelope — only `id` is structurally
        // required. Everything else defaults gracefully.
        let msg = json!({"id": 7});
        let m = parse_message(&msg).expect("id-only envelope parses");
        assert_eq!(m.id, 7);
        assert!(m.sender.is_empty());
        assert!(m.device.is_empty());
        assert_eq!(m.sent_at, 0);
        assert_eq!(m.sent_at_ms, 0);
        assert!(matches!(
            m.content,
            crate::domain::MessageContent::Unknown { .. }
        ));
        assert!(m.reactions.is_empty());
    }

    #[test]
    fn parse_message_extracts_full_text_envelope() {
        let msg = json!({
            "id": 42,
            "sender": {"username": "alice", "device_name": "laptop"},
            "sent_at": 1000,
            "sent_at_ms": 1_000_000,
            "content": {"type": "text", "text": {"body": "hola"}},
        });
        let m = parse_message(&msg).expect("full envelope parses");
        assert_eq!(m.id, 42);
        assert_eq!(m.sender, "alice");
        assert_eq!(m.device, "laptop");
        assert_eq!(m.sent_at, 1000);
        assert_eq!(m.sent_at_ms, 1_000_000);
        match &m.content {
            crate::domain::MessageContent::Text(s) => assert_eq!(s, "hola"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_teams_keeps_good_rows_when_one_is_malformed() {
        let arr = vec![
            good_team_json("phoenix"),
            // `member_count` is the wrong type.
            json!({"fq_name": "broken", "member_count": "not-a-number"}),
        ];
        let out = parse_teams_array(&arr);
        assert_eq!(out.teams.len(), 1);
        assert_eq!(out.teams[0].name, "phoenix");
        assert_eq!(out.skipped.len(), 1);
        assert!(out.skipped[0].contains("fq_name=broken"));
    }

    // ── validate_api_json — error-in-body even when exit was 0 ────

    #[test]
    fn validate_returns_bytes_on_success() {
        let body = r#"{"result":{"messages":[]}}"#;
        let z = Zeroizing::new(body.to_string());
        let out = validate_api_json(z, "chat").expect("should accept");
        assert_eq!(out.as_str(), body);
    }

    #[test]
    fn validate_surfaces_error_object_message() {
        // Real-world shape: exit 0 + JSON body with `error.message`.
        // The old code returned this as a successful payload.
        let body = r#"{"error":{"message":"chat conv not found","code":7}}"#;
        let z = Zeroizing::new(body.to_string());
        let err = validate_api_json(z, "chat").expect_err("should reject");
        match err {
            KeybaseError::Api { code, message } => {
                assert_eq!(code, Some(7));
                assert_eq!(message, "chat conv not found");
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn validate_surfaces_top_level_error_string() {
        let body = r#"{"error":"bad query"}"#;
        let z = Zeroizing::new(body.to_string());
        let err = validate_api_json(z, "chat").expect_err("should reject");
        match err {
            KeybaseError::Api { code, message } => {
                assert!(code.is_none());
                assert_eq!(message, "bad query");
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn validate_surfaces_invalid_json() {
        let body = r#"not actually json{"#;
        let z = Zeroizing::new(body.to_string());
        let err = validate_api_json(z, "chat").expect_err("should reject");
        match err {
            KeybaseError::InvalidJson { family, detail } => {
                assert_eq!(family, "chat");
                assert!(!detail.is_empty());
            }
            other => panic!("expected InvalidJson, got {other:?}"),
        }
    }

    #[test]
    fn validate_preserves_zeroizing_wrapper_on_success() {
        // The byte-identity check above already implies this, but
        // assert that the returned String is exactly the input —
        // proving no re-allocation happened that would leave a copy
        // unzeroized.
        let body = r#"{"result":42}"#;
        let z = Zeroizing::new(body.to_string());
        let input_ptr = z.as_ptr();
        let out = validate_api_json(z, "team").expect("ok");
        assert_eq!(out.as_ptr(), input_ptr);
    }
}

/// JSON-fixture tests for the wire-format → typed-domain mapping in
/// [`parse_content`], [`parse_attachment`], [`parse_reactions`], and
/// [`parse_system`].
///
/// These functions used to be exercised only indirectly via
/// `read_messages` with mock data, which made it easy for a Keybase
/// schema bump (e.g. renaming `/reaction/m` to `/reaction/message_id`)
/// to slip through the test suite and only surface as a broken
/// renderer in production. Each variant now has its own anchor here.
#[cfg(test)]
mod content_parse_tests {
    use super::*;
    use serde_json::json;

    // ── parse_content variants ──────────────────────────────────

    #[test]
    fn content_text_extracts_body() {
        let c = json!({"type": "text", "text": {"body": "hello"}});
        match &parse_content(&c) {
            MessageContent::Text(s) => assert_eq!(s, "hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn content_text_defaults_to_empty_body_when_missing() {
        let c = json!({"type": "text"});
        match &parse_content(&c) {
            MessageContent::Text(s) => assert_eq!(s, ""),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn content_edit_extracts_target_and_body() {
        let c = json!({"type": "edit", "edit": {"messageID": 42, "body": "fixed"}});
        match &parse_content(&c) {
            MessageContent::Edit { target_id, body } => {
                assert_eq!(*target_id, 42_u64);
                assert_eq!(body, "fixed");
            }
            other => panic!("expected Edit, got {other:?}"),
        }
    }

    #[test]
    fn content_delete_collects_multiple_target_ids() {
        let c = json!({"type": "delete", "delete": {"messageIDs": [3, 5, 7]}});
        match &parse_content(&c) {
            MessageContent::Delete { target_ids } => {
                assert_eq!(target_ids.as_slice(), &[3_u64, 5, 7]);
            }
            other => panic!("expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn content_delete_empty_array_yields_empty_vec() {
        let c = json!({"type": "delete", "delete": {"messageIDs": []}});
        match &parse_content(&c) {
            MessageContent::Delete { target_ids } => assert!(target_ids.is_empty()),
            other => panic!("expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn content_reaction_uses_short_field_names_m_and_b() {
        // Wire format anchor: Keybase emits `m` (target id) and `b`
        // (body shortcode). If a future server bump renames these,
        // this test trips immediately.
        let c = json!({"type": "reaction", "reaction": {"m": 7, "b": ":+1:"}});
        match &parse_content(&c) {
            MessageContent::Reaction { target_id, body } => {
                assert_eq!(*target_id, 7_u64);
                assert_eq!(body, ":+1:");
            }
            other => panic!("expected Reaction, got {other:?}"),
        }
    }

    #[test]
    fn content_metadata_extracts_conversation_title() {
        let c = json!({"type": "metadata", "metadata": {"conversationTitle": "Welcome!"}});
        match &parse_content(&c) {
            MessageContent::Metadata { title } => assert_eq!(title, "Welcome!"),
            other => panic!("expected Metadata, got {other:?}"),
        }
    }

    #[test]
    fn content_headline_extracts_text() {
        let c = json!({"type": "headline", "headline": {"headline": "Daily standup at 10am"}});
        match &parse_content(&c) {
            MessageContent::Headline { headline } => assert_eq!(headline, "Daily standup at 10am"),
            other => panic!("expected Headline, got {other:?}"),
        }
    }

    #[test]
    fn content_pin_extracts_target_id() {
        let c = json!({"type": "pin", "pin": {"messageID": 99}});
        match &parse_content(&c) {
            MessageContent::Pin { target_id } => assert_eq!(*target_id, 99_u64),
            other => panic!("expected Pin, got {other:?}"),
        }
    }

    #[test]
    fn content_pin_cleared_emits_target_id_zero() {
        // Keybase signals "pin cleared" via target_id == 0. The
        // app-level `rebuild_pinned` relies on this convention.
        let c = json!({"type": "pin", "pin": {"messageID": 0}});
        match &parse_content(&c) {
            MessageContent::Pin { target_id } => assert_eq!(*target_id, 0_u64),
            other => panic!("expected Pin, got {other:?}"),
        }
    }

    #[test]
    fn content_join_extracts_username() {
        let c = json!({"type": "join", "join": {"joiner": "alice"}});
        match &parse_content(&c) {
            MessageContent::Join { joiner } => assert_eq!(joiner, "alice"),
            other => panic!("expected Join, got {other:?}"),
        }
    }

    #[test]
    fn content_leave_extracts_username() {
        let c = json!({"type": "leave", "leave": {"leaver": "bob"}});
        match &parse_content(&c) {
            MessageContent::Leave { leaver } => assert_eq!(leaver, "bob"),
            other => panic!("expected Leave, got {other:?}"),
        }
    }

    #[test]
    fn content_sendpayment_extracts_text_with_default() {
        let c = json!({"type": "sendpayment", "sendpayment": {"paymentText": "10 XLM to alice"}});
        match &parse_content(&c) {
            MessageContent::SendPayment { text } => assert_eq!(text, "10 XLM to alice"),
            other => panic!("expected SendPayment, got {other:?}"),
        }
        let c_default = json!({"type": "sendpayment"});
        match &parse_content(&c_default) {
            MessageContent::SendPayment { text } => assert_eq!(text, "Stellar payment"),
            other => panic!("expected SendPayment, got {other:?}"),
        }
    }

    #[test]
    fn content_requestpayment_extracts_text_with_default() {
        let c = json!({
            "type": "requestpayment",
            "requestpayment": {"requestText": "Please send 5 XLM"}
        });
        match &parse_content(&c) {
            MessageContent::RequestPayment { text } => assert_eq!(text, "Please send 5 XLM"),
            other => panic!("expected RequestPayment, got {other:?}"),
        }
        let c_default = json!({"type": "requestpayment"});
        match &parse_content(&c_default) {
            MessageContent::RequestPayment { text } => assert_eq!(text, "Stellar payment request"),
            other => panic!("expected RequestPayment, got {other:?}"),
        }
    }

    #[test]
    fn content_unknown_type_string_is_preserved_verbatim() {
        let c = json!({"type": "future_feature_xyz"});
        match &parse_content(&c) {
            MessageContent::Unknown { type_name } => assert_eq!(type_name, "future_feature_xyz"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn content_empty_type_falls_back_to_unknown_with_placeholder() {
        let c = json!({"type": ""});
        match &parse_content(&c) {
            MessageContent::Unknown { type_name } => assert_eq!(type_name, "(empty)"),
            other => panic!("expected Unknown(empty), got {other:?}"),
        }
    }

    #[test]
    fn content_missing_type_field_falls_back_to_unknown_with_placeholder() {
        // No `type` at all — should not panic, falls into the
        // `""` branch which emits the "(empty)" placeholder.
        let c = json!({});
        match &parse_content(&c) {
            MessageContent::Unknown { type_name } => assert_eq!(type_name, "(empty)"),
            other => panic!("expected Unknown(empty), got {other:?}"),
        }
    }

    // ── parse_attachment ───────────────────────────────────────

    #[test]
    fn attachment_extracts_all_fields() {
        let c = json!({
            "type": "attachment",
            "attachment": {
                "uploaded": true,
                "object": {
                    "title": "Vacation pic",
                    "filename": "beach.jpg",
                    "size": 2_345_678,
                    "mimeType": "image/jpeg",
                },
            },
        });
        let att = parse_attachment(&c);
        assert_eq!(att.title, "Vacation pic");
        assert_eq!(att.filename, "beach.jpg");
        assert_eq!(att.size, 2_345_678);
        assert_eq!(att.mime_type, "image/jpeg");
        assert!(att.uploaded);
    }

    #[test]
    fn attachment_defaults_when_object_missing() {
        let c = json!({"type": "attachment", "attachment": {"uploaded": false}});
        let att = parse_attachment(&c);
        assert!(att.title.is_empty());
        assert!(att.filename.is_empty());
        assert_eq!(att.size, 0);
        assert!(att.mime_type.is_empty());
        assert!(!att.uploaded);
    }

    #[test]
    fn content_attachment_delegates_to_parse_attachment() {
        // Smoke: content dispatcher actually routes "attachment"
        // through parse_attachment (vs swallowing as Unknown).
        let c = json!({
            "type": "attachment",
            "attachment": {"uploaded": true, "object": {"filename": "x.bin"}},
        });
        match &parse_content(&c) {
            MessageContent::Attachment(att) => {
                assert_eq!(att.filename, "x.bin");
                assert!(att.uploaded);
            }
            other => panic!("expected Attachment, got {other:?}"),
        }
    }

    // ── parse_reactions ────────────────────────────────────────

    #[test]
    fn reactions_flattens_emoji_user_map_in_deterministic_order() {
        let r = json!({
            "reactions": {
                ":heart:": {"users": {"bob": {}, "alice": {}}},
                ":+1:": {"users": {"charlie": {}}},
            }
        });
        let out = parse_reactions(&r);
        // Sorted by emoji ascending so render order is stable.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].emoji, ":+1:");
        assert_eq!(out[0].usernames, vec!["charlie".to_string()]);
        assert_eq!(out[1].emoji, ":heart:");
        // User order within an emoji is whatever the serde_json map
        // iteration yielded; just check membership.
        assert_eq!(out[1].usernames.len(), 2);
        assert!(out[1].usernames.iter().any(|u| u == "alice"));
        assert!(out[1].usernames.iter().any(|u| u == "bob"));
    }

    #[test]
    fn reactions_drops_emojis_with_no_remaining_users() {
        // Keybase can leave behind an empty user map after a
        // reaction is undone — those entries should be elided.
        let r = json!({
            "reactions": {
                ":+1:": {"users": {}},
                ":heart:": {"users": {"alice": {}}},
            }
        });
        let out = parse_reactions(&r);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].emoji, ":heart:");
    }

    #[test]
    fn reactions_returns_empty_when_field_missing_or_wrong_shape() {
        assert!(parse_reactions(&json!({})).is_empty());
        assert!(parse_reactions(&json!({"reactions": "not an object"})).is_empty());
        assert!(parse_reactions(&Value::Null).is_empty());
    }

    // ── parse_system: each SystemKind ──────────────────────────

    /// Wraps a `system.*` payload into the same envelope shape the
    /// adapter sees (`{"system": {"systemType": N, …}}`).
    fn sys(kind_int: i64, payload: Value) -> Value {
        let mut sys_obj = serde_json::Map::new();
        sys_obj.insert("systemType".into(), json!(kind_int));
        // Merge payload at the system level.
        if let Some(obj) = payload.as_object() {
            for (k, v) in obj {
                sys_obj.insert(k.clone(), v.clone());
            }
        }
        json!({"system": Value::Object(sys_obj)})
    }

    #[test]
    fn system_added_to_team_renders_adder_addee_role() {
        let c = sys(
            0,
            json!({"addedtoteam": {"adder": "alice", "addee": "bob", "role": 2}}),
        );
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::AddedToTeam);
        assert_eq!(info.description, "alice added bob as writer");
    }

    #[test]
    fn system_invite_added_to_team_uses_inviteaddedtoteam_key() {
        let c = sys(
            1,
            json!({"inviteaddedtoteam": {"adder": "owner", "addee": "guest", "role": 1}}),
        );
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::InviteAddedToTeam);
        assert_eq!(info.description, "owner added guest as reader");
    }

    #[test]
    fn system_complex_team() {
        let c = sys(2, json!({"complexteam": {"team": "phoenix"}}));
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::ComplexTeam);
        assert_eq!(info.description, "team phoenix upgraded to complex team");
    }

    #[test]
    fn system_create_team_with_and_without_name() {
        let with = sys(3, json!({"createteam": {"team": "phoenix"}}));
        assert_eq!(parse_system(&with).description, "team phoenix created");
        let without = sys(3, json!({}));
        assert_eq!(parse_system(&without).description, "team created");
    }

    #[test]
    fn system_git_push() {
        let c = sys(
            4,
            json!({"gitpush": {"pusher": "alice", "repoName": "tools"}}),
        );
        assert_eq!(parse_system(&c).description, "alice pushed to git tools");
    }

    #[test]
    fn system_change_avatar() {
        let c = sys(5, json!({"changeavatar": {"user": "alice"}}));
        assert_eq!(
            parse_system(&c).description,
            "alice changed the team avatar"
        );
    }

    #[test]
    fn system_change_retention_has_static_description() {
        let c = sys(6, json!({}));
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::ChangeRetention);
        assert_eq!(info.description, "retention policy changed");
    }

    #[test]
    fn system_bulk_add_to_conv_joins_usernames() {
        let c = sys(
            7,
            json!({"bulkaddtoconv": {"usernames": ["alice", "bob", "charlie"]}}),
        );
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::BulkAddToConv);
        assert_eq!(info.description, "added alice, bob, charlie");
        // Empty username list falls back to a generic phrasing.
        let empty = sys(7, json!({"bulkaddtoconv": {"usernames": []}}));
        assert_eq!(parse_system(&empty).description, "members added");
    }

    #[test]
    fn system_sbs_resolve_static_description() {
        let c = sys(8, json!({}));
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::SbsResolve);
        assert_eq!(info.description, "social-based invite resolved");
    }

    #[test]
    fn system_new_channel() {
        let c = sys(
            9,
            json!({"newchannel": {"creator": "alice", "nameAtCreation": "random"}}),
        );
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::NewChannel);
        assert_eq!(info.description, "alice created channel random");
    }

    #[test]
    fn system_other_carries_kind_int_in_description() {
        let c = sys(999, json!({}));
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::Other);
        assert_eq!(info.description, "system event (type 999)");
    }

    #[test]
    fn system_missing_systemtype_falls_back_to_other_minus_one() {
        let c = json!({"system": {}});
        let info = parse_system(&c);
        assert_eq!(info.kind, SystemKind::Other);
        assert_eq!(info.description, "system event (type -1)");
    }

    #[test]
    fn content_system_delegates_to_parse_system() {
        // End-to-end through the content dispatcher.
        let c = json!({
            "type": "system",
            "system": {"systemType": 3, "createteam": {"team": "phoenix"}}
        });
        match &parse_content(&c) {
            MessageContent::System(info) => {
                assert_eq!(info.kind, SystemKind::CreateTeam);
                assert_eq!(info.description, "team phoenix created");
            }
            other => panic!("expected System, got {other:?}"),
        }
    }
}
