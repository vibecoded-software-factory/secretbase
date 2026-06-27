//! Screen / focus enums used by the TUI state machine.

/// Top-level screens of the TUI.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Screen {
    /// Boot splash shown while `keybase status` runs.
    Splash,
    /// Login / device-provisioning hint screen (Keybase has no
    /// in-TUI password login — when not logged in the only path
    /// forward is `keybase login` in a shell).
    Login,
    /// Main inbox: conversation list with sidebar filters.
    Inbox,
    /// Teams screen — list of the user's team memberships with role
    /// and member count.
    Teams,
    /// Single-conversation detail view — messages of the selected
    /// conversation.
    Conversation,
    /// Help popup overlay.
    Help,
    /// Settings overlay — a sectioned preferences screen (Theme first),
    /// opened with `F9` from any base screen.
    Settings,
    /// Confirm-logout popup overlay.
    ConfirmLogout,
    /// Confirm a status action (ignore/…) on the selected conversation.
    ConfirmConvAction,
    /// New-conversation popup (Alt+N) — input for comma-separated
    /// usernames.
    NewConversation,
    /// Server-side search popup (Ctrl+G) — input + results list.
    SearchGlobal,
    /// Confirm-delete popup for a message inside the open
    /// conversation.
    ConfirmDeleteMessage,
    /// Emoji input popup for reacting to a selected message.
    React,
    /// Quick switcher (Ctrl+K) — fuzzy jump to any conversation.
    QuickSwitcher,
}

/// Panels inside the [`Screen::Inbox`] layout that can hold focus.
///
/// The identity bar (top) and the status strip (bottom) are chrome,
/// not focus targets — they mirror jewel's `split_main` stack.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Focus {
    /// Search / filter box.
    Search,
    /// Source picker — Direct messages + one entry per team (left rail).
    Source,
    /// Status-filter panel (All/Unread).
    Filters,
    /// Main conversation list.
    List,
    /// Bottom command-log panel.
    CmdLog,
}
