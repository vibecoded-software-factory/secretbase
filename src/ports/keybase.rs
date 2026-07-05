//! Keybase backend port.

use zeroize::Zeroizing;

use crate::domain::{
    ChatMember, Conversation, Emoji, IdentityInfo, InboxHit, Message, TeamMembership,
};
use crate::ports::error::KeybaseError;

/// Successful outcome of [`KeybasePort::list_conversations`].
///
/// `skipped` carries one diagnostic per inbox row the adapter could
/// not decode (e.g. an unexpected JSON shape after a Keybase server
/// upgrade). It is empty when every row decoded cleanly. A non-empty
/// `skipped` is **not** a fatal error — the caller is expected to
/// render the good rows and surface the diagnostics as a soft
/// warning. Returning `Err` from the call is reserved for failures
/// that prevent *any* progress (transport, auth, top-level shape).
pub struct ListConversationsOk {
    pub conversations: Vec<Conversation>,
    pub skipped: Vec<String>,
}

/// Successful outcome of [`KeybasePort::list_self_memberships`].
/// Mirrors [`ListConversationsOk`] for team-membership rows.
pub struct ListTeamsOk {
    pub teams: Vec<TeamMembership>,
    pub skipped: Vec<String>,
}

/// Abstraction over the Keybase backend.
///
/// The current concrete implementation is
/// [`crate::adapters::keybase_cli::KeybaseCliAdapter`], which shells out
/// to the `keybase` binary. Tests can plug in a fake implementation.
///
/// All methods take `&mut self` (rather than the more granular `&self`
/// for reads) because every operation may mutate session state under
/// the hood — and a uniform mutability boundary keeps the trait object
/// signature simple for the call sites.
///
/// Errors are returned as `String` for now to keep the surface terse;
/// they are treated as opaque by the application layer and surfaced in
/// the TUI feedback strip.
pub trait KeybasePort {
    // ── Identity / session ────────────────────────────────────────────────

    /// Returns a snapshot of the current authentication state by
    /// running `keybase status --json`. The local `keybased` service is
    /// the source of truth — there is no separate session key the
    /// adapter has to carry around.
    fn status(&mut self) -> Result<IdentityInfo, KeybaseError>;

    /// Logs out of the current account: removes the local credentials
    /// and stops the cached session in the service. The user has to
    /// re-provision a device (or `keybase login` from a terminal) to
    /// come back.
    ///
    /// Distinct from a Bitwarden-style "lock": Keybase has no
    /// equivalent of a local session that can be locked without
    /// purging it.
    fn logout(&mut self) -> Result<(), KeybaseError>;

    /// Non-interactive **paper-key** login: `keybase login --devicename
    /// <device> <username>` with the paper key written to the child's
    /// stdin. This is the *only* scripted login the CLI supports (per
    /// `keybase login`'s own docs) and only works on a device that has
    /// **never** been provisioned for this account — a device that was
    /// merely logged out is still provisioned and rejects it with "already
    /// provisioned this device", in which case the interactive
    /// (passphrase) flow is required instead (the TUI cedes the terminal
    /// to `keybase login` for that). The paper key is secret material and
    /// must be zeroized by the caller.
    fn login_paperkey(
        &mut self,
        username: &str,
        device: &str,
        paperkey: &str,
    ) -> Result<(), KeybaseError>;

    // ── Chat — inbox / read ───────────────────────────────────────────────

    /// `{"method":"list"}` — returns every conversation the user can
    /// see, regardless of member-status. The TUI filters with the status
    /// unread state and groups them in the tree.
    ///
    /// Rows that fail to decode end up in [`ListConversationsOk::skipped`]
    /// rather than aborting the whole call: one corrupted row should
    /// not lock the user out of a 200-conversation inbox.
    fn list_conversations(&mut self) -> Result<ListConversationsOk, KeybaseError>;

    /// `{"method":"read","params":{"options":{"channel":...}}}` — reads
    /// up to `num` messages of a conversation, newest first.
    ///
    /// * `peek = true` does not mark the messages as read.
    /// * `next_cursor` is the opaque cursor returned by a previous
    ///   call. Pass [`None`] for the first (most recent) page; pass
    ///   the previous reply's cursor to paginate one page further
    ///   back into history.
    ///
    /// Returns the messages together with the cursor for the next
    /// older page, or [`None`] when the bottom of history has been
    /// reached (`pagination.last == true`).
    fn read_messages(
        &mut self,
        channel: &ReadChannel,
        num: u32,
        peek: bool,
        next_cursor: Option<&str>,
    ) -> Result<(Vec<Message>, Option<String>), KeybaseError>;

