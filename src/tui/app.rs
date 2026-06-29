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
    ChatEvent, Conversation, Emoji, IdentityInfo, InboxHit, LineEditor, LoweredConversation,
    MemberStatus, Message, StatusFilter, TeamMembership, fuzzy_score_lowered,
};
use crate::ports::{ClipboardPort, OpenerPort, SettingsPort, UserSettings};
use crate::tui::action::{ActionState, CmdEntry};
use crate::tui::file_picker::FilePicker;
use crate::tui::mouse_areas::MouseAreas;
use crate::tui::screens::{Focus, Screen};
use crate::tui::theme::{self, Theme};
use crate::tui::worker::{InFlight, WorkerRequest, WorkerResponse};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Maximum number of command-log entries kept in memory.
pub const CMD_LOG_LIMIT: usize = 50;

/// Number of rows the inbox list reserves before scrolling kicks in —
/// used by PgUp/PgDn handlers to compute the right step size.
pub const INBOX_VIEWPORT_ROWS: usize = 20;

/// Step size in rows for PgUp/PgDn navigation.
pub const PAGE_STEP: usize = 10;

/// A status-changing action on a single conversation, shown behind the
/// shared y/n confirm popup ([`Screen::ConfirmConvAction`]). Each maps to
/// a `keybase chat api {"method":"setstatus"}` status value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvAction {
    /// Hide from the inbox until a new message arrives (`ignored`).
    Ignore,
}

impl ConvAction {
    /// The keybase `setstatus` value.
    pub fn status(self) -> &'static str {
        match self {
            ConvAction::Ignore => "ignored",
        }
    }
    /// Spinner label while the call runs.
    pub fn running(self) -> &'static str {
        match self {
            ConvAction::Ignore => "Ignoring…",
        }
    }
    /// Feedback label on success.
    pub fn done(self) -> &'static str {
        match self {
            ConvAction::Ignore => "Ignored",
        }
    }
    /// Confirm-popup title.
    pub fn title(self) -> &'static str {
        match self {
            ConvAction::Ignore => " Ignore conversation? ",
        }
    }
    /// One-line description of the effect, shown in the confirm popup.
    pub fn note(self) -> &'static str {
        match self {
            ConvAction::Ignore => "Hides it from the inbox until a new message arrives.",
        }
    }
}

/// Which pane of the Settings overlay currently holds focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFocus {
    /// The left-hand list of sections.
    Sidebar,
    /// The right-hand panel showing the active section's options.
    Panel,
}

/// A section of the Settings overlay. Sectioned so the preferences
/// surface can grow (Clipboard, Notifications…) without changing the
/// layout. Today only [`SettingsSection::Theme`] exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Theme,
}

impl SettingsSection {
    /// Every section, in sidebar order.
    pub const ALL: [SettingsSection; 1] = [SettingsSection::Theme];

    /// The sidebar label.
    pub fn label(self) -> &'static str {
        match self {
            SettingsSection::Theme => "Theme",
        }
    }
}

/// Delivery state of an optimistic [`PendingSend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendState {
    /// In flight — waiting for the send reply.
    Pending,
    /// The send succeeded; awaiting the reconciling re-read that will
    /// replace this bubble with the real message (then it is pruned).
    Delivered,
    /// The send failed — kept on screen so the user can resend it.
    Failed,
}

/// An optimistically-rendered outgoing message (see [`App::outbox`]).
/// Shown the instant the user hits Enter and tracked to delivery, so the
/// send never feels like it waited on a round-trip — and a failure stays
/// visible with a resend affordance instead of silently vanishing.
#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct PendingSend {
    /// Conversation this send targets (matches `Conversation::id`).
    pub conv_id: String,
    /// Message body — chat content, wiped on drop.
    pub body: String,
    /// Threaded-reply target, if any.
    #[zeroize(skip)]
    pub reply_to: Option<u64>,
    /// Wall-clock send time (unix millis) for the bubble's timestamp.
    #[zeroize(skip)]
    pub sent_at_ms: u64,
    /// Current delivery state.
    #[zeroize(skip)]
    pub state: SendState,
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
pub enum TreeRow {
    /// Group header. `key` is the collapse key ([`App::DMS_KEY`] or the team
    /// name); `unread` is the count of unread conversations in the group.
    Group {
        key: String,
        label: String,
        is_team: bool,
        collapsed: bool,
        unread: usize,
    },
    /// A conversation leaf — index into [`App::conversations`].
    Conv { idx: usize },
}

