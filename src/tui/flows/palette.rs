//! Command palette (`Ctrl+P`) — a fuzzy, discoverable list of **actions**.
//!
//! Sibling of the quick switcher (`Ctrl+K`, which jumps to a *conversation*):
//! this palette runs a *command*. It's a thin discovery layer over the existing
//! `flows::*` — `Enter` on a row calls the very function the command's keybinding
//! would, so it never diverges from the real behaviour. Each row also shows its
//! keybinding, so the palette doubles as an executable cheat-sheet (the `F1`
//! help is read-only; this acts). The command set is **context-aware**: only
//! the actions valid for where you are are listed (like the help popup).

use crate::tui::app::{App, ConvAction};
use crate::tui::flows::{chat, teams};
use crate::tui::screens::{Focus, Screen};

/// One invocable command. Every arm maps to an existing `flows::*` entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteAction {
    NewConversation,
    RefreshInbox,
    MarkRead,
    ToggleMute,
    ToggleFavorite,
    Ignore,
    Block,
    Report,
    Unhide,
    ChannelBrowser,
    ConvSearch,
    Attach,
    SelectMode,
    CloseConversation,
    QuickSwitcher,
    GlobalSearch,
    Teams,
    Settings,
    Help,
}

/// A palette row's static metadata (the runtime availability is decided in
/// [`palette_commands`]).
#[derive(Debug, Clone, Copy)]
pub struct Command {
    pub action: PaletteAction,
    /// Shown label.
    pub label: &'static str,
    /// The keybinding that also triggers it (shown right-aligned).
    pub keys: &'static str,
    /// Extra search terms so fuzzy matching finds it by intent, not just label.
    pub keywords: &'static str,
    /// Display group header.
    pub category: &'static str,
}

const fn cmd(
    action: PaletteAction,
    label: &'static str,
    keys: &'static str,
    keywords: &'static str,
    category: &'static str,
) -> Command {
    Command {
        action,
        label,
        keys,
        keywords,
        category,
    }
}