    /// `{"method":"mark","params":{"options":{"channel":...,"message_id":N}}}`.
    /// Marks the conversation read up to (and including) `message_id`.
    fn mark_read(&mut self, channel: &ReadChannel, message_id: u64) -> Result<(), KeybaseError>;

    /// `{"method":"searchinbox","params":{"options":{...}}}` — server-side
    /// inbox search. Returns the raw JSON as zeroized buffer.
    fn search_inbox(
        &mut self,
        query: &str,
        max_hits: u32,
    ) -> Result<Zeroizing<String>, KeybaseError>;

    /// `{"method":"searchregexp","params":{"options":{"channel":…,"query":…,
    /// "is_regex":false,"max_hits":N}}}` — server-side search *within one
    /// conversation* (full history). Returns the raw JSON as zeroized buffer.
    fn search_regexp(
        &mut self,
        channel: &ReadChannel,
        query: &str,
        max_hits: u32,
    ) -> Result<Zeroizing<String>, KeybaseError>;

    /// Typed wrapper around [`Self::search_regexp`]: decodes the standard
    /// `result.hits[]` shape into [`InboxHit`] rows (scoped to the open
    /// conversation, so `conv_id`/`conv_name` are left empty).
    fn search_regexp_hits(
        &mut self,
        channel: &ReadChannel,
        query: &str,
        max_hits: u32,
    ) -> Result<Vec<InboxHit>, KeybaseError> {
        let raw = self.search_regexp(channel, query, max_hits)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| KeybaseError::InvalidJson {
                family: "chat".into(),
                detail: e.to_string(),
            })?;
        let mut out = Vec::new();
        let Some(hits) = parsed
            .pointer("/result/hits")
            .and_then(serde_json::Value::as_array)
        else {
            return Ok(out);
        };
        for h in hits {
            let valid = h
                .pointer("/hitMessage/valid")
                .unwrap_or(&serde_json::Value::Null);
            let message_id = valid
                .get("messageID")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if message_id == 0 {
                continue;
            }
            let sender = valid
                .get("senderUsername")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let body_summary = valid
                .get("bodySummary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            // `ctime` is the send time in milliseconds → Unix seconds.
            let sent_at = valid
                .get("ctime")
                .and_then(serde_json::Value::as_u64)
                .map(|ms| ms / 1000)
                .unwrap_or(0);
            out.push(InboxHit {
                conv_id: String::new(),
                conv_name: String::new(),
                message_id,
                sender,
                body_summary,
                sent_at,
            });
        }
        Ok(out)
    }

