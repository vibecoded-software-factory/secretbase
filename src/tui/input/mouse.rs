//! Mouse-event translation.
//!
//! Maps screen-space clicks to semantic focus changes / row selection via
//! [`crate::tui::mouse_areas::MouseAreas`] rectangles populated by the view.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::input::common::clamp_move;
use crate::tui::input::conversation;
use crate::tui::mouse_areas::hit_test;
use crate::tui::screens::{Focus, Screen};
use crate::tui::view::widgets::{ClickAction, ScrollTarget, table_row_at};

pub fn handle(app: &mut App, ev: MouseEvent) {
    // Reject clicks whose coordinates predate the most recent resize.
    if app.mouse_areas.frame_size != app.last_terminal_size {
        return;
    }
    // The file picker floats over every screen — while it's open the wheel
    // moves *its* selection (through the same key path as ↑/↓, so Outcome
    // handling can't diverge), never whatever sits underneath.
    if app.file_picker.is_some() {
        match ev.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let code = if matches!(ev.kind, MouseEventKind::ScrollUp) {
                    crossterm::event::KeyCode::Up
                } else {
                    crossterm::event::KeyCode::Down
                };
                crate::tui::input::file_picker_key(
                    app,
                    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
                );
            }
            MouseEventKind::Down(_) => {
                crate::tui::input::file_picker_click(app, ev.column, ev.row);
            }
            _ => {}
        }
        return;
    }
    // A click outside any centered overlay dismisses it — one generic path for
    // every modal (picker / confirm / input / settings / help), whose rect the
    // drawing widget recorded this frame. A click inside falls through to the
    // overlay's own handling below.
    if let MouseEventKind::Down(_) = ev.kind
        && let Some(rect) = crate::tui::view::widgets::active_modal_rect()
        && !hit_test(ev.column, ev.row, rect)
    {
        dismiss_overlay(app);
        return;
    }
    // Clickable chrome buttons (confirm yes/no, the F1/F10 status anchor) — the
    // mouse twin of their keys. Checked after the click-outside dismiss so a
    // click *inside* a confirm popup reaches its buttons, while a click outside
    // still cancels.
    if let MouseEventKind::Down(_) = ev.kind
        && let Some(action) = crate::tui::view::widgets::button_at(ev.column, ev.row)
    {
        apply_click_action(app, action);
        return;
    }
    // Section tabs woven into the Home shell's right-pane border are clickable
    // across all three sections — the mouse twin of `t` / `Alt+M`. Handle a tab
    // click before the per-screen routing below: on Teams a click otherwise
    // does nothing, and on the channel browser it would route to the picker.
    if matches!(ev.kind, MouseEventKind::Down(_))
        && matches!(
            app.screen,
            Screen::Inbox | Screen::Teams | Screen::ChannelBrowser
        )
        && section_tab_click(app, ev.column, ev.row)
    {
        return;
    }
    match ev.kind {
        MouseEventKind::Down(button) => {
            if app.screen == Screen::Login {
                // Focus a login field / press a login button.
                crate::tui::input::login::mouse(app, ev.column, ev.row);
            } else if app.screen == Screen::Settings {
                // Click a sidebar section / panel row / theme preset.
                crate::tui::input::settings::mouse(app, ev.column, ev.row);
            } else if picker_screen(app.screen) {
                // Click selects the row under the pointer; clicking the already-
                // selected row activates it — the tree/messages contract, now on
                // every picker (the hit map comes from the shared skeleton).
                if let Some(item) = crate::tui::view::widgets::picker_row_at(ev.column, ev.row) {
                    // Context menus run on a single click (no select-first).
                    if app.screen == Screen::MessageActions {
                        app.msg_actions_selected = item;
                        chat::run_message_action(app);
                    } else if app.screen == Screen::ConvActions {
                        app.conv_actions_selected = item;
                        chat::run_conv_action(app);
                    } else if app.screen == Screen::ChannelActions {
                        app.channel_actions_selected = item;
                        chat::run_channel_action(app);
                    } else if app.screen == Screen::MemberActions {
                        app.member_actions_selected = item;
                        chat::run_member_action(app);
                    } else if app.screen == Screen::ChannelBrowser && button == MouseButton::Right {
                        // Right-click a channel → its action menu.
                        chat::open_channel_actions(app, item);
                    } else if app.screen == Screen::Members && button == MouseButton::Right {
                        // Right-click a member → its action menu.
                        chat::open_member_actions(app, item);
                    } else {
                        picker_click(app, item);
                    }
                }
            } else if matches!(app.screen, Screen::Inbox | Screen::Teams) {
                // The Home shell handles clicks on its own panes (tree, command
                // log, the active right-pane list); the channel browser is a
                // picker above.
                handle_home(app, ev);
            }
        }
        // One generic wheel path: scroll whatever registered region sits under
        // the pointer. The widget layer records those regions each frame, so
        // there is no per-screen `match` here — a new scrollable list is one
        // `register_scroll` call at its draw site.
        MouseEventKind::ScrollUp => {
            if let Some(t) = crate::tui::view::widgets::scroll_target_at(ev.column, ev.row) {
                apply_scroll(app, t, -1);
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(t) = crate::tui::view::widgets::scroll_target_at(ev.column, ev.row) {
                apply_scroll(app, t, 1);
            }
        }
        _ => {}
    }
}

