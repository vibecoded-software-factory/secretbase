//! Global TUI state container.
//!
//! `App` owns the entire visible state of the TUI: which screen is
//! active, which conversation is selected, the search query, the
//! injected ports, etc. Sub-modules ([`flows`](crate::tui::flows),
//! [`input`](crate::tui::input), [`view`](crate::tui::view)) read and
//! mutate `App` through `&mut App`.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use crate::domain::{
    ChatEvent, Conversation, IdentityInfo, LineEditor, LoweredConversation, MemberStatus,
    fuzzy_score_lowered,
};
use crate::ports::{ClipboardPort, OpenerPort, SettingsPort, UserSettings};
use crate::tui::action::{ActionState, CmdEntry};
use crate::tui::file_picker::FilePicker;
use crate::tui::mouse_areas::MouseAreas;
use crate::tui::screens::{Focus, Screen};
use crate::tui::theme::{self, Theme};
use crate::tui::worker::{InFlight, WorkerRequest, WorkerResponse};

/// Step size in rows for PgUp/PgDn navigation.
pub const PAGE_STEP: usize = 10;

/// A status-changing action on a single conversation, shown behind the
/// shared y/n confirm popup ([`Screen::ConfirmConvAction`]). Each maps to
/// a `keybase chat api {"method":"setstatus"}` status value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvAction {
    /// Hide from the inbox until a new message arrives (`ignored`).
    Ignore,
    /// Block and remove from the inbox (`blocked`). Reversible via the
    /// by-name **Unhide** popup (`unfiled`) since blocked convs leave `list`.
    Block,
    /// Report to Keybase and remove from the inbox (`reported`). Reversible
    /// via the by-name **Unhide** popup.
    Report,
}

impl ConvAction {
    /// The keybase `setstatus` value (a `chat1.ConversationStatus`).
    pub fn status(self) -> &'static str {
        match self {
            ConvAction::Ignore => "ignored",
            ConvAction::Block => "blocked",
            ConvAction::Report => "reported",
        }
    }
    /// Spinner label while the call runs.
    pub fn running(self) -> &'static str {
        match self {
            ConvAction::Ignore => "Ignoring…",
            ConvAction::Block => "Blocking…",
            ConvAction::Report => "Reporting…",
        }
    }
    /// Feedback label on success.
    pub fn done(self) -> &'static str {
        match self {
            ConvAction::Ignore => "Ignored",
            ConvAction::Block => "Blocked",
            ConvAction::Report => "Reported",
        }
    }
    /// Confirm-popup title.
    pub fn title(self) -> &'static str {
        match self {
            ConvAction::Ignore => " Ignore conversation? ",
            ConvAction::Block => " Block conversation? ",
            ConvAction::Report => " Report conversation? ",
        }
    }
    /// One-line description of the effect, shown in the confirm popup.
    pub fn note(self) -> &'static str {
        match self {
            ConvAction::Ignore => "Hides it from the inbox until a new message arrives.",
            ConvAction::Block => {
                "Blocks it and removes it from your inbox. Restore later with Alt+H (by name)."
            }
            ConvAction::Report => {
                "Reports it to Keybase and removes it from your inbox. Restore with Alt+H."
            }
        }
    }
}

pub use crate::tui::login_state::{LoginField, LoginState};
pub use crate::tui::outbox::{PendingSend, SendState};
pub use crate::tui::settings_model::*;
// Re-exported so existing `app::AlignReq` / `app::PendingBatch` paths keep
// working after these moved into their cohesive home.
pub use crate::tui::select_state::{AlignReq, PendingBatch, SelectState};

/// Quotes `v` as a TOML string value for the settings writer. The reader
/// (`settings_toml`'s `unquote`) does **not** process escape sequences, so
/// instead of escaping, characters that would corrupt the config line
/// (quotes, backslashes, control chars) are stripped. Today's callers only
/// pass hex conversation ids — which never contain them — so this is a
/// guard against a future caller widening the value domain, not a change
/// in behaviour.
fn toml_quoted(v: &str) -> String {
    let clean: String = v
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .collect();
    format!("\"{clean}\"")
}

/// What the open [`crate::tui::file_picker::FilePicker`] is for — set when
/// the picker opens, read when it returns a path.
#[derive(Debug, Clone)]
pub enum PickerAction {
    /// Upload the picked file as an attachment to the open conversation.
    Upload,
    /// Download attachment `message_id` into the picked directory, saved
    /// under `filename`.
    Download { message_id: u64, filename: String },
}

/// A row in the quick switcher: a non-selectable section header, or a
/// conversation (index into `conversations`).
pub enum SwitcherRow {
    Header(&'static str),
    Conv(usize),
}

/// A row of the inbox conversation tree: a collapsible group header
/// (Direct messages / a team) or a conversation leaf.
#[derive(Clone)]
pub enum TreeRow {
    /// Group header. `key` is the collapse key ([`App::DMS_KEY`] or the team
    /// name); `unread` is the count of unread conversations in the group.
    Group {
        key: String,
        label: String,
        is_team: bool,
        collapsed: bool,
        unread: usize,
        /// Any member conversation carries an unseen @mention — so a
        /// collapsed group can still pull the eye to it.
        mentioned: bool,
    },
    /// A conversation leaf — index into [`App::conversations`].
    Conv { idx: usize },
}

/// Top-level mutable state of the TUI.
pub struct App {
    // ── Screen / focus / filter ───────────────────────────────────────────
    pub screen: Screen,
    pub focus: Focus,
    /// **Expanded** tree groups (by key: [`Self::DMS_KEY`] or a team name).
    /// Empty = everything collapsed, so the tree starts fully folded and a
    /// user's expansions survive inbox refreshes.
    pub expanded: HashSet<String>,
    /// Cursor into [`Self::tree_rows`] — the selected tree row.
    pub tree_selected: usize,

    // ── Identity (from `keybase status --json`) ───────────────────────────
    pub identity: IdentityInfo,

    // ── Inbox data ────────────────────────────────────────────────────────
    /// All conversations returned by `keybase chat api {"method":"list"}`.
    pub conversations: Vec<Conversation>,
    /// Pre-lowercased projection kept parallel to `conversations` so
    /// search doesn't re-allocate on every keystroke.
    pub conversations_lowered: Vec<LoweredConversation>,
    /// Indices into `conversations` after filter + search are applied.
    pub filtered_cache: Vec<usize>,
    /// Cached flattened tree rows (groups + visible conversation leaves) —
    /// see [`Self::rebuild_tree_rows`]. Read per frame via
    /// [`Self::tree_rows`]; rebuilt only when an input changes.
    pub tree_rows_cache: Vec<TreeRow>,
    /// First visible row in the conversation tree — driven by scrolling.
    pub list_scroll: usize,
    /// Cursor into `filtered_cache` for the **Find a conversation** landing —
    /// the spacious search+list the right pane shows when no chat is open. Its
    /// own cursor (not `tree_selected`, which indexes the grouped tree rows).
    pub find_selected: usize,
    /// Error from the **last inbox `list`** — kept so the empty tree shows a
    /// persistent "couldn't load, retry" state (the feedback toast expires
    /// after ~1.5 s, leaving nothing but the command log otherwise). `None`
    /// after a successful load.
    pub inbox_error: Option<String>,

    /// The Teams screen's state — the loaded memberships plus the list's
    /// cursor, scroll and `/` filter — extracted into its own cohesive type
    /// (see [`crate::tui::teams_state`]). The `list-user-memberships` load
    /// lives in the flow layer (it needs the worker).
    pub teams: crate::tui::teams_state::TeamsState,

    // ── Channel browser (`c` on a team) ───────────────────────────────
    /// The channel browser modal's state — the team being browsed, its
    /// channels and cursor, the inline create / rename / delete / filter
    /// modes, and the team's default-channel set — extracted into its own
    /// cohesive type (see [`crate::tui::channel_browser_state`]). The
    /// `listconvsonname` / `join` / `leave` / `newconv` / rename / delete /
    /// default-channels calls live in the flow layer (they need the worker).
    pub channel_browser: crate::tui::channel_browser_state::ChannelBrowserState,

    // ── Members view (Screen::Members) ──────────────────────────────────
    /// The Members modal's state — its target channel + label, the member
    /// list and cursor, the return screen, and the inline add / filter /
    /// remove-confirm modes — extracted into its own cohesive type (see
    /// [`crate::tui::members_state`]). The `listmembers` / `addtochannel` /
    /// `removefromchannel` calls live in the flow layer (they need the worker).
    pub members: crate::tui::members_state::MembersState,

    // ── Conversation detail ──────────────────────────────────────────────
    /// Conversation id currently open on the detail screen. `None`
    /// while we're on the inbox screen.
    pub open_conv_id: Option<String>,
    /// The loaded conversation thread — the open conversation's messages plus
    /// the projections derived from them (the id→index lookup and the
    /// topic/headline) — extracted into its own cohesive type (see
    /// [`crate::tui::thread_state`]). `rebuild_msg_meta` reprojects it and the
    /// render-cache epoch stays on `App` (it's cross-cutting).
    pub thread: crate::tui::thread_state::ThreadState,
    /// The open conversation's message-viewport state — the scroll offset,
    /// the older-page cursor and the derived scroll/pagination cues (max-back,
    /// new-since-scroll, the new-messages divider anchor) — extracted into its
    /// own cohesive type (see [`crate::tui::pagination_state`]). The
    /// read-handler pagination flows stay in the flow layer.
    pub pagination: crate::tui::pagination_state::PaginationState,
    /// The pinned-message subsystem's state — the open conversation's pin
    /// banner projection plus the local/persisted bookkeeping the JSON API
    /// can't give us — extracted into its own cohesive type (see
    /// [`crate::tui::pin_state`]). The persisting `set_local_pin` /
    /// `set_pin_dismissed` and the background pin-body fetch stay outside it.
    pub pins: crate::tui::pin_state::PinState,
    /// Giphy GIF-search modal state (`Alt+G`) — see [`crate::tui::search_overlays`].
    pub giphy: crate::tui::search_overlays::GiphyState,
    /// When true, the emoji picker (`Screen::React`) inserts the chosen
    /// emoji into the **compose draft** instead of reacting to a message —
    /// the compose bar's emoji button / `Alt+I`.
    pub react_to_compose: bool,
    /// Invalidation epoch for the per-message rendered-lines cache
    /// (`view::conversation`). Bumped by
    /// [`Self::invalidate_msg_render_cache`] whenever an input that feeds a
    /// message's rendered block changes outside the block itself: the
    /// history (via [`Self::rebuild_msg_meta`]), the theme, a setting, or
    /// the emoji catalogue.
    pub msg_cache_epoch: u64,
    /// **Session-local** highest message id already *seen* per conversation
    /// (keyed by conv id). Recorded when a conversation is left / switched away
    /// from; on the next open it seeds [`Self::pagination`]'s `unread_boundary`
    /// so the message stream can draw a `new messages` divider above anything that
    /// arrived since. Not persisted (a fresh run starts with no baseline).
    pub conv_last_seen: HashMap<String, u64>,
    /// Optimistic send queue: messages shown immediately and tracked
    /// through `Pending → Delivered → Failed`. Kept separate from
    /// `messages` so a re-read (which replaces `messages` wholesale)
    /// never drops a still-pending or failed send. Rendered below the
    /// loaded history for the conversation each entry targets.
    pub outbox: Vec<PendingSend>,
    /// Open embedded file picker (attachment upload). When `Some`, it
    /// captures all input and renders as a modal overlay.
    pub file_picker: Option<FilePicker>,

