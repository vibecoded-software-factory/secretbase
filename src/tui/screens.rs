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
    /// Help popup overlay.
    Help,
    /// Settings overlay — a sectioned preferences screen (Theme first),
    /// opened with `F10` from any base screen.
    Settings,
    /// Confirm-logout popup overlay.
    ConfirmLogout,
    /// Confirm a status action (ignore/…) on the selected conversation.
    ConfirmConvAction,
    /// New-conversation popup (Alt+N) — input for comma-separated
    /// usernames.
    NewConversation,
    /// Unhide popup (Alt+H) — restore a blocked/reported conversation by
    /// name (it's gone from the inbox `list`, so it can't be selected).
    UnhideConversation,
    /// Channel browser (`c` on a team) — lists every channel of a team
    /// (`listconvsonname`) to join / open / leave.
    ChannelBrowser,
    /// Members view (`m` in the channel browser, or `Alt+P` on an open team
    /// channel) — lists a conversation's members (`listmembers`) to add/remove.
    Members,
    /// Server-side search popup (Ctrl+G) — input + results list.
    SearchGlobal,
    /// Confirm-delete popup for a message inside the open
    /// conversation.
    ConfirmDeleteMessage,
    /// Emoji input popup for reacting to a selected message.
    React,
    /// Quick switcher (Ctrl+K) — fuzzy jump to any conversation.
    QuickSwitcher,
    /// Command palette (Ctrl+P) — fuzzy list of context-aware actions.
    CommandPalette,
    /// In-conversation search (Ctrl+F) — a modal `searchregexp` box + results,
    /// jumping to the picked match (sibling of the global-search modal).
    ConvSearch,
    /// GIF search popup (giphy, user API key) — query + results; Enter
    /// sends the selected GIF's media URL to the open conversation.
    GiphySearch,
}

/// Panels inside the [`Screen::Inbox`] layout that can hold focus.
///
/// The identity bar (top) and the status strip (bottom) are chrome,
/// not focus targets — they belong to the `split_main` stack.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Focus {
    /// Search / filter box.
    Search,
    /// Conversation tree (Direct messages + teams) — the left pane.
    Tree,
    /// The chat (right pane) — compose / select.
    Chat,
    /// Bottom command-log panel.
    CmdLog,
}