    /// Typed wrapper around [`Self::search_inbox`] that decodes the
    /// JSON into [`InboxHit`] rows so the view layer doesn't have to
    /// know the wire format. Adapters that override [`Self::search_inbox`]
    /// can leave the default impl alone — it parses the standard
    /// `result.results.hits[].hits[]` shape.
    fn search_inbox_hits(
        &mut self,
        query: &str,
        max_hits: u32,
    ) -> Result<Vec<InboxHit>, KeybaseError> {
        let raw = self.search_inbox(query, max_hits)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| KeybaseError::InvalidJson {
                family: "chat".into(),
                detail: e.to_string(),
            })?;
        let mut out = Vec::new();
        let conv_hits = parsed
            .pointer("/result/results/hits")
            .and_then(serde_json::Value::as_array);
        let Some(conv_hits) = conv_hits else {
            return Ok(out);
        };
        for conv in conv_hits {
            // `searchinbox` returns the conversation id base64-encoded,
            // whereas `list` returns it as lowercase hex — normalise to hex
            // so hits re-key into the cached inbox (open-from-search).
            let raw_conv_id = conv
                .get("convID")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let conv_id =
                base64_conv_id_to_hex(raw_conv_id).unwrap_or_else(|| raw_conv_id.to_string());
            let conv_name = conv
                .get("convName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let Some(inner) = conv.get("hits").and_then(serde_json::Value::as_array) else {
                continue;
            };
            for h in inner {
                let valid = h
                    .pointer("/hitMessage/valid")
                    .unwrap_or(&serde_json::Value::Null);
                let message_id = valid
                    .get("messageID")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let sender = valid
                    .get("senderUsername")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let body_summary = valid
                    .get("bodySummary")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let sent_at = valid
                    .get("ctime")
                    .and_then(serde_json::Value::as_u64)
                    .map(|ms| ms / 1000)
                    .unwrap_or(0);
                out.push(InboxHit {
                    conv_id: conv_id.clone(),
                    conv_name: conv_name.clone(),
                    message_id,
                    sender,
                    body_summary,
                    sent_at,
                });
            }
        }
        Ok(out)
    }

    /// `{"method":"newconv","params":{"options":{"channel":...}}}` —
    /// creates a new conversation (or returns the existing one) for
    /// the given participant list. Returns the conversation id.
    fn new_conversation(&mut self, channel: &ReadChannel) -> Result<String, KeybaseError>;

    /// `{"method":"listconvsonname","params":{"options":{"topic_type":"CHAT",
    /// "members_type":"team","name":TEAM}}}` — lists **every channel of a
    /// team** (not just the ones you're in). Same result shape as `list`
    /// (`result.conversations[]` of `ConvSummary`), so tolerant parsing is
    /// reused; a channel's `member_status` tells joined (`Active`) from not.
    fn list_channels_on_name(&mut self, team: &str) -> Result<ListConversationsOk, KeybaseError>;

    /// `{"method":"join","params":{"options":{"channel":...}}}` — joins a team
    /// channel (`channel` = team name + `members_type:"team"` + `topic_name`).
    fn join_channel(&mut self, channel: &ReadChannel) -> Result<(), KeybaseError>;

    /// `{"method":"leave","params":{"options":{"channel":...}}}` — leaves a
    /// team channel.
    fn leave_channel(&mut self, channel: &ReadChannel) -> Result<(), KeybaseError>;

    /// `{"method":"listmembers","params":{"options":{"channel":...}}}` — the
    /// members of a conversation / team channel, from the `ChatMembersDetails`
    /// role buckets (owners/admins/writers/readers/bots/restrictedBots),
    /// flattened to [`ChatMember`]s.
    fn list_members(&mut self, channel: &ReadChannel) -> Result<Vec<ChatMember>, KeybaseError>;

    /// `{"method":"addtochannel","params":{"options":{"channel":...,
    /// "usernames":[...]}}}` — adds team members to a channel.
    fn add_to_channel(
        &mut self,
        channel: &ReadChannel,
        usernames: &[String],
    ) -> Result<(), KeybaseError>;

    /// `{"method":"removefromchannel","params":{"options":{"channel":...,
    /// "usernames":[...]}}}` — removes members from a channel.
    fn remove_from_channel(
        &mut self,
        channel: &ReadChannel,
        usernames: &[String],
    ) -> Result<(), KeybaseError>;

    /// `keybase chat rename-channel <team> <old> <new>` — renames a team
    /// channel. A **CLI subcommand** (no API method), so a one-shot spawn.
    fn rename_channel(&mut self, team: &str, old: &str, new: &str) -> Result<(), KeybaseError>;

    /// `keybase chat delete-channel <team> <channel>` — deletes a channel.
    /// CLI subcommand (one-shot); **non-interactive** (verified: resolves
    /// non-interactively + `DeleteConversationLocal`, no prompt). Destructive +
    /// irreversible, so the caller confirms first.
    fn delete_channel(&mut self, team: &str, channel: &str) -> Result<(), KeybaseError>;

    /// `keybase chat default-channels <team> [--channel C]…` — the team's
    /// **default channels** (new members auto-join these). A CLI subcommand
    /// (one-shot) whose output is plain text, not JSON. With `set` empty it
    /// **gets**; with `set` non-empty it **replaces** the default set with
    /// exactly those channels, then prints the result. Either way returns the
    /// resulting default channel names, **excluding the implicit `#general`**
    /// (always default, not settable). Set requires team-admin rights.
    fn default_channels(&mut self, team: &str, set: &[String])
    -> Result<Vec<String>, KeybaseError>;

    /// `{"method":"setstatus","params":{"options":{"channel":...,"status":STATUS}}}`.
    /// `status` is one of `"unfiled"`, `"favorite"`, `"muted"`,
    /// `"ignored"`. Used to mute / unmute (`"muted"` / `"unfiled"`).
    fn set_conversation_status(
        &mut self,
        channel: &ReadChannel,
        status: &str,
    ) -> Result<(), KeybaseError>;

    /// `{"method":"pin","params":{...}}` — pins a single message.
    fn pin_message(&mut self, channel: &ReadChannel, message_id: u64) -> Result<(), KeybaseError>;

    /// Fetches a single message by id (`keybase chat api {"method":"get"}`,
    /// same `result.messages[].msg` shape as `read`). Returns `Ok(None)`
    /// when the server doesn't return it (deleted / never existed). Used to
    /// resolve a pinned message that is older than the loaded window.
    fn get_message(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
    ) -> Result<Option<Message>, KeybaseError>;

    /// `{"method":"unpin","params":{...}}` — clears the channel's pin.
    fn unpin_message(&mut self, channel: &ReadChannel) -> Result<(), KeybaseError>;

    /// `{"method":"download","params":{...}}` — downloads the
    /// attachment carried by `message_id` to the local `output`
    /// path. The Keybase service streams the file directly to disk;
    /// we do not buffer it.
    fn download_attachment(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
        output: &str,
    ) -> Result<(), KeybaseError>;

    // ── Chat — write ──────────────────────────────────────────────────────

    /// `{"method":"send","params":{"options":{"channel":...,"message":{"body":...}}}}`.
    /// `reply_to` is an optional message id for threaded replies.
    fn send_message(
        &mut self,
        channel: &ReadChannel,
        body: &str,
        reply_to: Option<u64>,
    ) -> Result<(), KeybaseError>;

    /// `{"method":"edit","params":{...}}` — overwrite a previously
    /// sent message.
    fn edit_message(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
        body: &str,
    ) -> Result<(), KeybaseError>;

    /// `{"method":"delete","params":{...}}`.
    fn delete_message(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
    ) -> Result<(), KeybaseError>;

    /// `{"method":"reaction","params":{...}}` — `body` is the reaction
    /// content (e.g. `:+1:`).
    fn react(
        &mut self,
        channel: &ReadChannel,
        message_id: u64,
        body: &str,
    ) -> Result<(), KeybaseError>;

    /// `{"method":"attach","params":{...}}` — upload a local file as an
    /// attachment to the conversation. `title` is an optional caption
    /// (empty string ⇒ keybase defaults it to the filename).
    fn upload_attachment(
        &mut self,
        channel: &ReadChannel,
        filename: &str,
        title: &str,
    ) -> Result<(), KeybaseError>;

    /// `{"method":"emojilist"}` — the emojis the user can send (stock +
    /// the team's custom ones), for the reaction picker.
    fn list_emojis(&mut self) -> Result<Vec<Emoji>, KeybaseError>;

    // ── Teams ─────────────────────────────────────────────────────────────

    /// `{"method":"list-self-memberships"}`.
    ///
    /// Tolerant of malformed rows in the same way as
    /// [`Self::list_conversations`].
    /// The teams the local user belongs to — one row per team, with the
    /// user's role and the team's member count.
    ///
    /// NB: it deliberately does **not** use the `list-self-memberships` API
    /// method — that maps to `TeamListTeammates`, which returns one row per
    /// *teammate* across every team (including an implicit team per DM), i.e.
    /// thousands of duplicate rows. Instead it queries `list-user-memberships`
    /// with the user's own `username` (`TeamListUnverified`), which returns one
    /// `AnnotatedMemberInfo` per real team (implicit teams excluded). Same
    /// `result.teams[]` shape, so parsing is unchanged.
    fn list_self_memberships(&mut self, username: &str) -> Result<ListTeamsOk, KeybaseError>;
}