    // ── Compose ──────────────────────────────────────────────────────────
    /// Whether the compose pane is open (user is typing a new message).
    pub compose_open: bool,
    /// Current draft, held in the shared [`LineEditor`] so every text
    /// input across the app edits identically (UTF-8-safe cursor,
    /// mid-string editing).
    pub compose: LineEditor,
    /// If `Some`, the compose buffer is editing an existing message
    /// instead of composing a new one. Submitting fires
    /// `PendingAction::SaveEdit` against this id.
    pub edit_target_id: Option<u64>,
    /// If `Some`, the next send is a threaded reply to this message
    /// id (the `reply_to` field of `keybase chat api send`). Cleared
    /// after a successful send and whenever the draft is wiped.
    pub reply_to_id: Option<u64>,

    // ── Conversation-screen interaction mode ─────────────────────────────
    /// Currently selected message inside the loaded history. `None`
    /// when the user is in Compose mode (default). When `Some`, the
    /// conversation screen is in [`ConvMode::Select`] and single-key
    /// shortcuts (`e`/`d`/`:`/`p`) trigger message actions.
    /// The conversation's Select-mode state — the multi-select cursor,
    /// marked ids, visual anchor, `zz`/`zt`/`zb` align request, and the
    /// sequential delete/react batch — extracted into its own cohesive type
    /// (see [`crate::tui::select_state`]). The batch orchestration stays in
    /// the flow layer (the serial worker advances it).
    pub select: crate::tui::select_state::SelectState,
    /// Candidate usernames for `@`-mention autocomplete in the open
    /// conversation (participants + people who've spoken). Rebuilt on
    /// open / message-load (`rebuild_conv_members`).
    pub conv_members: Vec<String>,
    /// Selected row in the `@`-mention autocomplete popup.
    pub mention_selected: usize,

    /// Search query in the reaction picker — also doubles as a custom
    /// `:shortcode:` if it matches no listed emoji. Separate from
    /// [`Self::compose`] so the message draft is preserved.
    pub react: LineEditor,
    /// Selected row in the reaction picker (indexes the *filtered* emoji
    /// list — see [`crate::tui::emoji_catalog::EmojiCatalog::filtered`]).
    pub react_selected: usize,
    /// The emoji picker's catalogue, lookup index and frecency ranking —
    /// extracted into its own cohesive type (see
    /// [`crate::tui::emoji_catalog`]). The `emojilist` fetch that fills the
    /// team's custom emojis lives in the flow layer (it needs the worker),
    /// and the picker query stays on [`Self::react`]; see
    /// [`Self::rebuild_emoji_filter`].
    pub emoji: crate::tui::emoji_catalog::EmojiCatalog,

    // ── Quick switcher (Ctrl+K) ──────────────────────────────────────────
    /// `@`-token whose mention popup was Esc-dismissed — the popup stays
    /// closed while the token under the cursor still matches it.
    pub mention_dismissed_token: Option<String>,
    /// Cursor in the `:emoji:` autocomplete popup.
    pub emoji_ac_selected: usize,
    /// `:token` whose emoji popup was Esc-dismissed (same contract as
    /// [`Self::mention_dismissed_token`]).
    pub emoji_ac_dismissed_token: Option<String>,
    /// The `Ctrl+K` quick-switcher overlay's state — the fuzzy query, the
    /// selected row and the return screen — extracted into its own type (see
    /// [`crate::tui::switcher_state`]). The row projections stay `App` methods.
    pub switcher: crate::tui::switcher_state::SwitcherState,
    /// Per-conversation unsent draft text (in memory only — not persisted
    /// across restarts). Keyed by conversation id.
    pub drafts: HashMap<String, String>,

    // ── Command palette (Ctrl+P) ─────────────────────────────────────────
    /// The `Ctrl+P` command-palette overlay's state — the fuzzy query, the
    /// selected row and the return screen — extracted into its own type (see
    /// [`crate::tui::palette_state`]). The command list stays in `flows::palette`.
    pub palette: crate::tui::palette_state::PaletteState,
    /// When opening a conversation from a global-search hit, the message id
    /// to scroll to + highlight once it's loaded (paginating older if the
    /// match is below the first page). Cleared once landed or exhausted.
    pub pending_search_jump: Option<u64>,

    // ── In-conversation search (Ctrl+F → keybase chat api searchregexp) ──
    /// `Ctrl+F` in-conversation search modal state — see
    /// [`crate::tui::search_overlays`].
    pub conv_search: crate::tui::search_overlays::ConvSearchState,

    // ── New-conversation popup ──────────────────────────────────────────
    /// Comma-separated usernames typed by the user in the Alt+N popup.
    pub new_conv: LineEditor,

    // ── Unhide popup (restore a blocked/reported conv by name) ──────────
    /// Username(s) typed in the Alt+H **Unhide** popup. Blocked/reported convs
    /// leave the inbox `list`, so they're restored (`setstatus unfiled`) by
    /// name instead of by tree selection.
    pub unhide_input: LineEditor,

    // ── Login screen (signed-out) ───────────────────────────────────────
    /// The signed-out Login form's state — the three fields, focus and the
    /// paper-key reveal toggle — extracted into its own cohesive type (see
    /// [`crate::tui::login_state`]). The login flows stay in `flows::auth`.
    pub login: crate::tui::login_state::LoginState,
    /// Set by the Login handler to ask the run loop to **cede the terminal**
    /// to interactive `keybase login <username>` (empty = no username arg).
    /// The run loop consumes it, suspends the TUI, runs the command, restores,
    /// and re-checks status.
    pub pending_native_login: Option<String>,
    /// Set by `Ctrl+E` in the compose: the run loop cedes the terminal to
    /// `$VISUAL`/`$EDITOR` with the draft in a 0600 temp file (wiped after)
    /// and reads the result back into the compose.
    pub pending_editor_compose: bool,

    // ── Server-side search popup ────────────────────────────────────────
    /// `Ctrl+G` server-side inbox search modal state — see
    /// [`crate::tui::search_overlays`].
    pub global_search: crate::tui::search_overlays::GlobalSearchState,

    /// What the open `file_picker` will do with the chosen path (upload a
    /// file, or download an attachment into the chosen directory).
    pub picker_action: PickerAction,

    // ── Search ────────────────────────────────────────────────────────────
    /// Incremental inbox filter box.
    pub search: LineEditor,

    // ── Help overlay ──────────────────────────────────────────────────────
    /// Screen the help overlay was opened from — used to scope the help
    /// content and to return there on close.
    pub help_from: Screen,
    /// Vertical scroll offset of the help overlay (rows). Clamped by the
    /// renderer against the real content/viewport overflow.
    pub help_scroll: u16,

    // ── Confirm overlays (navigable y/n) ──────────────────────────────────
    /// Whether "confirm" is the highlighted button in the logout
    /// overlay. Defaults to `false` (cancel highlighted) for the
    /// destructive action.
    pub logout_yes: bool,
    /// Whether "confirm" is highlighted in the delete-message overlay.
    pub delete_msg_yes: bool,
    /// Pending conversation action awaiting confirmation (ignore/…).
    pub pending_conv_action: Option<ConvAction>,
    /// Whether "confirm" is highlighted in the conversation-action overlay.
    pub conv_action_yes: bool,

    // ── Action queue / feedback strip ─────────────────────────────────────
    pub action_state: ActionState,
    pub action_tick: u8,
    /// Caller-side context for the request the worker is currently
    /// processing. `None` when idle. Set by `flows::*::request_*` and
    /// cleared by [`crate::tui::flows::apply_response`] once the
    /// matching response arrives.
    pub in_flight: Option<InFlight>,
    /// The command-log panel's state — the entry ring buffer, its scroll,
    /// and the visual-multi-select cursor / marks / anchor — extracted into
    /// its own cohesive type (see [`crate::tui::cmdlog_state`]). Appending
    /// stays [`Self::push_cmd`], which bridges to `last_op_elapsed`.
    pub cmdlog: crate::tui::cmdlog_state::CmdLogState,
    /// `Ctrl+W` window-nav leader is armed — the next key is read as a
    /// direction (`h/j/k/l` or an arrow) to move between panels positionally.
    pub pending_pane_nav: bool,
    /// Last OSC terminal title we set — avoids re-emitting the escape on
    /// every frame. Session-local.
    pub last_term_title: String,
    /// `Ctrl+W z` — the chat column takes the whole Home (tree + command
    /// log hidden) until toggled back. tmux's prefix+z, on our pane leader.
    pub pane_zoomed: bool,

    // ── Settings / theme ──────────────────────────────────────────────────
    pub settings_cache: UserSettings,
    pub theme: Theme,
    /// **Local-only** favourited conversation ids (fast lookup), synced from
    /// `settings_cache.favorites`. Our own star — never touches Keybase's
    /// `favorite` status (which the CLI can't read back). Fully owned and
    /// persisted by us. See [`Self::toggle_favorite`].
    pub favorites: HashSet<String>,
    /// **Local-only** muted conversation ids. Like [`Self::favorites`], purely
    /// ours: muting suppresses the unread indicators secretbase renders (no `●`,
    /// no bold, excluded from the unread count + filter), since the TUI has no
    /// notifications to silence. See [`Self::toggle_muted`] / [`Self::conv_is_unread`].
    pub muted: HashSet<String>,