/// Top-level mutable state of the TUI.
pub struct App {
    // ── Screen / focus / filter ───────────────────────────────────────────
    pub screen: Screen,
    pub focus: Focus,
    /// Status axis of the inbox filter (All / Unread).
    pub status_filter: StatusFilter,
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
    /// Selected row inside `filtered_cache`. Reset on filter/search
    /// changes.
    pub list_selected: usize,
    /// First visible row in the list — driven by scrolling.
    pub list_scroll: usize,

    /// Team memberships (from `keybase team api {"method":"list-self-memberships"}`).
    pub teams: Vec<TeamMembership>,
    /// Currently selected row inside [`Self::teams`].
    pub teams_selected: usize,

    // ── Conversation detail ──────────────────────────────────────────────
    /// Conversation id currently open on the detail screen. `None`
    /// while we're on the inbox screen.
    pub open_conv_id: Option<String>,
    /// Messages of the open conversation, in chronological order
    /// (oldest first, latest last — keybase's read response is
    /// newest-first, the flow reverses it).
    pub messages: Vec<Message>,
    /// First visible row in the detail view. The renderer pins the
    /// "latest" line to the bottom of the panel by default; this
    /// offset lets the user scroll back to read history.
    pub messages_scroll: usize,
    /// Cursor for the next *older* page of messages, supplied by the
    /// Keybase service in the previous `read` reply. `None` once the
    /// service signals it has reached the bottom of history.
    pub messages_next: Option<String>,
    /// Whether a pagination call is currently in flight — used to
    /// debounce repeated Up-arrow presses while we wait for the next
    /// older page to arrive.
    pub messages_loading_older: bool,
    /// The maximum bottom-relative scroll offset the view rendered on
    /// the last frame. The input handler reads it to know when the
    /// user has reached the top of loaded history (so it can trigger
    /// a pagination fetch).
    pub messages_max_back: usize,
    /// Message id pinned in the open conversation, derived from the
    /// most recent `Pin` system message in `messages`. `None` when
    /// the conversation has no pin (or the pin event is older than
    /// the loaded history).
    pub pinned_msg_id: Option<u64>,
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
    pub selected_msg_idx: Option<usize>,
    /// Whether the current Select-mode action (react/delete popup) was
    /// triggered from Compose via an `Alt+` shortcut. When `true`, the
    /// popup's cancel path returns the user to Compose instead of
    /// leaving them stuck in Select mode.
    pub select_from_compose: bool,
    /// Messages marked in Select mode for a multi-select copy (indices into
    /// [`Self::messages`]). Empty = copy falls back to the cursor message.
    pub msg_marks: HashSet<usize>,
    /// Anchor for `Shift+↑/↓` range shading in Select mode — the fixed end of
    /// the contiguous selection while the cursor moves.
    pub select_anchor: Option<usize>,
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
    /// list — see [`Self::filtered_emoji_indices`]).
    pub react_selected: usize,
    /// Sendable emojis from `emojilist`, fetched once and cached for the
    /// reaction picker.
    pub emojis: Vec<Emoji>,
    /// Whether [`Self::emojis`] has been loaded (so we fetch only once).
    pub emojis_loaded: bool,
    /// Whether an `emojilist` fetch is in flight (de-dupes the request).
    pub emojis_loading: bool,
    /// Per-alias reaction usage this session — floats the most-used emojis
    /// to the top of the picker (Discord-style frecency).
    pub emoji_uses: HashMap<String, u32>,

    // ── Quick switcher (Ctrl+K) ──────────────────────────────────────────
    /// Fuzzy query in the Ctrl+K quick switcher.
    pub switcher: LineEditor,
    /// Selected row in the switcher (indexes [`Self::switcher_results`]).
    pub switcher_selected: usize,
    /// Screen the switcher was opened from, restored on cancel.
    pub switcher_from: Screen,
    /// Per-conversation unsent draft text (in memory only — not persisted
    /// across restarts). Keyed by conversation id.
    pub drafts: HashMap<String, String>,
    /// When opening a conversation from a global-search hit, the message id
    /// to scroll to + highlight once it's loaded (paginating older if the
    /// match is below the first page). Cleared once landed or exhausted.
    pub pending_search_jump: Option<u64>,