/// The single table mapping a [`ScrollTarget`] to the state its wheel moves —
/// the only place that knows how each list scrolls. Every scrollable widget
/// registers its rect + target as it draws (`register_scroll`); the wheel
/// handler above dispatches here purely by pointer position.
fn apply_scroll(app: &mut App, target: ScrollTarget, delta: isize) {
    match target {
        ScrollTarget::Tree => chat::tree_move(app, delta),
        ScrollTarget::CmdLog => app.cmdlog.move_cursor(delta),
        ScrollTarget::Messages => wheel_messages(app, delta),
        ScrollTarget::Help => {
            app.help_scroll = if delta < 0 {
                app.help_scroll.saturating_sub(3)
            } else {
                app.help_scroll.saturating_add(3)
            };
        }
        ScrollTarget::Teams => {
            let len = app.teams.filtered().len();
            app.teams.selected = clamp_move(app.teams.selected, delta, len);
        }
        ScrollTarget::Find => {
            let len = app.filtered_cache.len();
            app.find_selected = clamp_move(app.find_selected, delta, len);
        }
        ScrollTarget::ChannelBrowser => chat::channel_browser_move(app, delta),
        ScrollTarget::Members => chat::members_move(app, delta),
        ScrollTarget::React => {
            let len = app.emoji.filtered().len();
            app.react_selected = clamp_move(app.react_selected, delta, len);
        }
        ScrollTarget::Palette => {
            let len = crate::tui::flows::palette::filtered_commands(app).len();
            app.palette.selected = clamp_move(app.palette.selected, delta, len);
        }
        ScrollTarget::Switcher => {
            let len = app.switcher_selectable().len();
            app.switcher.selected = clamp_move(app.switcher.selected, delta, len);
        }
        ScrollTarget::ConvSearch => {
            let len = app.conv_search.results.len();
            app.conv_search.selected = clamp_move(app.conv_search.selected, delta, len);
        }
        ScrollTarget::GlobalSearch => {
            let len = app.global_search.results.len();
            app.global_search.selected = clamp_move(app.global_search.selected, delta, len);
        }
        ScrollTarget::Giphy => {
            let len = app.giphy.results.len();
            app.giphy.selected = clamp_move(app.giphy.selected, delta, len);
        }
    }
}

/// Dismisses the active centered overlay — the mouse twin of `Esc`. Called when
/// a click lands outside the overlay's rect. Confirms cancel (the safe default);
/// help/settings step back to their base screen.
fn dismiss_overlay(app: &mut App) {
    match app.screen {
        Screen::React => chat::close_react(app),
        Screen::GiphySearch => chat::close_giphy_search(app),
        Screen::QuickSwitcher => chat::close_quick_switcher(app),
        Screen::CommandPalette => crate::tui::flows::palette::close_command_palette(app),
        Screen::ConvSearch => chat::close_conv_search(app),
        Screen::SearchGlobal => chat::close_search_global(app),
        Screen::Members => chat::close_members(app),
        Screen::NewConversation => chat::close_new_conversation(app),
        Screen::UnhideConversation => chat::close_unhide(app),
        Screen::ConfirmDeleteMessage => chat::close_delete_confirm(app),
        Screen::ConfirmConvAction => chat::cancel_conv_action(app),
        Screen::ConfirmLogout => app.screen = Screen::Inbox,
        Screen::Settings => app.close_settings(),
        Screen::Help => app.screen = app.help_from,
        // Right-click context menus close back to their base on a click outside.
        Screen::MessageActions => chat::close_message_actions(app),
        Screen::ConvActions => chat::close_conv_actions(app),
        Screen::ChannelActions => chat::close_channel_actions(app),
        Screen::MemberActions => chat::close_member_actions(app),
        _ => {}
    }
}