    // ── Settings overlay (F10) ─────────────────────────────────────────────
    /// The Settings overlay's navigation + secret-editing state — which
    /// pane/section/row has focus, the live theme-picker index, the return
    /// screen and the secret-input popup — extracted into its own cohesive
    /// type (see [`crate::tui::settings_ui_state`]). The applying methods
    /// (`settings_adjust`, `apply_theme_idx`, …) stay on `App`.
    pub settings_ui: crate::tui::settings_ui_state::SettingsUiState,

    // ── Lifecycle ─────────────────────────────────────────────────────────
    pub should_quit: bool,
    /// Set once the worker response channel reports `Disconnected` (every
    /// worker thread gone) so the failure is surfaced a single time.
    pub worker_dead: bool,
    /// The conversation open before the current one — `Ctrl+O` toggles back
    /// to it (vim's `Ctrl+^` alternate buffer). Session-local.
    pub prev_conv_id: Option<String>,
    /// Conversations with an unseen **@mention of you** (session-local —
    /// the inbox `list` carries no mention state, same trade-off as
    /// `conv_last_seen`). Set by the push path, cleared on open; renders
    /// as a red `@` in the tree.
    pub mentioned: HashSet<String>,
    /// Whether the `api-listen` push stream is currently down (its
    /// supervisor is reconnecting with backoff) — surfaced as a dim `⇅`
    /// badge on the status strip. Cleared by the next event that arrives.
    pub listener_down: bool,
    /// Persistent boot-failure notice for the Login screen (e.g. "keybase
    /// binary not found") — the toast expires in seconds, but a form that
    /// can never succeed needs the real cause on screen. Cleared when a
    /// status retry succeeds.
    pub boot_error: Option<String>,
    pub last_activity: Instant,
    /// Wall-clock timestamp of the last inbox load — used by the run
    /// loop's auto-refresh hook.
    pub last_inbox_load: Instant,

    // ── Mouse ─────────────────────────────────────────────────────────────
    pub mouse_areas: MouseAreas,
    /// `(width, height)` of the terminal observed at the top of the
    /// most recent run-loop iteration. Compared against
    /// [`MouseAreas::frame_size`] to discard clicks whose
    /// coordinates predate the latest resize.
    pub last_terminal_size: (u16, u16),

    // ── Worker channels (keybase lives in another thread) ─────────────────
    /// Send half — `flows::*::request_*` push [`WorkerRequest`]s here.
    pub worker_tx: Sender<WorkerRequest>,
    /// Send half for the **background** lane (idle inbox auto-refresh)
    /// so it never occupies the user's `in_flight` slot.
    pub bg_worker_tx: Sender<WorkerRequest>,
    /// Whether a background (silent) refresh is in flight. Tracked
    /// separately from `in_flight` so `busy_blocks` never gates input
    /// because of it.
    pub bg_inflight: bool,
    /// When the current user request was started (set by [`Self::begin`]),
    /// used to time the operation for the command log.
    pub request_started: Option<Instant>,
    /// When the current background refresh was started.
    pub bg_started: Option<Instant>,
    /// When the background emoji-catalogue fetch was started — its own slot
    /// so it can't clobber `bg_started` if a silent refresh overlaps; used
    /// to time the `emojilist` command-log row like every other op.
    pub emojis_started: Option<Instant>,
    /// Elapsed time of the just-completed operation, stamped onto the
    /// next [`Self::push_cmd`] entry. Set by `flows::apply_response`
    /// right before dispatching the response handler.
    pub last_op_elapsed: Option<Duration>,
    /// Receive half — the run loop polls this each tick.
    pub worker_rx: Receiver<WorkerResponse>,

    /// Push events from the `keybase chat api-listen` stream, drained
    /// each frame for real-time inbox + open-conversation updates. `None`
    /// when the listener couldn't spawn (e.g. logged out at launch).
    pub chat_rx: Option<Receiver<ChatEvent>>,

    // ── Inline image attachments (chafa / kitty …) ────────────────────────
    /// All inline image / animated-GIF rendering state — the download and
    /// GIF-decode lifecycles plus the per-frame paint flags. Extracted into
    /// its own cohesive type (see [`crate::tui::image_pipeline`]) so this one
    /// concern has one place to change; cache paths are keyed by the on-disk
    /// `{conv}-{msg}.ext` (not the per-conversation message id, which would
    /// collide across chats).
    pub images: crate::tui::image_pipeline::ImagePipeline,

    // ── Injected ports (synchronous, stay on the render thread) ───────────
    pub clipboard: Box<dyn ClipboardPort>,
    pub opener: Box<dyn OpenerPort>,
    pub settings: Box<dyn SettingsPort>,
}

/// The current interaction mode, shown as an nvim-style `-- MODE --` badge in
/// the status strip so the user always knows what a keystroke will do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiMode {
    /// Navigating a list — letters/keys act (or move), nothing is typed.
    Normal,
    /// Typing a chat message.
    Compose,
    /// Multi-selecting messages to act on them.
    Select,
    /// Typing into a search / filter box.
    Search,
}

impl UiMode {
    pub fn label(self) -> &'static str {
        match self {
            UiMode::Normal => "NORMAL",
            UiMode::Compose => "COMPOSE",
            UiMode::Select => "SELECT",
            UiMode::Search => "SEARCH",
        }
    }
}

impl App {
    /// Builds the initial state. The composition root (`main.rs`) is
    /// the only intended caller.
    ///
    /// `worker_tx` / `worker_rx` are the two halves of the channel
    /// pair owned by [`crate::tui::worker::WorkerHandle`]. App holds
    /// the send half (used by `request_*` helpers) and the receive
    /// half (drained by the run loop).
    pub fn new(
        worker_tx: Sender<WorkerRequest>,
        bg_worker_tx: Sender<WorkerRequest>,
        worker_rx: Receiver<WorkerResponse>,
        chat_rx: Option<Receiver<ChatEvent>>,
        clipboard: Box<dyn ClipboardPort>,
        opener: Box<dyn OpenerPort>,
        settings: Box<dyn SettingsPort>,
    ) -> Self {
        let settings_cache = settings.read();
        let images = crate::tui::image_pipeline::ImagePipeline::from_settings(&settings_cache);
        let favorites: HashSet<String> = settings_cache.favorites.iter().cloned().collect();
        // Persisted pin targets ("convid:msgid" pairs) — malformed entries
        // are skipped, same tolerance as every other config read.
        let pinned_local: HashMap<String, u64> = settings_cache
            .pins
            .iter()
            .filter_map(|p| {
                let (conv, msg) = p.split_once(':')?;
                Some((conv.to_string(), msg.trim().parse().ok()?))
            })
            .collect();
        let pins_dismissed: HashMap<String, u64> = settings_cache
            .pins_dismissed
            .iter()
            .filter_map(|p| {
                let (conv, msg) = p.split_once(':')?;
                Some((conv.to_string(), msg.trim().parse().ok()?))
            })
            .collect();
        // The two persisted maps seed the pin subsystem; the rest is derived
        // from the loaded history at runtime.
        let pins = crate::tui::pin_state::PinState {
            local: pinned_local,
            dismissed: pins_dismissed,
            ..Default::default()
        };
        let muted: HashSet<String> = settings_cache.muted.iter().cloned().collect();
        let theme = theme::load(&settings.config_dir());
        // Preselect the picker on the configured preset, else the shared
        // default (Nord).
        let settings_theme_idx = theme::configured_preset(&settings.config_dir())
            .or(Some(theme::Preset::DEFAULT))
            .and_then(|p| theme::Preset::ALL.iter().position(|&q| q == p))
            .unwrap_or(0);
        let settings_ui = crate::tui::settings_ui_state::SettingsUiState {
            theme_idx: settings_theme_idx,
            ..Default::default()
        };
        Self {
            screen: Screen::Splash,
            focus: Focus::Tree,
            expanded: HashSet::new(),
            tree_selected: 0,
            find_selected: 0,
            identity: IdentityInfo::default(),
            conversations: Vec::new(),
            conversations_lowered: Vec::new(),
            filtered_cache: Vec::new(),
            tree_rows_cache: Vec::new(),
            list_scroll: 0,
            inbox_error: None,
            teams: crate::tui::teams_state::TeamsState::default(),
            channel_browser: crate::tui::channel_browser_state::ChannelBrowserState::default(),
            members: crate::tui::members_state::MembersState::default(),
            open_conv_id: None,
            conv_last_seen: HashMap::new(),
            thread: crate::tui::thread_state::ThreadState::default(),
            outbox: Vec::new(),
            file_picker: None,
            pagination: crate::tui::pagination_state::PaginationState::default(),
            pins,
            giphy: crate::tui::search_overlays::GiphyState::default(),
            react_to_compose: false,
            msg_cache_epoch: 0,
            compose_open: false,
            compose: LineEditor::default(),
            edit_target_id: None,
            reply_to_id: None,
            select: crate::tui::select_state::SelectState::default(),
            conv_members: Vec::new(),
            mention_selected: 0,
            react: LineEditor::default(),
            react_selected: 0,
            // Seeds the bundled standard set (so the picker has content and
            // reactions resolve to glyphs immediately) and builds its index +
            // filter; the emojilist fetch merges the team's custom emojis on top.
            emoji: crate::tui::emoji_catalog::EmojiCatalog::new(),
            mention_dismissed_token: None,
            emoji_ac_selected: 0,
            emoji_ac_dismissed_token: None,
            switcher: crate::tui::switcher_state::SwitcherState::default(),
            drafts: HashMap::new(),
            palette: crate::tui::palette_state::PaletteState::default(),
            pending_search_jump: None,
            conv_search: crate::tui::search_overlays::ConvSearchState::default(),
            new_conv: LineEditor::default(),
            unhide_input: LineEditor::default(),
            login: crate::tui::login_state::LoginState::default(),
            pending_native_login: None,
            pending_editor_compose: false,
            global_search: crate::tui::search_overlays::GlobalSearchState::default(),
            picker_action: PickerAction::Upload,
            search: LineEditor::default(),
            help_from: Screen::Inbox,
            help_scroll: 0,
            logout_yes: false,
            delete_msg_yes: false,
            pending_conv_action: None,
            conv_action_yes: false,
            action_state: ActionState::Idle,
            action_tick: 0,
            in_flight: None,
            cmdlog: crate::tui::cmdlog_state::CmdLogState::default(),
            pending_pane_nav: false,
            last_term_title: String::new(),
            pane_zoomed: false,
            settings_cache,
            favorites,
            muted,
            images,
            theme,
            settings_ui,
            should_quit: false,
            worker_dead: false,
            prev_conv_id: None,
            boot_error: None,
            listener_down: false,
            mentioned: HashSet::new(),
            last_activity: Instant::now(),
            last_inbox_load: Instant::now(),
            mouse_areas: MouseAreas::default(),
            last_terminal_size: (0, 0),
            worker_tx,
            bg_worker_tx,
            bg_inflight: false,
            request_started: None,
            bg_started: None,
            emojis_started: None,
            last_op_elapsed: None,
            worker_rx,
            chat_rx,
            clipboard,
            opener,
            settings,
        }
    }