    // ── In-conversation search (Ctrl+F → keybase chat api searchregexp) ──
    /// Query for the search box at the top of the conversation screen.
    pub conv_search: LineEditor,
    /// Whether the conversation search box holds focus.
    pub conv_search_active: bool,
    /// Matches from `searchregexp`, scoped to the open conversation.
    pub conv_search_results: Vec<InboxHit>,
    /// Selected row in `conv_search_results`.
    pub conv_search_selected: usize,

    // ── New-conversation popup ──────────────────────────────────────────
    /// Comma-separated usernames typed by the user in the Alt+N popup.
    pub new_conv: LineEditor,

    // ── Server-side search popup ────────────────────────────────────────
    /// Query box in the Ctrl+G popup.
    pub search_global_input: LineEditor,
    /// Hits returned by `searchinbox` for the last query.
    pub search_global_results: Vec<InboxHit>,
    /// Selected row inside [`Self::search_global_results`].
    pub search_global_selected: usize,

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

    // ── Confirm overlays (navigable y/n, jewel-style) ─────────────────────
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
    /// Free-running animation counter (wraps at 256), advanced every
    /// `tick_action`. Drives smooth animations like the skeleton shimmer —
    /// unlike `action_tick` it isn't reset by `set_action` or capped at 4.
    pub anim_tick: u8,
    /// Caller-side context for the request the worker is currently
    /// processing. `None` when idle. Set by `flows::*::request_*` and
    /// cleared by [`crate::tui::flows::apply_response`] once the
    /// matching response arrives.
    pub in_flight: Option<InFlight>,
    pub cmd_log: Vec<CmdEntry>,
    /// Number of cmd-log lines scrolled UP from the bottom. `0` keeps
    /// the latest entry pinned to the bottom-visible row; larger
    /// values walk back through history. Resets to `0` on every new
    /// `push_cmd` so the user always sees the freshest entry by
    /// default.
    pub cmd_log_scroll: usize,
    /// Cursor over the command log (absolute index into [`Self::cmd_log`])
    /// when the panel holds focus — used for the visual multi-select.
    pub cmdlog_cursor: usize,
    /// Command-log lines marked for copy (absolute indices). Empty = none;
    /// copying then falls back to the cursor line.
    pub cmdlog_marks: HashSet<usize>,
    /// Anchor for `Shift+↑/↓` range shading in the command log.
    pub cmdlog_anchor: Option<usize>,
    /// `Ctrl+W` window-nav leader is armed — the next key is read as a
    /// direction (`h/j/k/l` or an arrow) to move between panels positionally.
    pub pending_pane_nav: bool,

    // ── Settings / theme ──────────────────────────────────────────────────
    pub settings_cache: UserSettings,
    pub theme: Theme,

    // ── Settings overlay (F9) ──────────────────────────────────────────────
    /// Which pane of the Settings overlay holds focus.
    pub settings_focus: SettingsFocus,
    /// Highlighted section in the sidebar (index into
    /// [`SettingsSection::ALL`]).
    pub settings_section: usize,
    /// Highlighted preset in the Theme panel (index into
    /// [`theme::Preset::ALL`]). Previews live as it moves.
    pub settings_theme_idx: usize,
    /// Theme active when the Settings overlay opened — restored if the
    /// user cancels (`Esc`/`F9`) instead of confirming.
    pub theme_before_settings: Theme,
    /// Screen the Settings overlay was opened from (returned to on close).
    pub settings_from: Screen,

    // ── Lifecycle ─────────────────────────────────────────────────────────
    pub should_quit: bool,
    pub last_activity: Instant,
    pub start_time: Instant,
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