/// A row in the rendered palette: a non-selectable category header, or a
/// command. Headers only appear with an empty query (the filtered view is flat).
pub enum PaletteRow {
    Header(&'static str),
    Cmd(Command),
}

/// The commands available in the **current context**, in canonical (grouped)
/// order. Conversation-scoped commands appear only when a conversation is open;
/// the per-conversation status actions only when one is selected/open.
fn palette_commands(app: &App) -> Vec<Command> {
    use PaletteAction::*;
    let has_conv = app.selected_conversation().is_some() || app.open_conv_id.is_some();
    let conv_open = app.open_conv_id.is_some();
    let sel_is_team = app
        .selected_conversation()
        .map(|c| c.channel.members_type.is_team())
        .unwrap_or(false);

    let mut v: Vec<Command> = Vec::new();
    // ── Chat ──
    v.push(cmd(
        NewConversation,
        "New conversation",
        "n",
        "create dm chat",
        "Chat",
    ));
    v.push(cmd(
        RefreshInbox,
        "Refresh inbox",
        "r",
        "reload sync",
        "Chat",
    ));
    if has_conv {
        v.push(cmd(MarkRead, "Mark as read", "e", "seen unread", "Chat"));
        v.push(cmd(
            ToggleMute,
            "Toggle mute (local)",
            "u",
            "silence quiet",
            "Chat",
        ));
        v.push(cmd(
            ToggleFavorite,
            "Toggle \u{2605} favorite (local)",
            "s",
            "star pin",
            "Chat",
        ));
        v.push(cmd(
            Ignore,
            "Ignore conversation",
            "Shift+I",
            "hide",
            "Chat",
        ));
        v.push(cmd(
            Block,
            "Block conversation",
            "Shift+B",
            "hide remove",
            "Chat",
        ));
        v.push(cmd(
            Report,
            "Report conversation",
            "Shift+R",
            "flag hide",
            "Chat",
        ));
    }
    v.push(cmd(
        Unhide,
        "Unhide a chat by name",
        "Shift+H",
        "restore blocked",
        "Chat",
    ));
    if sel_is_team {
        v.push(cmd(
            ChannelBrowser,
            "Channel browser",
            "c",
            "channels team join leave create",
            "Chat",
        ));
    }
    // ── Conversation (only when one is open) ──
    if conv_open {
        v.push(cmd(
            ConvSearch,
            "Search this conversation",
            "Ctrl+F",
            "find message regexp",
            "Conversation",
        ));
        v.push(cmd(
            Attach,
            "Attach a file",
            "Alt+A",
            "upload image file",
            "Conversation",
        ));
        v.push(cmd(
            SelectMode,
            "Select messages",
            "Alt+V",
            "edit delete react pin reply copy",
            "Conversation",
        ));
        v.push(cmd(
            CloseConversation,
            "Close conversation",
            "Esc",
            "back",
            "Conversation",
        ));
    }
    // ── Navigate ──
    v.push(cmd(
        QuickSwitcher,
        "Quick switcher (jump to chat)",
        "Ctrl+K",
        "goto conversation jump",
        "Navigate",
    ));
    v.push(cmd(
        GlobalSearch,
        "Global search",
        "Ctrl+G",
        "find messages inbox",
        "Navigate",
    ));
    v.push(cmd(Teams, "Teams", "t", "memberships roles", "Navigate"));
    // ── App ──
    v.push(cmd(
        Settings,
        "Settings",
        "F10",
        "preferences theme config",
        "App",
    ));
    v.push(cmd(Help, "Help", "F1", "shortcuts keys cheatsheet", "App"));
    v
}

/// The context commands narrowed by the palette query (case-insensitive
/// substring over label + keywords), in canonical order. This is exactly what
/// `App::palette_selected` indexes.
pub fn filtered_commands(app: &App) -> Vec<Command> {
    let q = app.palette.text().trim().to_lowercase();
    palette_commands(app)
        .into_iter()
        .filter(|c| {
            q.is_empty()
                || format!("{} {}", c.label, c.keywords)
                    .to_lowercase()
                    .contains(&q)
        })
        .collect()
}

/// The rows to render: with an empty query, grouped under category headers;
/// with a query, a flat filtered list.
pub fn palette_rows(app: &App) -> Vec<PaletteRow> {
    let cmds = filtered_commands(app);
    if !app.palette.text().trim().is_empty() {
        return cmds.into_iter().map(PaletteRow::Cmd).collect();
    }
    let mut rows = Vec::new();
    let mut last_cat = "";
    for c in cmds {
        if c.category != last_cat {
            rows.push(PaletteRow::Header(c.category));
            last_cat = c.category;
        }
        rows.push(PaletteRow::Cmd(c));
    }
    rows
}

pub fn open_command_palette(app: &mut App) {
    app.palette_from = app.screen;
    app.palette.clear();
    app.palette_selected = 0;
    app.screen = Screen::CommandPalette;
}

pub fn close_command_palette(app: &mut App) {
    app.palette.clear();
    app.screen = app.palette_from;
}

/// Runs the highlighted command (or closes if the filtered list is empty).
pub fn palette_run_selected(app: &mut App) {
    let cmds = filtered_commands(app);
    match cmds.get(app.palette_selected).copied() {
        Some(c) => {
            app.palette.clear();
            run_palette_action(app, c.action);
        }
        None => close_command_palette(app),
    }
}

/// Dispatches a command to its existing flow. Restores the base screen the
/// palette floated over first; the flow may then re-navigate (open a popup,
/// switch screen, …), exactly as its keybinding would.
pub fn run_palette_action(app: &mut App, action: PaletteAction) {
    use PaletteAction::*;
    app.screen = app.palette_from;
    match action {
        NewConversation => chat::open_new_conversation(app),
        RefreshInbox => chat::request_load_inbox(app),
        MarkRead => chat::request_mark_read(app),
        ToggleMute => chat::toggle_muted_conversation(app),
        ToggleFavorite => chat::toggle_favorite_conversation(app),
        Ignore => chat::open_conv_action(app, ConvAction::Ignore),
        Block => chat::open_conv_action(app, ConvAction::Block),
        Report => chat::open_conv_action(app, ConvAction::Report),
        Unhide => chat::open_unhide(app),
        ChannelBrowser => chat::open_channel_browser(app),
        ConvSearch => {
            chat::open_conv_search(app);
            app.focus = Focus::ChatSearch;
        }
        Attach => crate::tui::input::conversation::open_attach_picker(app),
        SelectMode => {
            app.focus = Focus::Chat;
            chat::enter_select_mode(app);
        }
        CloseConversation => chat::close_conversation(app),
        QuickSwitcher => chat::open_quick_switcher(app),
        GlobalSearch => chat::open_search_global(app),
        Teams => teams::open_teams(app),
        Settings => app.open_settings(),
        Help => {
            app.help_from = app.palette_from;
            app.help_scroll = 0;
            app.screen = Screen::Help;
        }
    }
}