    /// Refilters the emoji catalogue against the live reaction-picker query.
    /// The thin bridge between the picker input (`react`, an `App` field) and
    /// the query-agnostic [`crate::tui::emoji_catalog::EmojiCatalog`]: it hands
    /// the query in so the coupling to the picker lives in exactly one place.
    /// Callers keep calling `app.rebuild_emoji_filter()` — the query plumbing
    /// stays here.
    pub fn rebuild_emoji_filter(&mut self) {
        let query = self.react.text().to_string();
        self.emoji.rebuild_filter(&query);
    }

    /// Conversations matching the quick-switcher query, as indices into
    /// `conversations`. Empty query → all, most-recent first; otherwise
    /// fuzzy-ranked over the lowered projection (same scorer as the inbox).
    pub fn switcher_results(&self) -> Vec<usize> {
        let q = self.switcher.query.text().trim().to_lowercase();
        if q.is_empty() {
            let mut idx: Vec<usize> = (0..self.conversations.len()).collect();
            idx.sort_by_key(|&i| std::cmp::Reverse(self.conversations[i].active_at_ms));
            idx
        } else {
            let mut scored: Vec<(usize, u32)> = self
                .conversations_lowered
                .iter()
                .enumerate()
                .map(|(i, l)| (i, fuzzy_score_lowered(l, &q)))
                .filter(|(_, s)| *s > 0)
                .collect();
            scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
            scored.into_iter().map(|(i, _)| i).collect()
        }
    }

    /// Rows for the quick switcher. With a query it's a flat fuzzy list;
    /// empty, it's Discord-style sections: Drafts, Unread, then Recent
    /// (each by recency, no conversation repeated across sections).
    pub fn switcher_rows(&self) -> Vec<SwitcherRow> {
        if !self.switcher.query.text().trim().is_empty() {
            return self
                .switcher_results()
                .into_iter()
                .map(SwitcherRow::Conv)
                .collect();
        }
        let by_recency = |idxs: &mut Vec<usize>| {
            idxs.sort_by_key(|&i| std::cmp::Reverse(self.conversations[i].active_at_ms));
        };
        let mut drafts: Vec<usize> = (0..self.conversations.len())
            .filter(|&i| {
                self.drafts
                    .get(&self.conversations[i].id)
                    .is_some_and(|d| !d.trim().is_empty())
            })
            .collect();
        by_recency(&mut drafts);
        let mut unread: Vec<usize> = (0..self.conversations.len())
            .filter(|&i| self.conv_is_unread(&self.conversations[i]) && !drafts.contains(&i))
            .collect();
        // Same priority the hotlist / Ctrl+N use: unseen mentions first,
        // then DMs, then team channels — recency within each tier.
        unread.sort_by_key(|&i| {
            let c = &self.conversations[i];
            let tier: u8 = if self.mentioned.contains(&c.id) {
                0
            } else if c.channel.members_type.is_team() {
                2
            } else {
                1
            };
            (tier, std::cmp::Reverse(c.active_at_ms))
        });
        let shown: HashSet<usize> = drafts.iter().chain(unread.iter()).copied().collect();
        let mut recent: Vec<usize> = (0..self.conversations.len())
            .filter(|i| !shown.contains(i))
            .collect();
        by_recency(&mut recent);

        let mut rows = Vec::new();
        for (label, group) in [("Drafts", drafts), ("Unread", unread), ("Recent", recent)] {
            if !group.is_empty() {
                rows.push(SwitcherRow::Header(label));
                rows.extend(group.into_iter().map(SwitcherRow::Conv));
            }
        }
        rows
    }

    /// The selectable conversation indices of [`Self::switcher_rows`], in
    /// display order — what `switcher_selected` indexes.
    pub fn switcher_selectable(&self) -> Vec<usize> {
        self.switcher_rows()
            .into_iter()
            .filter_map(|r| match r {
                SwitcherRow::Conv(i) => Some(i),
                SwitcherRow::Header(_) => None,
            })
            .collect()
    }

    /// Opens the Settings overlay over the current screen, focus on the
    /// section sidebar. Every change applies (and persists) immediately, so
    /// there is nothing to stash for a cancel.
    pub fn open_settings(&mut self) {
        self.settings_ui.from = self.screen;
        self.settings_ui.focus = SettingsFocus::Sidebar;
        self.settings_ui.section = 0;
        self.settings_ui.item = 0;
        self.screen = Screen::Settings;
    }

    /// Closes the Settings overlay, returning to where it was opened.
    pub fn close_settings(&mut self) {
        self.screen = self.settings_ui.from;
    }

    /// The section currently highlighted in the sidebar.
    pub fn settings_section_obj(&self) -> SettingsSection {
        SettingsSection::ALL[self.settings_ui.section.min(SettingsSection::ALL.len() - 1)]
    }

    /// Applies (and persists) the theme preset at `idx`, live. The Theme
    /// picker calls this as its cursor moves — apply-immediately, no
    /// separate confirm step.
    pub fn apply_theme_idx(&mut self, idx: usize) {
        let Some(&p) = theme::Preset::ALL.get(idx) else {
            return;
        };
        self.settings_ui.theme_idx = idx;
        self.theme = theme::adapt(
            Theme::from_palette(&p.palette()),
            theme::ColorCaps::detect(),
        );
        if !self.settings.write_theme_name(p.name()) {
            self.set_action(ActionState::Error(
                "theme applied but not saved (config not writable)".into(),
            ));
            self.push_cmd("settings write", false, "theme");
        }
        // Message blocks bake theme colours into their spans.
        self.invalidate_msg_render_cache();
    }

    /// Bumps [`Self::msg_cache_epoch`] so the render thread drops its cached
    /// per-message blocks on the next frame.
    pub fn invalidate_msg_render_cache(&mut self) {
        self.msg_cache_epoch = self.msg_cache_epoch.wrapping_add(1);
    }

    /// The display value of a Settings row.
    pub fn setting_value(&self, id: SettingId) -> String {
        let s = &self.settings_cache;
        let secs_off = |n: u64| {
            if n == 0 {
                "off".to_string()
            } else {
                format!("{n}s")
            }
        };
        match id {
            SettingId::Username => or_dash(&self.identity.username),
            SettingId::Device => or_dash(&self.identity.device_name),
            SettingId::DeviceType => or_dash(&self.identity.device_type),
            SettingId::AutoMarkRead => if s.auto_mark_read { "on" } else { "off" }.to_string(),
            SettingId::WebPreviews => if s.web_previews { "on" } else { "off" }.to_string(),
            SettingId::SmartJoins => if s.smart_joins { "on" } else { "off" }.to_string(),
            // Never render the key itself — presence only.
            SettingId::GiphyApiKey => {
                if s.giphy_api_key.is_empty() {
                    "not set".to_string()
                } else {
                    "set ••••".to_string()
                }
            }
            SettingId::InboxRefresh => secs_off(s.inbox_refresh_secs),
            SettingId::CmdlogRows => {
                if s.cmdlog_rows == 0 {
                    "hidden".to_string()
                } else {
                    s.cmdlog_rows.to_string()
                }
            }
            SettingId::ClipboardClear => secs_off(s.clipboard_clear_secs),
            SettingId::ListTimeout => format!("{}s", s.list_inbox_timeout_secs),
            SettingId::DownloadTimeout => format!("{}s", s.download_timeout_secs),
            SettingId::ImageProtocol => s.image_protocol.clone(),
            SettingId::ImageSymbols => s.image_symbols.clone(),
            SettingId::EmojiStyle => s.emoji_style.clone(),
            SettingId::IconStyle => s.icon_style.clone(),
        }
    }