/// Dispatches a click on a registered chrome button — the mouse twin of its key.
fn apply_click_action(app: &mut App, action: ClickAction) {
    match action {
        ClickAction::ConfirmYes => confirm_active(app),
        // Cancel is the safe default — the same path a click-outside takes.
        ClickAction::ConfirmNo => dismiss_overlay(app),
        ClickAction::OpenHelp => {
            if app.screen == Screen::Help {
                app.screen = app.help_from;
            } else {
                app.help_from = app.screen;
                app.screen = Screen::Help;
            }
        }
        ClickAction::OpenSettings => app.open_settings(),
    }
}

/// Commits the active confirm popup (the `[ confirm ]` click) — mirrors each
/// confirm's `run_confirm` commit closure.
fn confirm_active(app: &mut App) {
    match app.screen {
        Screen::ConfirmLogout => {
            // Return to the inbox; only `handle_logout_response` moves to Login.
            app.screen = Screen::Inbox;
            crate::tui::flows::auth::request_logout(app);
        }
        Screen::ConfirmConvAction => chat::confirm_conv_action(app),
        Screen::ConfirmDeleteMessage => {
            app.screen = Screen::Inbox;
            app.focus = Focus::Chat;
            chat::request_delete_selected_message(app);
        }
        _ => {}
    }
}

/// Screens rendered on the shared picker-modal skeleton (click-to-select).
fn picker_screen(s: Screen) -> bool {
    matches!(
        s,
        Screen::QuickSwitcher
            | Screen::CommandPalette
            | Screen::React
            | Screen::ConvSearch
            | Screen::SearchGlobal
            | Screen::ChannelBrowser
            | Screen::Members
            | Screen::GiphySearch
            | Screen::MessageActions
            | Screen::ConvActions
            | Screen::ChannelActions
            | Screen::MemberActions
    )
}

/// Handles a click on the section tabs woven into the right-pane border —
/// `Messages` · `Teams` · `Find` — switching sections (the mouse twin of `t` /
/// `Alt+M`). Returns true when the click landed on a tab.
fn section_tab_click(app: &mut App, c: u16, r: u16) -> bool {
    if hit_test(c, r, app.mouse_areas.tab_teams) {
        match app.screen {
            Screen::Teams => {} // already at the teams list
            // In the channel-browser drill-down the Teams tab steps back up.
            Screen::ChannelBrowser => chat::close_channel_browser(app),
            _ => crate::tui::flows::teams::open_teams(app),
        }
        return true;
    }
    if hit_test(c, r, app.mouse_areas.tab_messages) {
        // Back to the Messages section: the open conversation, or the tree when
        // none is open (matches `Alt+M` / `close_teams`).
        crate::tui::flows::teams::close_teams(app);
        return true;
    }
    if hit_test(c, r, app.mouse_areas.tab_find) {
        // Show the Find landing: leave any section and close the open
        // conversation (draft-safe), then focus the landing.
        if app.open_conv_id.is_some() {
            chat::close_conversation(app);
        } else {
            app.screen = Screen::Inbox;
        }
        app.focus = Focus::Chat;
        return true;
    }
    false
}

/// Select-then-activate for a clicked picker row.
fn picker_click(app: &mut App, item: usize) {
    let (sel, activate): (&mut usize, fn(&mut App)) = match app.screen {
        Screen::QuickSwitcher => (
            &mut app.switcher.selected,
            chat::quick_switcher_open_selected,
        ),
        Screen::CommandPalette => (
            &mut app.palette.selected,
            crate::tui::flows::palette::palette_run_selected,
        ),
        Screen::React => (&mut app.react_selected, chat::request_send_reaction),
        Screen::ConvSearch => (
            &mut app.conv_search.selected,
            chat::conv_search_jump_selected,
        ),
        Screen::SearchGlobal => (
            &mut app.global_search.selected,
            chat::open_selected_search_result,
        ),
        Screen::ChannelBrowser => (
            &mut app.channel_browser.selected,
            chat::channel_browser_activate,
        ),
        // Members has no Enter action — click just moves the cursor.
        Screen::Members => (&mut app.members.selected, |_| {}),
        Screen::GiphySearch => (&mut app.giphy.selected, chat::giphy_send_selected),
        _ => return,
    };
    if *sel == item {
        activate(app);
    } else {
        *sel = item;
    }
}