    // ── Injected ports (synchronous, stay on the render thread) ───────────
    pub clipboard: Box<dyn ClipboardPort>,
    pub opener: Box<dyn OpenerPort>,
    pub settings: Box<dyn SettingsPort>,
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
        let theme = theme::load(&settings.config_dir());
        // Preselect the picker on the configured preset, else the shared
        // default (Nord).
        let settings_theme_idx = theme::configured_preset(&settings.config_dir())
            .or(Some(theme::Preset::DEFAULT))
            .and_then(|p| theme::Preset::ALL.iter().position(|&q| q == p))
            .unwrap_or(0);
        Self {
            screen: Screen::Splash,
            focus: Focus::Tree,
            status_filter: StatusFilter::All,
            expanded: HashSet::new(),
            tree_selected: 0,
            identity: IdentityInfo::default(),
            conversations: Vec::new(),
            conversations_lowered: Vec::new(),
            filtered_cache: Vec::new(),
            list_selected: 0,
            list_scroll: 0,
            teams: Vec::new(),
            teams_selected: 0,
            open_conv_id: None,
            messages: Vec::new(),
            outbox: Vec::new(),
            file_picker: None,
            messages_scroll: 0,
            messages_next: None,
            messages_loading_older: false,
            messages_max_back: 0,
            pinned_msg_id: None,
            compose_open: false,
            compose: LineEditor::default(),
            edit_target_id: None,
            reply_to_id: None,
            selected_msg_idx: None,
            select_from_compose: false,
            msg_marks: HashSet::new(),
            select_anchor: None,
            conv_members: Vec::new(),
            mention_selected: 0,
            react: LineEditor::default(),
            react_selected: 0,
            // Seed the bundled standard set so the picker has content and
            // reactions resolve to glyphs immediately; the emojilist fetch
            // merges the team's custom emojis on top.
            emojis: crate::domain::emoji::standard(),
            emojis_loaded: false,
            emojis_loading: false,
            emoji_uses: HashMap::new(),
            switcher: LineEditor::default(),
            switcher_selected: 0,
            switcher_from: Screen::Inbox,
            drafts: HashMap::new(),
            pending_search_jump: None,
            conv_search: LineEditor::default(),
            conv_search_active: false,
            conv_search_results: Vec::new(),
            conv_search_selected: 0,
            new_conv: LineEditor::default(),
            search_global_input: LineEditor::default(),
            search_global_results: Vec::new(),
            search_global_selected: 0,
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
            anim_tick: 0,
            in_flight: None,
            cmd_log: Vec::new(),
            cmd_log_scroll: 0,
            cmdlog_cursor: 0,
            cmdlog_marks: HashSet::new(),
            cmdlog_anchor: None,
            pending_pane_nav: false,
            settings_cache,
            theme: theme.clone(),
            settings_focus: SettingsFocus::Sidebar,
            settings_section: 0,
            settings_theme_idx,
            theme_before_settings: theme,
            settings_from: Screen::Inbox,
            should_quit: false,
            last_activity: Instant::now(),
            start_time: Instant::now(),
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

    /// Indices into [`Self::emojis`] matching the reaction-picker query
    /// (case-insensitive substring on the alias; all when empty).
    pub fn filtered_emoji_indices(&self) -> Vec<usize> {
        let q = self.react.text().trim().to_lowercase();
        let mut idx: Vec<usize> = self
            .emojis
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.keywords.contains(&q))
            .map(|(i, _)| i)
            .collect();
        // Most-used first; `sort_by` is stable, so ties keep catalogue order.
        idx.sort_by(|&a, &b| {
            let ua = self
                .emoji_uses
                .get(&self.emojis[a].alias)
                .copied()
                .unwrap_or(0);
            let ub = self
                .emoji_uses
                .get(&self.emojis[b].alias)
                .copied()
                .unwrap_or(0);
            ub.cmp(&ua)
        });
        idx
    }