    /// Adjusts a Settings row by `delta` (−1 / +1 from `←`/`→`), applying it
    /// live to `settings_cache` and persisting to `config.toml`. Read-only
    /// identity rows are no-ops.
    pub fn settings_adjust(&mut self, id: SettingId, delta: isize) {
        let (key, value): (&str, String) = match id {
            SettingId::Username | SettingId::Device | SettingId::DeviceType => return,
            SettingId::AutoMarkRead => {
                let v = !self.settings_cache.auto_mark_read;
                self.settings_cache.auto_mark_read = v;
                (
                    "auto_mark_read",
                    if v { "true" } else { "false" }.to_string(),
                )
            }
            SettingId::WebPreviews => {
                let v = !self.settings_cache.web_previews;
                self.settings_cache.web_previews = v;
                ("web_previews", if v { "true" } else { "false" }.to_string())
            }
            SettingId::SmartJoins => {
                let v = !self.settings_cache.smart_joins;
                self.settings_cache.smart_joins = v;
                ("smart_joins", if v { "true" } else { "false" }.to_string())
            }
            // Secrets aren't stepped — adjust opens the input editor.
            SettingId::GiphyApiKey => {
                self.settings_ui.editing = Some(id);
                self.settings_ui.input =
                    crate::domain::LineEditor::from_text(self.settings_cache.giphy_api_key.clone());
                return;
            }
            SettingId::InboxRefresh => {
                let n = step_clamp(self.settings_cache.inbox_refresh_secs, delta, 30, 0, 3600);
                self.settings_cache.inbox_refresh_secs = n;
                ("inbox_refresh_secs", n.to_string())
            }
            SettingId::CmdlogRows => {
                let n = step_clamp(self.settings_cache.cmdlog_rows, delta, 1, 0, 6);
                self.settings_cache.cmdlog_rows = n;
                ("cmdlog_rows", n.to_string())
            }
            SettingId::ClipboardClear => {
                let n = step_clamp(self.settings_cache.clipboard_clear_secs, delta, 5, 0, 600);
                self.settings_cache.clipboard_clear_secs = n;
                ("clipboard_clear_secs", n.to_string())
            }
            SettingId::ListTimeout => {
                let n = step_clamp(
                    self.settings_cache.list_inbox_timeout_secs,
                    delta,
                    5,
                    5,
                    600,
                );
                self.settings_cache.list_inbox_timeout_secs = n;
                ("list_inbox_timeout_secs", n.to_string())
            }
            SettingId::DownloadTimeout => {
                let n = step_clamp(
                    self.settings_cache.download_timeout_secs,
                    delta,
                    30,
                    10,
                    3600,
                );
                self.settings_cache.download_timeout_secs = n;
                ("download_timeout_secs", n.to_string())
            }
            SettingId::ImageProtocol => {
                let next = cycle(&IMAGE_PROTOCOLS, &self.settings_cache.image_protocol, delta);
                self.settings_cache.image_protocol = next.clone();
                // Take effect immediately for the next render.
                self.images.set_protocol(crate::tui::image::resolve(&next));
                ("image_protocol", format!("\"{next}\""))
            }
            SettingId::ImageSymbols => {
                let next = cycle(
                    &IMAGE_SYMBOL_SETS,
                    &self.settings_cache.image_symbols,
                    delta,
                );
                self.settings_cache.image_symbols = next.clone();
                self.images.set_symbols(next.clone());
                ("image_symbols", format!("\"{next}\""))
            }
            SettingId::EmojiStyle => {
                let next = cycle(&EMOJI_STYLES, &self.settings_cache.emoji_style, delta);
                self.settings_cache.emoji_style = next.clone();
                ("emoji_style", format!("\"{next}\""))
            }
            SettingId::IconStyle => {
                let next = cycle(&ICON_STYLES, &self.settings_cache.icon_style, delta);
                self.settings_cache.icon_style = next.clone();
                ("icon_style", format!("\"{next}\""))
            }
        };
        if !self.settings.write_setting(key, &value) {
            // The change is applied live either way; the user must know it
            // won't survive a restart.
            self.set_action(ActionState::Error(
                "setting applied but not saved (config not writable)".into(),
            ));
            self.push_cmd("settings write", false, key);
        }
        // Several settings feed message blocks (emoji/icon style, image
        // protocol) — invalidating on any adjust is cheap and can't go stale.
        self.invalidate_msg_render_cache();
    }

    /// The terminal-window title reflecting the current state: the open
    /// conversation plus the attention badges — tmux/SSH users get a free
    /// status surface (tut's terminal-title integration).
    pub fn desired_term_title(&self) -> String {
        let mut t = String::from("secretbase");
        if let Some(conv) = self
            .open_conv_id
            .as_deref()
            .and_then(|id| self.conversations.iter().position(|c| c.id == id))
            .and_then(|i| self.conversations_lowered.get(i))
        {
            t.push_str(" — ");
            t.push_str(&conv.display_label);
        }
        let mentions = self.mentioned.len();
        let unread = self
            .conversations
            .iter()
            .filter(|c| c.member_status == crate::domain::MemberStatus::Active)
            .filter(|c| self.conv_is_unread(c))
            .count();
        if mentions > 0 {
            t.push_str(&format!(" @{mentions}"));
        }
        if unread > 0 {
            t.push_str(&format!(" ({unread})"));
        }
        t
    }

    /// Convenience: whether the worker is currently processing a
    /// request. Equivalent to `self.in_flight.is_some()`.
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether a conversation id is starred (our **local-only** favourite).
    pub fn is_favorite(&self, conv_id: &str) -> bool {
        self.favorites.contains(conv_id)
    }

    /// Toggles the local star on a conversation id and **persists** it to
    /// `config.toml`. Purely local — no Keybase call. Returns the new state.
    pub fn toggle_favorite(&mut self, conv_id: String) -> bool {
        let now_on = !self.favorites.contains(&conv_id);
        if now_on {
            self.favorites.insert(conv_id);
        } else {
            self.favorites.remove(&conv_id);
        }
        // Stable order keeps the config diff minimal.
        let mut ids: Vec<String> = self.favorites.iter().cloned().collect();
        ids.sort();
        self.settings_cache.favorites = ids.clone();
        if !self
            .settings
            .write_setting("favorites", &toml_quoted(&ids.join(",")))
        {
            self.push_cmd("settings write", false, "favorites not saved");
        }
        now_on
    }

    /// Whether a conversation id is locally muted.
    pub fn is_muted(&self, conv_id: &str) -> bool {
        self.muted.contains(conv_id)
    }

    /// Toggles the local mute on a conversation id and **persists** it. Purely
    /// local — no Keybase call. Returns the new state.
    pub fn toggle_muted(&mut self, conv_id: String) -> bool {
        let now_on = !self.muted.contains(&conv_id);
        if now_on {
            self.muted.insert(conv_id);
        } else {
            self.muted.remove(&conv_id);
        }
        let mut ids: Vec<String> = self.muted.iter().cloned().collect();
        ids.sort();
        self.settings_cache.muted = ids.clone();
        if !self
            .settings
            .write_setting("muted", &toml_quoted(&ids.join(",")))
        {
            self.push_cmd("settings write", false, "muted not saved");
        }
        // Mute gates `conv_is_unread`, which the tree's group unread counts
        // derive from — refresh the cached rows so the badge updates now.
        self.rebuild_tree_rows();
        now_on
    }

    /// Records (or clears, `None`) the local pin target for a conversation
    /// and **persists** the map to config — mirrors [`Self::toggle_favorite`].
    /// Purely local bookkeeping; the pin itself was already posted to Keybase.
    pub fn set_local_pin(&mut self, conv_id: String, target: Option<u64>) {
        match target {
            Some(id) => {
                self.pins.local.insert(conv_id.clone(), id);
            }
            None => {
                self.pins.local.remove(&conv_id);
            }
        }
        // The cached body (and its one-shot fetch marker) belong to the old
        // target — drop them so a re-pin resolves fresh.
        self.pins.bodies.remove(&conv_id);
        self.pins.fetch_attempted.retain(|(c, _)| c != &conv_id);
        let mut pairs: Vec<String> = self
            .pins
            .local
            .iter()
            .map(|(c, m)| format!("{c}:{m}"))
            .collect();
        pairs.sort();
        self.settings_cache.pins = pairs.clone();
        if !self
            .settings
            .write_setting("pins", &toml_quoted(&pairs.join(",")))
        {
            self.push_cmd("settings write", false, "pins not saved");
        }
    }

    /// Saves the secret-setting editor (`Enter` in its popup): writes the
    /// value to the cache + config and closes the editor. The value never
    /// reaches the command log.
    pub fn settings_secret_save(&mut self) {
        let Some(id) = self.settings_ui.editing.take() else {
            return;
        };
        let value = self.settings_ui.input.text().trim().to_string();
        #[allow(clippy::single_match_else)]
        let key = match id {
            SettingId::GiphyApiKey => {
                self.settings_cache.giphy_api_key = value.clone();
                "giphy_api_key"
            }
            _ => return,
        };
        self.settings_ui.input = crate::domain::LineEditor::default();
        if !self.settings.write_setting(key, &toml_quoted(&value)) {
            self.set_action(ActionState::Error(
                "setting applied but not saved (config not writable)".into(),
            ));
        }
    }

    /// Records a locally-dismissed pin banner (conv → the pin *envelope* id)
    /// and **persists** it — the GUI-parity ✕ (`IgnorePinnedMessage` is local
    /// there too, so there is nothing to sync). One entry per conversation:
    /// a newer dismiss overwrites, a newer pin simply no longer matches.
    pub fn set_pin_dismissed(&mut self, conv_id: String, envelope_id: u64) {
        self.pins.dismissed.insert(conv_id, envelope_id);
        let mut pairs: Vec<String> = self
            .pins
            .dismissed
            .iter()
            .map(|(c, m)| format!("{c}:{m}"))
            .collect();
        pairs.sort();
        self.settings_cache.pins_dismissed = pairs.clone();
        if !self
            .settings
            .write_setting("pins_dismissed", &toml_quoted(&pairs.join(",")))
        {
            self.push_cmd("settings write", false, "pins_dismissed not saved");
        }
    }

    /// **Effective** unread for display: the conversation is unread **and** not
    /// locally muted. Every unread surface (the `●` dot, the bold, the unread
    /// count, the Unread filter, the switcher's Unread section) uses this so a
    /// muted conversation stops demanding attention without leaving the inbox.
    /// The current [`UiMode`] for the status-strip badge, derived from the
    /// active screen + focus + sub-state.
    pub fn ui_mode(&self) -> UiMode {
        match self.screen {
            // Text-entry overlays.
            Screen::NewConversation
            | Screen::UnhideConversation
            | Screen::SearchGlobal
            | Screen::ConvSearch
            | Screen::GiphySearch
            | Screen::React
            | Screen::QuickSwitcher => UiMode::Search,
            // Modal browsers: Search while an inline text mode is open, else Normal.
            Screen::ChannelBrowser => {
                if self.channel_browser.creating || self.channel_browser.renaming.is_some() {
                    UiMode::Search
                } else {
                    UiMode::Normal
                }
            }
            Screen::Members => {
                if self.members.adding {
                    UiMode::Search
                } else {
                    UiMode::Normal
                }
            }
            Screen::Inbox => {
                if self.select.cursor.is_some() {
                    UiMode::Select
                } else {
                    match self.focus {
                        Focus::Search => UiMode::Search,
                        Focus::Chat if self.open_conv_id.is_some() => UiMode::Compose,
                        _ => UiMode::Normal,
                    }
                }
            }
            _ => UiMode::Normal,
        }
    }

    pub fn conv_is_unread(&self, conv: &Conversation) -> bool {
        conv.unread && !self.muted.contains(&conv.id)
    }