fn handle_home(app: &mut App, ev: MouseEvent) {
    let (c, r) = (ev.column, ev.row);
    // Only clicks reach here; the wheel is handled generically in `handle`.
    if let MouseEventKind::Down(button) = ev.kind {
        // Compose-bar chips: emoji picker (insert mode) and the attach
        // file picker — the compose's clickable buttons.
        if hit_test(c, r, app.mouse_areas.compose_gif) {
            chat::open_giphy_search(app);
            return;
        }
        if hit_test(c, r, app.mouse_areas.compose_emoji) {
            chat::open_emoji_for_compose(app);
            return;
        }
        if hit_test(c, r, app.mouse_areas.compose_attach) {
            crate::tui::input::conversation::open_attach_picker(app);
            return;
        }
        if hit_test(c, r, app.mouse_areas.compose_send) {
            // The mouse twin of Enter — submit the draft.
            crate::tui::input::conversation::submit_compose(app);
            return;
        }
        // Tree pane (mouse_areas.source): focus + select/activate the row.
        if hit_test(c, r, app.mouse_areas.source) {
            app.focus = Focus::Tree;
            let len = app.tree_rows().len();
            if let Some(idx) = table_row_at(app.mouse_areas.source, r, app.list_scroll, len) {
                let is_conv = matches!(
                    app.tree_rows().get(idx),
                    Some(crate::tui::app::TreeRow::Conv { .. })
                );
                if button == MouseButton::Right && is_conv {
                    // Right-click a conversation → its action menu.
                    chat::open_conv_actions(app, idx);
                } else {
                    // Left-click selects + activates (open conv / toggle group).
                    app.tree_selected = idx;
                    chat::tree_activate(app);
                }
            }
            return;
        }
        // Right pane (mouse_areas.list): focus it (stealing focus from the
        // tree/log if need be), then act by section — a team row, a
        // Find-landing conversation, or a message to select.
        if hit_test(c, r, app.mouse_areas.list) {
            app.focus = Focus::Chat;
            if app.screen == Screen::Teams {
                // A team row drills into its channels (the Enter action).
                if let Some(item) = crate::tui::view::widgets::picker_row_at(c, r) {
                    app.teams.selected = item;
                    let team = {
                        let filtered = app.teams.filtered();
                        filtered
                            .get(item)
                            .and_then(|&i| app.teams.list.get(i))
                            .map(|tm| tm.name.clone())
                    };
                    if let Some(team) = team {
                        chat::open_channel_browser_for_team(app, team);
                    }
                }
            } else if app.open_conv_id.is_none() {
                // The Find landing: a row opens that conversation (single
                // click, like the tree — this list *is* the conversations).
                if let Some(item) = crate::tui::view::widgets::picker_row_at(c, r) {
                    app.find_selected = item;
                    if let Some(&i) = app.filtered_cache.get(item) {
                        let id = app.conversations[i].id.clone();
                        chat::enter_conversation(app, id);
                    }
                }
            } else if let Some(idx) = app
                .mouse_areas
                .message_rows
                .iter()
                .find(|(rect, _)| hit_test(c, r, *rect))
                .map(|(_, idx)| *idx)
            {
                if button == MouseButton::Right {
                    // Right-click opens the per-message action menu on that
                    // message (react / reply / edit / delete / copy).
                    chat::open_message_actions(app, idx);
                } else if app.select.cursor == Some(idx) {
                    // Left-click selects; clicking the already-selected message
                    // *activates* it (a reply jumps to its quoted parent) —
                    // the tree's select-then-activate.
                    chat::select_activate(app);
                } else {
                    app.select.cursor = Some(idx);
                }
            }
            return;
        }
        // Other panels just take focus; the command log also seats its
        // visual-select cursor on focus.
        if let Some(target) = app.mouse_areas.focus_for(c, r) {
            app.focus = target;
            if target == crate::tui::screens::Focus::CmdLog {
                app.cmdlog.enter();
            }
        }
    }
}

/// One wheel notch over the message history (`dir` −1 = up / 1 = down).
///
/// In **Select mode the wheel moves the cursor** — the render follows the
/// highlighted message, so mutating the raw viewport offset there was a
/// dead control: the view stayed pinned to the cursor while
/// `messages_scroll` silently accumulated (triggering invisible pagination
/// and a jump on exit). Moving the selection matches what the wheel does on
/// the tree and the command log, and the cursor path paginates at the top
/// edge. In Compose mode it stays a viewport scroll.
fn wheel_messages(app: &mut App, dir: isize) {
    if app.select.cursor.is_some() {
        for _ in 0..3 {
            if dir < 0 {
                chat::select_move_up(app);
            } else {
                chat::select_move_down(app);
            }
        }
    } else if dir < 0 {
        app.pagination.scroll = app.pagination.scroll.saturating_add(3);
        conversation::maybe_queue_older(app);
    } else {
        app.pagination.scroll = app.pagination.scroll.saturating_sub(3);
    }
}