    /// Conversations matching the quick-switcher query, as indices into
    /// `conversations`. Empty query → all, most-recent first; otherwise
    /// fuzzy-ranked over the lowered projection (same scorer as the inbox).
    pub fn switcher_results(&self) -> Vec<usize> {
        let q = self.switcher.text().trim().to_lowercase();
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
        if !self.switcher.text().trim().is_empty() {
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
            .filter(|&i| self.conversations[i].unread && !drafts.contains(&i))
            .collect();
        by_recency(&mut unread);
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

    /// Opens the Settings overlay over the current screen. Stashes the
    /// originating screen and the active theme (so `Esc`/`F9` can restore
    /// it), and starts focus on the section sidebar.
    pub fn open_settings(&mut self) {
        self.settings_from = self.screen;
        self.theme_before_settings = self.theme.clone();
        self.settings_focus = SettingsFocus::Sidebar;
        self.settings_section = 0;
        self.screen = Screen::Settings;
    }

    /// Applies the highlighted preset to [`Self::theme`] as a live
    /// preview — no persistence. Called whenever the picker moves.
    pub fn settings_preview_theme(&mut self) {
        if let Some(&p) = theme::Preset::ALL.get(self.settings_theme_idx) {
            self.theme = Theme::from_palette(&p.palette());
        }
    }

    /// Confirms the highlighted preset: applies it, persists
    /// `name = "<preset>"` to `config.toml`, and closes the overlay.
    pub fn settings_confirm_theme(&mut self) {
        if let Some(&p) = theme::Preset::ALL.get(self.settings_theme_idx) {
            self.theme = Theme::from_palette(&p.palette());
            self.settings.write_theme_name(p.name());
            self.push_cmd("theme", true, format!("saved {}", p.name()));
            self.set_action(ActionState::Done(format!("Theme: {}", p.label())));
        }
        self.screen = self.settings_from;
    }

    /// Cancels the Settings overlay: restores the theme that was active
    /// when it opened (dropping any live preview) and closes it.
    pub fn settings_cancel(&mut self) {
        self.theme = self.theme_before_settings.clone();
        self.screen = self.settings_from;
    }

    /// Convenience: whether the worker is currently processing a
    /// request. Equivalent to `self.in_flight.is_some()`.
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
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
        if self.in_flight.is_some() {
            self.push_cmd("worker request", false, "busy — request ignored");
            return false;
        }
        self.in_flight = Some(slot);
        self.request_started = Some(Instant::now());
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
    /// modulo itself. Also advances `anim_tick`, a free-running counter for
    /// smoother animations (the loading skeleton's shimmer sweep).
    pub fn tick_action(&mut self) {
        self.action_tick = (self.action_tick + 1) % 4;
        self.anim_tick = self.anim_tick.wrapping_add(1);
    }

    /// Pushes a new entry to the command log, trimming to
    /// [`CMD_LOG_LIMIT`] from the front. Resets [`Self::cmd_log_scroll`]
    /// so the user always sees the freshest entry after every action.
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
        self.cmd_log.push(CmdEntry {
            cmd,
            ok,
            detail,
            duration,
        });
        let over = self.cmd_log.len().saturating_sub(CMD_LOG_LIMIT);
        if over > 0 {
            self.cmd_log.drain(..over);
            // Keep the visual-select cursor / marks pointing at the same
            // entries after the front of the ring is trimmed.
            self.cmdlog_cursor = self.cmdlog_cursor.saturating_sub(over);
            self.cmdlog_marks = self
                .cmdlog_marks
                .iter()
                .filter_map(|&i| i.checked_sub(over))
                .collect();
        }
        self.cmd_log_scroll = 0;
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
                    | Screen::SearchGlobal
                    | Screen::ConfirmDeleteMessage
                    | Screen::React
                    | Screen::QuickSwitcher
            )
    }

    /// Enters the command-log panel: seat the cursor on the newest entry and
    /// clear any prior selection.
    pub fn enter_cmdlog(&mut self) {
        self.cmdlog_cursor = self.cmd_log.len().saturating_sub(1);
        self.cmdlog_marks.clear();
        self.cmdlog_anchor = None;
    }

    /// Moves the command-log cursor by `delta`, clamped to the log. A plain
    /// move re-anchors the next `Shift+↑/↓` range.
    pub fn cmdlog_move(&mut self, delta: isize) {
        self.cmdlog_anchor = None;
        let len = self.cmd_log.len();
        if len == 0 {
            return;
        }
        let max = (len - 1) as isize;
        let cur = (self.cmdlog_cursor.min(len - 1)) as isize;
        self.cmdlog_cursor = cur.saturating_add(delta).clamp(0, max) as usize;
    }