    /// Starts a worker request: stamps `in_flight` with `slot` and
    /// returns `true`. Refuses (returns `false`, leaving the current
    /// request untouched) when one is already in flight — the
    /// single-in-flight invariant the dispatcher relies on.
    ///
    /// Input is already gated while busy (`input::common::busy_blocks`),
    /// but `begin` is the belt-and-suspenders guard against a
    /// *programmatic* double-send (e.g. an auto-refresh racing an open)
    /// silently overwriting `in_flight` and desynchronising the
    /// `in_flight` ↔ response ordering — which surfaced as a "dispatch
    /// mismatch". Every `request_*` flow uses this instead of assigning
    /// `in_flight` directly.
    pub fn begin(&mut self, slot: InFlight) -> bool {
        if self.worker_dead {
            // No worker is alive to serve it — refuse instantly rather than
            // claiming a slot whose response can never arrive.
            self.set_action(ActionState::Error(
                "worker thread died — restart secretbase".into(),
            ));
            return false;
        }
        if self.in_flight.is_some() {
            self.push_cmd("worker request", false, "busy — request ignored");
            return false;
        }
        self.in_flight = Some(slot);
        self.request_started = Some(Instant::now());
        true
    }

    /// Unwedges the UI after the worker response channel closed — every
    /// worker thread is gone, so no response will ever arrive. Releases the
    /// in-flight slot (otherwise `busy_blocks` swallows keys forever) and
    /// surfaces a persistent error, once.
    pub fn on_worker_dead(&mut self) {
        if self.worker_dead {
            return;
        }
        self.worker_dead = true;
        self.in_flight = None;
        self.bg_inflight = false;
        self.select.batch = None;
        self.set_action(ActionState::Error(
            "worker thread died — keybase calls disabled; restart secretbase".into(),
        ));
        self.push_cmd("worker", false, "response channel closed — worker died");
    }

    /// Watchdog for a lost in-flight ticket: every `keybase` call has a
    /// per-op timeout, so a claimed slot must resolve within the largest
    /// configured budget. If it doesn't (worker died mid-call, response
    /// dropped), release the slot so the UI doesn't stay busy forever.
    /// Called once per run-loop tick.
    pub fn watchdog_release_stuck_request(&mut self) {
        let Some(started) = self.request_started else {
            return;
        };
        if self.in_flight.is_none() {
            return;
        }
        let budget = self
            .settings_cache
            .download_timeout_secs
            .max(self.settings_cache.list_inbox_timeout_secs)
            .saturating_add(30);
        if started.elapsed() > std::time::Duration::from_secs(budget) {
            self.in_flight = None;
            self.select.batch = None;
            self.set_action(ActionState::Error(
                "request got no response in time — released".into(),
            ));
            self.push_cmd("worker watchdog", false, "abandoned in-flight request");
        }
    }

    /// Starts a worker request **end-to-end**: claims the in-flight slot
    /// ([`Self::begin`]), shows the `Running` toast, and sends on the user
    /// lane. A failed send (worker gone) releases the slot and routes
    /// through [`Self::on_worker_dead`] instead of leaving the UI busy
    /// forever. Returns whether the request was dispatched — the shared
    /// body of every `request_*` flow.
    pub fn submit(&mut self, slot: InFlight, label: &str, req: WorkerRequest) -> bool {
        if !self.begin(slot) {
            return false;
        }
        self.set_action(ActionState::Running(label.to_string()));
        if self.worker_tx.send(req).is_err() {
            self.in_flight = None;
            self.on_worker_dead();
            return false;
        }
        true
    }

    /// Replaces the action state and resets the spinner tick counter so
    /// the next animation frame starts from zero.
    pub fn set_action(&mut self, state: ActionState) {
        self.action_state = state;
        self.action_tick = 0;
    }

    /// Increments the spinner animation tick. Wraps modulo 4 so the
    /// renderer can index a 4-frame braille spinner without doing the
    /// modulo itself.
    pub fn tick_action(&mut self) {
        self.action_tick = (self.action_tick + 1) % 4;
    }

    /// Pushes a new entry to the command log. Bridges to the timing state
    /// this method owns (the elapsed duration from [`Self::last_op_elapsed`])
    /// and mirrors the line to the debug log, then hands the built entry to
    /// [`crate::tui::cmdlog_state::CmdLogState::push`], which owns the
    /// ring-buffer trim + cursor/marks alignment.
    ///
    /// Each entry is also mirrored to `~/.secretbase.log` when the
    /// `SECRETBASE_DEBUG=1` env var is set, giving the user a
    /// post-crash audit trail without changing the in-memory ring
    /// buffer semantics. The file write is a no-op when the env var
    /// is unset, so production runs pay nothing.
    pub fn push_cmd(&mut self, cmd: impl Into<String>, ok: bool, detail: impl Into<String>) {
        let cmd = cmd.into();
        let detail = detail.into();
        crate::tui::debug_log::log(&format!(
            "cmd {} {cmd} | {detail}",
            if ok { "ok" } else { "ERR" }
        ));
        let duration = self.last_op_elapsed.take();
        self.cmdlog.push(CmdEntry {
            cmd,
            ok,
            detail,
            duration,
        });
    }

    /// Whether a modal overlay (a popup screen or the file picker) is on top.
    /// Used by the run loop to force a full repaint when one closes — wide
    /// glyphs (emoji in the reaction picker) otherwise leave residual cells.
    pub fn has_overlay(&self) -> bool {
        self.file_picker.is_some()
            || matches!(
                self.screen,
                Screen::Help
                    | Screen::Settings
                    | Screen::ConfirmLogout
                    | Screen::ConfirmConvAction
                    | Screen::NewConversation
                    | Screen::UnhideConversation
                    | Screen::ChannelBrowser
                    | Screen::Members
                    | Screen::SearchGlobal
                    | Screen::ConfirmDeleteMessage
                    | Screen::React
                    | Screen::QuickSwitcher
                    | Screen::CommandPalette
                    | Screen::ConvSearch
                    | Screen::GiphySearch
            )
    }

    /// Rebuilds the `@`-mention candidate list for the open conversation:
    /// its participants (DM channel name) plus everyone who's spoken in the
    /// loaded history. Sorted + de-duplicated.
    pub fn rebuild_conv_members(&mut self) {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if let Some(id) = self.open_conv_id.clone()
            && let Some(c) = self.conversations.iter().find(|c| c.id == id)
            && !c.channel.members_type.is_team()
        {
            for u in c.channel.name.split(',') {
                let u = u.trim();
                if !u.is_empty() {
                    set.insert(u.to_string());
                }
            }
        }
        for m in &self.thread.messages {
            if !m.sender.is_empty() {
                set.insert(m.sender.clone());
            }
        }
        self.conv_members = set.into_iter().collect();
    }

    /// The autocomplete matches for the `@`-mention currently being typed in
    /// the compose box (empty when not in a mention). Prefix-matched,
    /// case-insensitive, capped.
    pub fn mention_matches(&self) -> Vec<String> {
        let Some((_, prefix)) =
            crate::domain::active_mention(self.compose.text(), self.compose.cursor())
        else {
            return Vec::new();
        };
        let p = prefix.to_ascii_lowercase();
        self.conv_members
            .iter()
            .filter(|m| m.to_ascii_lowercase().starts_with(&p))
            .take(8)
            .cloned()
            .collect()
    }

    /// The cheap state gate for the `@`-mention popup — everything except the
    /// (allocating) match computation, so a caller that already has the
    /// matches in hand doesn't recompute them just to test the gate.
    pub fn mention_popup_gate(&self) -> bool {
        self.focus == Focus::Chat
            && self.screen == Screen::Inbox
            && self.open_conv_id.is_some()
            && self.select.cursor.is_none()
            && self.edit_target_id.is_none()
            && !self.mention_popup_dismissed()
    }

    /// Whether the popup was Esc-dismissed **for the token currently under
    /// the cursor** — editing to a different token re-arms it.
    fn mention_popup_dismissed(&self) -> bool {
        match (
            &self.mention_dismissed_token,
            crate::domain::active_mention(self.compose.text(), self.compose.cursor()),
        ) {
            (Some(dismissed), Some((_, prefix))) => *dismissed == prefix,
            _ => false,
        }
    }

    /// Esc on the open popup: remember the token so the popup stays closed
    /// while it's unchanged (typing on re-arms naturally, since the token
    /// text differs).
    pub fn dismiss_mention_popup(&mut self) {
        self.mention_dismissed_token =
            crate::domain::active_mention(self.compose.text(), self.compose.cursor())
                .map(|(_, prefix)| prefix.to_string());
    }