/// Channel descriptor for read/write methods.
///
/// Mirrors the JSON shape `keybase chat api` expects:
///
/// ```json
/// {"name":"alice,bob","members_type":"impteamnative"}
/// // or
/// {"name":"phoenix","members_type":"team","topic_name":"general"}
/// ```
#[derive(Debug, Clone)]
pub struct ReadChannel {
    /// `"alice,bob"` for impteam, team name for explicit teams.
    pub name: String,
    /// `"impteamnative"`, `"impteamupgrade"`, `"team"`, `"kbfs"`.
    pub members_type: String,
    /// Sub-channel inside a team (e.g. `general`). Required for team
    /// conversations.
    pub topic_name: Option<String>,
}

/// Decodes a standard-base64 conversation id (as `searchinbox` returns it)
/// to the lowercase hex form `list` uses. Returns `None` on any non-base64
/// input so the caller can fall back to the raw string.
fn base64_conv_id_to_hex(s: &str) -> Option<String> {
    fn sextet(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let s = s.trim_end_matches('=');
    if s.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        buf = (buf << 6) | sextet(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buf >> bits) as u8);
        }
    }
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}

#[cfg(test)]
mod tests {
    use super::base64_conv_id_to_hex;

    #[test]
    fn base64_conv_id_decodes_to_list_hex() {
        // Real pair observed from `searchinbox` (base64) vs `list` (hex).
        assert_eq!(
            base64_conv_id_to_hex("AABi56bFVVvd3i3UnNPWSU6Zd19TSrVsSVZLwoktPgM=").as_deref(),
            Some("000062e7a6c5555bddde2dd49cd3d6494e99775f534ab56c49564bc2892d3e03"),
        );
    }

    #[test]
    fn base64_conv_id_rejects_garbage() {
        assert_eq!(base64_conv_id_to_hex(""), None);
        assert_eq!(base64_conv_id_to_hex("not valid!!"), None);
    }
}