    /// Extends a contiguous shaded selection by `delta` (Shift+↑/↓) in the
    /// command log: the anchor holds while the cursor moves; the range is
    /// re-marked each step.
    pub fn cmdlog_extend(&mut self, delta: isize) {
        let len = self.cmd_log.len();
        if len == 0 {
            return;
        }
        let max = (len - 1) as isize;
        let cur = self.cmdlog_cursor.min(len - 1);
        let anchor = *self.cmdlog_anchor.get_or_insert(cur);
        let new = (cur as isize).saturating_add(delta).clamp(0, max) as usize;
        self.cmdlog_cursor = new;
        let (lo, hi) = (anchor.min(new), anchor.max(new));
        self.cmdlog_marks = (lo..=hi).collect();
    }

    /// Toggles the mark on the cursor's command-log line (multi-select).
    pub fn cmdlog_toggle_mark(&mut self) {
        self.cmdlog_anchor = None;
        let len = self.cmd_log.len();
        if len == 0 {
            return;
        }
        let c = self.cmdlog_cursor.min(len - 1);
        if !self.cmdlog_marks.remove(&c) {
            self.cmdlog_marks.insert(c);
        }
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
        for m in &self.messages {
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

    /// Whether the `@`-mention popup should be shown / capture keys — only in
    /// the chat's compose mode with at least one match.
    pub fn mention_popup_active(&self) -> bool {
        self.focus == Focus::Chat
            && self.open_conv_id.is_some()
            && self.selected_msg_idx.is_none()
            && !self.conv_search_active
            && self.edit_target_id.is_none()
            && !self.mention_matches().is_empty()
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

    /// Recomputes [`Self::pinned_msg_id`] by scanning the loaded
    /// history for the most recent `Pin` system message. Called once
    /// after every load — pinning/unpinning emits a system message
    /// itself, so a fresh read is enough to keep the indicator
    /// accurate.
    pub fn rebuild_pinned(&mut self) {
        use crate::domain::MessageContent;
        // The first `Pin` event walking newest → oldest is
        // authoritative. A `target_id == 0` event (which Keybase uses
        // for "pin cleared") wins over older "pin set" events.
        self.pinned_msg_id = None;
        for m in self.messages.iter().rev() {
            if let MessageContent::Pin { target_id } = &m.content {
                self.pinned_msg_id = if *target_id == 0 {
                    None
                } else {
                    Some(*target_id)
                };
                return;
            }
        }
    }

    /// Clamps [`Self::list_selected`] into the bounds of
    /// [`Self::filtered_cache`]. Idempotent — when state is already
    /// synced (the common case post-[`rebuild_filter`]) this is a
    /// no-op.
    ///
    /// Called defensively from the inbox renderer so a code path
    /// that mutates `conversations` *without* refreshing the filter
    /// (e.g. an isolated `unread = false` flip from
    /// `handle_mark_read_response`) cannot leave the cursor pointing
    /// past the end of the filtered list. The render used to clamp
    /// inline but discarded the result, which meant the indicator
    /// strip ("N of M") could show `81 of 5` until something else
    /// triggered a rebuild. Now both stay consistent.
    pub fn clamp_list_selected(&mut self) {
        if self.filtered_cache.is_empty() {
            self.list_selected = 0;
        } else if self.list_selected >= self.filtered_cache.len() {
            self.list_selected = self.filtered_cache.len() - 1;
        }
    }

    /// Rebuilds [`Self::filtered_cache`] from the current filter +
    /// search query.
    pub fn rebuild_filter(&mut self) {
        let query_lc = self.search.text().to_lowercase();
        let mut indices: Vec<usize> = Vec::new();
        for (idx, conv) in self.conversations.iter().enumerate() {
            if conv.member_status != MemberStatus::Active || !self.status_filter.includes(conv) {
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
        // Land the tree cursor on the first conversation (skipping the leading
        // group header) so actions that need a selected conversation work
        // right after a load / filter / search change.
        let rows = self.tree_rows();
        self.tree_selected = rows
            .iter()
            .position(|r| matches!(r, TreeRow::Conv { .. }))
            .unwrap_or(0);
    }

    /// The conversation under the tree cursor, if it's on a conversation row
    /// (not a group header).
    pub fn selected_conversation(&self) -> Option<&Conversation> {
        match self.tree_rows().get(self.tree_selected) {
            Some(TreeRow::Conv { idx }) => self.conversations.get(*idx),
            _ => None,
        }
    }

    /// Whether a conversation counts toward the sidebar tallies.
    fn is_active(c: &Conversation) -> bool {
        c.member_status == MemberStatus::Active
    }

    /// Count of active conversations matching `filter` (whole inbox).
    pub fn count_status(&self, filter: StatusFilter) -> usize {
        self.conversations
            .iter()
            .filter(|c| Self::is_active(c) && filter.includes(c))
            .count()
    }

    /// Sentinel collapse-key for the Direct-messages group (a NUL byte can't
    /// occur in a team name, so it never collides).
    pub const DMS_KEY: &'static str = "\u{0}dms";

    /// Flattened rows of the conversation **tree**: a "Direct messages" group
    /// then one group per team, each (unless collapsed) followed by its
    /// conversations. Built from `filtered_cache` (already status/search
    /// filtered, recency-sorted). A non-empty search force-expands all groups.
    pub fn tree_rows(&self) -> Vec<TreeRow> {
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
                .filter(|&&i| self.conversations[i].unread)
                .count();
            let collapsed = !force_expand && !self.expanded.contains(&key);
            rows.push(TreeRow::Group {
                key,
                label,
                is_team: team,
                collapsed,
                unread,
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
        rows
    }

    /// Toggles a tree group's collapsed state (groups start collapsed).
    pub fn toggle_collapsed(&mut self, key: &str) {
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_string());
        }
    }

    /// Total number of conversations flagged as unread. Surfaced in
    /// the status panel of every screen so the user always knows if
    /// there is something to attend to.
    pub fn unread_total(&self) -> usize {
        self.conversations.iter().filter(|c| c.unread).count()
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
        fn write_auto_mark_read(&self, _: bool) {}
        fn write_clipboard_clear_secs(&self, _: u64) {}
        fn write_theme_name(&self, _: &str) {}
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
    fn settings_open_preview_cancel_restores_theme() {
        let mut app = fresh_app();
        app.screen = Screen::Inbox;
        let original = app.theme.accent;
        app.open_settings();
        assert_eq!(app.screen, Screen::Settings);
        assert_eq!(app.settings_from, Screen::Inbox);
        app.settings_theme_idx = 1; // dracula
        app.settings_preview_theme();
        assert_ne!(app.theme.accent, original);
        app.settings_cancel();
        assert_eq!(app.theme.accent, original);
        assert_eq!(app.screen, Screen::Inbox);
    }

    #[test]
    fn settings_confirm_applies_and_closes() {
        let mut app = fresh_app();
        app.screen = Screen::Teams;
        app.open_settings();
        app.settings_theme_idx = 1; // dracula
        app.settings_confirm_theme();
        assert_eq!(app.screen, Screen::Teams);
        assert_eq!(
            app.theme.accent,
            Theme::from_palette(&theme::Preset::Dracula.palette()).accent
        );
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
    fn clamp_list_selected_persists_clamp_when_stale() {
        let mut app = fresh_app();
        // Simulate: filter just shrank from 100 entries to 3, but
        // list_selected was pointing to row 80 before the shrink.
        // Without persistence the indicator would have read
        // "81 of 3" and the next move-up would have misbehaved.
        app.filtered_cache = vec![0, 1, 2];
        app.list_selected = 80;
        app.clamp_list_selected();
        assert_eq!(
            app.list_selected, 2,
            "clamp must persist to last valid index"
        );
    }

    #[test]
    fn clamp_list_selected_zeroes_when_filter_is_empty() {
        let mut app = fresh_app();
        app.filtered_cache.clear();
        app.list_selected = 42;
        app.clamp_list_selected();
        assert_eq!(app.list_selected, 0, "empty filter → cursor parks at 0");
    }

    #[test]
    fn clamp_list_selected_is_a_noop_when_in_range() {
        let mut app = fresh_app();
        app.filtered_cache = vec![0, 1, 2, 3, 4];
        app.list_selected = 2;
        app.clamp_list_selected();
        assert_eq!(app.list_selected, 2, "in-range index untouched");
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