    /// Catalogue indices matching the `:token` under the compose cursor —
    /// the `:emoji:` autocomplete's rows (alias prefix match, capped).
    pub fn emoji_ac_matches(&self) -> Vec<usize> {
        let Some((_, prefix)) =
            crate::domain::active_emoji_token(self.compose.text(), self.compose.cursor())
        else {
            return Vec::new();
        };
        self.emoji
            .all
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alias.to_ascii_lowercase().starts_with(&prefix))
            .map(|(i, _)| i)
            .take(8)
            .collect()
    }

    /// Whether the `:emoji:` popup should show / capture keys. The mention
    /// popup wins when both could apply (their sigils make that rare).
    pub fn emoji_ac_active(&self) -> bool {
        self.mention_popup_gate()
            && !self.mention_popup_active()
            && !self.emoji_ac_dismissed()
            && !self.emoji_ac_matches().is_empty()
    }

    fn emoji_ac_dismissed(&self) -> bool {
        match (
            &self.emoji_ac_dismissed_token,
            crate::domain::active_emoji_token(self.compose.text(), self.compose.cursor()),
        ) {
            (Some(dismissed), Some((_, prefix))) => *dismissed == prefix,
            _ => false,
        }
    }

    /// Esc on the open `:emoji:` popup — same per-token dismiss contract as
    /// the mention popup.
    pub fn dismiss_emoji_ac(&mut self) {
        self.emoji_ac_dismissed_token =
            crate::domain::active_emoji_token(self.compose.text(), self.compose.cursor())
                .map(|(_, prefix)| prefix);
    }

    /// Whether the `@`-mention popup should be shown / capture keys — only in
    /// the chat's compose mode with at least one match.
    pub fn mention_popup_active(&self) -> bool {
        self.mention_popup_gate() && !self.mention_matches().is_empty()
    }

    /// Updates `last_activity` to "now" — called on every keypress and
    /// mouse event so any future auto-lock / inactivity timeout has a
    /// fresh baseline.
    pub fn reset_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Rebuilds [`Self::conversations_lowered`] from
    /// [`Self::conversations`]. Called once after every load — the cache
    /// feeds the hot rendering/search paths and stays valid as long as
    /// `conversations` isn't mutated outside this function.
    pub fn rebuild_lowered(&mut self) {
        let me_owned = self.identity.username.clone();
        let me = if me_owned.is_empty() {
            None
        } else {
            Some(me_owned.as_str())
        };
        self.conversations_lowered = self
            .conversations
            .iter()
            .map(|c| LoweredConversation::from(c, me))
            .collect();
    }

    /// Recomputes the per-history projections — [`Self::pinned_msg_id`],
    /// [`Self::conv_headline`] and [`Self::msg_index`] — by scanning the
    /// loaded history once. Called after every mutation of
    /// [`Self::messages`] (load, live append, clear), so the render path
    /// never rescans the whole history per frame for them.
    pub fn rebuild_msg_meta(&mut self) {
        // Any history *replacement* invalidates the per-message render cache
        // (an edit re-read keeps ids but changes their bodies).
        self.invalidate_msg_render_cache();
        self.rebuild_msg_meta_impl();
    }

    /// [`Self::rebuild_msg_meta`] for **prepend-only** mutations (older-page
    /// loads): the retained ids' content is unchanged and the per-entry
    /// grouped/pinned fingerprints cover the page-boundary message, so the
    /// render cache stays valid. Invalidating here made every wheel-driven
    /// page rebuild the *whole* loaded history — quadratic over a scrollback
    /// session, felt as a scroll freeze.
    pub fn rebuild_msg_meta_after_prepend(&mut self) {
        self.rebuild_msg_meta_impl();
    }

    fn rebuild_msg_meta_impl(&mut self) {
        use crate::domain::MessageContent;
        // O(1) id → index lookups (reply quotes, the pin header). Projected
        // histories have unique ids; a duplicate would keep the later row,
        // matching what the reader sees.
        self.thread.msg_index = self
            .thread
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        // Latest channel topic/headline — the chat's adaptive header.
        self.thread.conv_headline =
            self.thread
                .messages
                .iter()
                .rev()
                .find_map(|m| match &m.content {
                    MessageContent::Headline { headline } if !headline.trim().is_empty() => {
                        Some(zeroize::Zeroizing::new(headline.clone()))
                    }
                    _ => None,
                });
        // The newest (undeleted) `Pin` message is authoritative — an unpin
        // is a DELETE superseding it, so `fold_deletes` already removed
        // cleared pins from the history. The JSON API does **not** carry
        // the pin payload (`convertMsgBody` omits `Pin__`), so `target_id`
        // is 0 in practice: presence + sender are what the read gives us,
        // and the target falls back to what *we* pinned this session.
        self.pins.msg_id = None;
        self.pins.present = false;
        self.pins.sender = None;
        self.pins.envelope_id = None;
        for m in self.thread.messages.iter().rev() {
            if let MessageContent::Pin { target_id } = &m.content {
                // Locally dismissed (the GUI-parity ✕)? The pin stays active
                // server-side, but this client hides the banner — until a
                // *newer* pin envelope (a different id) appears.
                if self
                    .open_conv_id
                    .as_ref()
                    .and_then(|c| self.pins.dismissed.get(c))
                    == Some(&m.id)
                {
                    return;
                }
                self.pins.envelope_id = Some(m.id);
                self.pins.present = true;
                self.pins.sender = Some(m.sender.clone());
                self.pins.msg_id = if *target_id != 0 {
                    Some(*target_id) // future-proof: use it if the API ever carries it
                } else {
                    self.open_conv_id
                        .as_ref()
                        .and_then(|id| self.pins.local.get(id))
                        .copied()
                };
                return;
            }
        }
    }

    /// Rebuilds [`Self::filtered_cache`] from the current filter +
    /// search query.
    pub fn rebuild_filter(&mut self) {
        let query_lc = self.search.text().to_lowercase();
        let mut indices: Vec<usize> = Vec::new();
        for (idx, conv) in self.conversations.iter().enumerate() {
            if conv.member_status != MemberStatus::Active {
                continue;
            }
            if !query_lc.is_empty() {
                let l = &self.conversations_lowered[idx];
                if fuzzy_score_lowered(l, &query_lc) == 0 {
                    continue;
                }
            }
            indices.push(idx);
        }
        // Most-recent first — `active_at_ms` is monotonically growing.
        indices.sort_by_key(|&i| std::cmp::Reverse(self.conversations[i].active_at_ms));
        self.filtered_cache = indices;
        self.rebuild_tree_rows();
        // Land the tree cursor on the first conversation (skipping the leading
        // group header) so actions that need a selected conversation work
        // right after a load / filter / search change.
        self.tree_selected = self
            .tree_rows_cache
            .iter()
            .position(|r| matches!(r, TreeRow::Conv { .. }))
            .unwrap_or(0);
    }

    /// [`Self::rebuild_filter`] for **resyncs and bumps**: keeps the tree
    /// cursor on the same conversation (found again by id) instead of
    /// re-seating it on the first row. A recency re-sort from a push message,
    /// a mark-read, or a background refresh must not yank the cursor while
    /// the user is navigating — an id survives the re-sort, a position
    /// doesn't. Falls back to the first row when the conversation left the
    /// tree (filtered out / removed). A genuinely **new** search/filter
    /// should use `rebuild_filter`, where snapping to the best match is the
    /// right behaviour.
    pub fn rebuild_filter_preserving_cursor(&mut self) {
        let prev_id = self.selected_conversation().map(|c| c.id.clone());
        self.rebuild_filter();
        if let Some(id) = prev_id
            && let Some(pos) = self.tree_rows_cache.iter().position(
                |r| matches!(r, TreeRow::Conv { idx } if self.conversations[*idx].id == id),
            )
        {
            self.tree_selected = pos;
        }
    }

    /// The conversation under the tree cursor, if it's on a conversation row
    /// (not a group header).
    pub fn selected_conversation(&self) -> Option<&Conversation> {
        match self.tree_rows().get(self.tree_selected) {
            Some(TreeRow::Conv { idx }) => self.conversations.get(*idx),
            _ => None,
        }
    }

    /// Sentinel collapse-key for the Direct-messages group (a NUL byte can't
    /// occur in a team name, so it never collides).
    pub const DMS_KEY: &'static str = "\u{0}dms";

    /// Flattened rows of the conversation **tree** — the cached result of
    /// [`Self::rebuild_tree_rows`]. Rendering and cursor math read this per
    /// frame; the O(teams × conversations) build runs only when an input
    /// changes (filter/search rebuild, fold/unfold, mute, reveal).
    pub fn tree_rows(&self) -> &[TreeRow] {
        &self.tree_rows_cache
    }

    /// Rebuilds [`Self::tree_rows_cache`]: a "Direct messages" group then one
    /// group per team, each (unless collapsed) followed by its conversations.
    /// Built from `filtered_cache` (already status/search filtered,
    /// recency-sorted). A non-empty search force-expands all groups. Call
    /// after mutating anything the rows derive from: `filtered_cache` /
    /// `conversations` (via [`Self::rebuild_filter`], which calls this),
    /// `expanded`, `muted`, or an `unread` flag.
    pub fn rebuild_tree_rows(&mut self) {
        let leaves = &self.filtered_cache;
        let is_team = |i: usize| self.conversations[i].channel.members_type.is_team();
        let force_expand = !self.search.text().trim().is_empty();

        // Teams present among the leaves, alphabetical.
        let mut teams: Vec<String> = leaves
            .iter()
            .filter(|&&i| is_team(i))
            .map(|&i| self.conversations[i].channel.name.clone())
            .collect();
        teams.sort();
        teams.dedup();

        let mut rows: Vec<TreeRow> = Vec::new();
        let push_group = |rows: &mut Vec<TreeRow>, key: String, label: String, team: bool| {
            let members: Vec<usize> = leaves
                .iter()
                .copied()
                .filter(|&i| {
                    is_team(i) == team && (!team || self.conversations[i].channel.name == key)
                })
                .collect();
            if members.is_empty() {
                return;
            }
            let unread = members
                .iter()
                .filter(|&&i| self.conv_is_unread(&self.conversations[i]))
                .count();
            let mentioned = members
                .iter()
                .any(|&i| self.mentioned.contains(&self.conversations[i].id));
            let collapsed = !force_expand && !self.expanded.contains(&key);
            rows.push(TreeRow::Group {
                key,
                label,
                is_team: team,
                collapsed,
                unread,
                mentioned,
            });
            if !collapsed {
                rows.extend(members.into_iter().map(|idx| TreeRow::Conv { idx }));
            }
        };

        push_group(
            &mut rows,
            Self::DMS_KEY.to_string(),
            "Direct messages".to_string(),
            false,
        );
        for team in teams {
            push_group(&mut rows, team.clone(), team, true);
        }
        self.tree_rows_cache = rows;
    }

    /// Toggles a tree group's collapsed state (groups start collapsed).
    pub fn toggle_collapsed(&mut self, key: &str) {
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_string());
        }
        self.rebuild_tree_rows();
    }

    /// Total number of conversations flagged as unread. Surfaced in
    /// the status panel of every screen so the user always knows if
    /// there is something to attend to.
    pub fn unread_total(&self) -> usize {
        self.conversations
            .iter()
            .filter(|c| self.conv_is_unread(c))
            .count()
    }

    /// Clears the compose buffer and resets the cursor. Called after
    /// a successful send and whenever the compose pane is dismissed.
    /// Also drops the in-flight `reply_to_id` (the reply context is
    /// tied to the draft).
    pub fn compose_clear(&mut self) {
        self.compose.clear();
        self.reply_to_id = None;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::channel;

    use super::*;
    use crate::ports::{ClipboardPort, OpenerPort, SettingsPort, UserSettings};

    struct FakeClipboard;
    impl ClipboardPort for FakeClipboard {
        fn write(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeOpener;
    impl OpenerPort for FakeOpener {
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

    /// Builds an [`App`] with dangling worker channels — the tests in
    /// this module only exercise pure state helpers (`compose_*`,
    /// `rebuild_*`, …) and never send a [`WorkerRequest`]. The flow
    /// tests that drive the worker live in `flows/chat/tests.rs`.
    fn fresh_app() -> App {
        let (req_tx, _req_rx) = channel::<WorkerRequest>();
        let (bg_tx, _bg_rx) = channel::<WorkerRequest>();
        let (_resp_tx, resp_rx) = channel::<WorkerResponse>();
        App::new(
            req_tx,
            bg_tx,
            resp_rx,
            None,
            Box::new(FakeClipboard),
            Box::new(FakeOpener),
            Box::new(FakeSettings),
        )
    }

    #[test]
    fn settings_open_and_close_round_trip() {
        let mut app = fresh_app();
        app.screen = Screen::Inbox;
        app.open_settings();
        assert_eq!(app.screen, Screen::Settings);
        assert_eq!(app.settings_ui.from, Screen::Inbox);
        app.close_settings();
        assert_eq!(app.screen, Screen::Inbox);
    }

    #[test]
    fn apply_theme_idx_applies_and_persists_live() {
        let mut app = fresh_app();
        app.screen = Screen::Teams;
        app.open_settings();
        let dracula = theme::Preset::ALL
            .iter()
            .position(|p| *p == theme::Preset::Dracula)
            .unwrap();
        app.apply_theme_idx(dracula);
        assert_eq!(app.settings_ui.theme_idx, dracula);
        assert_eq!(
            app.theme.accent,
            Theme::from_palette(&theme::Preset::Dracula.palette()).accent
        );
        // Apply-immediately: closing keeps the change (no cancel/restore).
        app.close_settings();
        assert_eq!(app.screen, Screen::Teams);
        assert_eq!(
            app.theme.accent,
            Theme::from_palette(&theme::Preset::Dracula.palette()).accent
        );
    }

    #[test]
    fn settings_adjust_toggle_number_choice() {
        use crate::tui::app::SettingId;
        let mut app = fresh_app();
        // Toggle flips.
        let before = app.settings_cache.auto_mark_read;
        app.settings_adjust(SettingId::AutoMarkRead, 1);
        assert_eq!(app.settings_cache.auto_mark_read, !before);
        // Number steps and clamps at the floor.
        app.settings_cache.clipboard_clear_secs = 5;
        app.settings_adjust(SettingId::ClipboardClear, -1); // step 5 → 0
        assert_eq!(app.settings_cache.clipboard_clear_secs, 0);
        app.settings_adjust(SettingId::ClipboardClear, -1); // clamped at min 0
        assert_eq!(app.settings_cache.clipboard_clear_secs, 0);
        // Choice cycles.
        app.settings_cache.image_protocol = "auto".into();
        app.settings_adjust(SettingId::ImageProtocol, 1);
        assert_eq!(app.settings_cache.image_protocol, "kitty");
        // Emoji style choice cycles glyph ↔ shortcode.
        app.settings_cache.emoji_style = "glyph".into();
        app.settings_adjust(SettingId::EmojiStyle, 1);
        assert_eq!(app.settings_cache.emoji_style, "shortcode");
        // Read-only identity row is a no-op.
        app.settings_adjust(SettingId::Username, 1);
    }

    #[test]
    fn compose_insert_advances_cursor() {
        let mut app = fresh_app();
        app.compose.insert('h');
        app.compose.insert('i');
        assert_eq!(app.compose.text(), "hi");
        assert_eq!(app.compose.cursor(), 2);
    }

    #[test]
    fn compose_insert_in_middle() {
        let mut app = fresh_app();
        app.compose.insert('a');
        app.compose.insert('c');
        app.compose.left();
        app.compose.insert('b');
        assert_eq!(app.compose.text(), "abc");
        assert_eq!(app.compose.cursor(), 2);
    }

    #[test]
    fn compose_backspace_removes_left_of_cursor() {
        let mut app = fresh_app();
        app.compose.insert('h');
        app.compose.insert('i');
        app.compose.backspace();
        assert_eq!(app.compose.text(), "h");
        assert_eq!(app.compose.cursor(), 1);
    }

    #[test]
    fn compose_backspace_at_start_is_noop() {
        let mut app = fresh_app();
        app.compose.backspace();
        assert_eq!(app.compose.text(), "");
        assert_eq!(app.compose.cursor(), 0);
    }

    #[test]
    fn compose_home_end_jumps() {
        let mut app = fresh_app();
        for c in "hello".chars() {
            app.compose.insert(c);
        }
        app.compose.home();
        assert_eq!(app.compose.cursor(), 0);
        app.compose.end();
        assert_eq!(app.compose.cursor(), 5);
    }

    #[test]
    fn compose_handles_multibyte_characters() {
        let mut app = fresh_app();
        // 'á' is 2 bytes; the editor keeps the cursor on a char boundary.
        app.compose.insert('á');
        app.compose.insert('!');
        assert_eq!(app.compose.text(), "á!");
        app.compose.left();
        app.compose.backspace();
        assert_eq!(app.compose.text(), "!");
    }

    #[test]
    fn compose_clear_resets_state() {
        let mut app = fresh_app();
        app.compose.set("draft");
        app.reply_to_id = Some(7);
        app.compose_clear();
        assert_eq!(app.compose.text(), "");
        assert_eq!(app.compose.cursor(), 0);
        assert_eq!(app.reply_to_id, None);
    }

    #[test]
    fn toggle_favorite_is_local_and_persists() {
        let mut app = fresh_app();
        assert!(!app.is_favorite("abc"));
        // Toggle on → starred + mirrored into the persisted cache.
        assert!(app.toggle_favorite("abc".into()));
        assert!(app.is_favorite("abc"));
        assert_eq!(app.settings_cache.favorites, vec!["abc".to_string()]);
        // Toggle off → cleared.
        assert!(!app.toggle_favorite("abc".into()));
        assert!(!app.is_favorite("abc"));
        assert!(app.settings_cache.favorites.is_empty());
    }

    #[test]
    fn rebuild_conv_members_unions_dm_participants_and_senders() {
        use crate::domain::{Channel, Conversation, MemberStatus, MembersType, Message};
        let mut app = fresh_app();
        app.conversations = vec![Conversation {
            id: "dm1".into(),
            channel: Channel {
                name: "alice, bob ,me".into(), // messy spacing must be trimmed
                members_type: MembersType::ImpTeamNative,
                topic_name: None,
            },
            unread: false,
            active_at: 0,
            active_at_ms: 0,
            member_status: MemberStatus::Active,
            creator_info: None,
        }];
        app.open_conv_id = Some("dm1".into());
        let mut m = Message::default();
        m.sender = "carol".into(); // spoke in the thread but isn't in the name
        app.thread.messages = vec![m, Message::default()]; // empty sender is dropped
        app.rebuild_conv_members();
        // Participants from the DM name + people who've spoken, sorted, no
        // blanks — the @-mention candidate pool.
        assert_eq!(app.conv_members, vec!["alice", "bob", "carol", "me"]);
    }

    #[test]
    fn rebuild_conv_members_team_channel_uses_only_senders() {
        use crate::domain::{Channel, Conversation, MemberStatus, MembersType, Message};
        let mut app = fresh_app();
        app.conversations = vec![Conversation {
            id: "t1".into(),
            channel: Channel {
                // A team's `name` is the team, not a participant list — it
                // must NOT leak into the mention candidates.
                name: "phoenix".into(),
                members_type: MembersType::Team,
                topic_name: Some("general".into()),
            },
            unread: false,
            active_at: 0,
            active_at_ms: 0,
            member_status: MemberStatus::Active,
            creator_info: None,
        }];
        app.open_conv_id = Some("t1".into());
        let mut m = Message::default();
        m.sender = "dave".into();
        app.thread.messages = vec![m];
        app.rebuild_conv_members();
        assert_eq!(app.conv_members, vec!["dave"]);
    }

    #[test]
    fn emoji_filter_matches_keywords_and_floats_most_used() {
        let mut app = fresh_app();
        // Substring over the keywords: "thumb" finds 👍 even though its
        // shortcode is `+1` (the whole point of the keywords field).
        app.react.set("thumb");
        app.rebuild_emoji_filter();
        let hits = app.emoji.filtered();
        assert!(!hits.is_empty(), "keyword search must match");
        assert!(
            hits.iter()
                .any(|&i| app.emoji.all[i].alias == "+1"
                    || app.emoji.all[i].keywords.contains("thumb")),
            "thumb should surface the thumbs-up family"
        );
        // Frecency: bump an arbitrary matching alias — it must float first.
        let last = *hits.last().expect("non-empty");
        let bumped = app.emoji.all[last].alias.clone();
        *app.emoji.uses.entry(bumped.clone()).or_insert(0) += 3;
        app.rebuild_emoji_filter();
        let first = app.emoji.filtered()[0];
        assert_eq!(
            app.emoji.all[first].alias, bumped,
            "most-used floats to the top"
        );
        // Empty query → the whole catalogue.
        app.react.clear();
        app.rebuild_emoji_filter();
        assert_eq!(app.emoji.filtered().len(), app.emoji.all.len());
    }

    #[test]
    fn toggle_muted_is_local_and_affects_effective_unread() {
        use crate::domain::{Channel, Conversation, MemberStatus, MembersType};
        let mut app = fresh_app();
        let c = Conversation {
            id: "abc".into(),
            channel: Channel {
                name: "alice".into(),
                members_type: MembersType::ImpTeamNative,
                topic_name: None,
            },
            unread: true,
            active_at: 0,
            active_at_ms: 0,
            member_status: MemberStatus::Active,
            creator_info: None,
        };
        assert!(app.conv_is_unread(&c)); // unread + not muted
        // Toggle mute (local + persisted).
        assert!(app.toggle_muted("abc".into()));
        assert!(app.is_muted("abc"));
        assert_eq!(app.settings_cache.muted, vec!["abc".to_string()]);
        assert!(!app.conv_is_unread(&c)); // muted → not "unread" for display
        // Toggle off.
        assert!(!app.toggle_muted("abc".into()));
        assert!(!app.is_muted("abc"));
        assert!(app.conv_is_unread(&c));
    }

    #[test]
    fn begin_enforces_single_in_flight() {
        let mut app = fresh_app();
        // First request starts and claims the slot.
        assert!(app.begin(InFlight::LoadMessages));
        assert!(app.is_busy());
        // A second request while one is in flight is refused, and the
        // original slot is left untouched — this is what prevents the
        // `in_flight` ↔ response desync that surfaced as a "dispatch
        // mismatch" when two requests raced.
        assert!(!app.begin(InFlight::LoadInbox));
        assert!(matches!(app.in_flight, Some(InFlight::LoadMessages)));
        // Once the slot is freed (response handled), a new request runs.
        app.in_flight = None;
        assert!(app.begin(InFlight::LoadInbox));
        assert!(matches!(app.in_flight, Some(InFlight::LoadInbox)));
    }
}
