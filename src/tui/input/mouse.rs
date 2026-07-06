//! Mouse-event translation.
//!
//! Maps screen-space clicks to semantic focus changes / row selection via
//! [`crate::tui::mouse_areas::MouseAreas`] rectangles populated by the view.

use crossterm::event::{MouseEvent, MouseEventKind};

use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::input::common::clamp_move;
use crate::tui::input::conversation;
use crate::tui::mouse_areas::hit_test;
use crate::tui::screens::{Focus, Screen};
use crate::tui::view::widgets::table_row_at;

pub fn handle(app: &mut App, ev: MouseEvent) {
    // Reject clicks whose coordinates predate the most recent resize.
    if app.mouse_areas.frame_size != app.last_terminal_size {
        return;
    }
    // The file picker floats over every screen — while it's open the wheel
    // moves *its* selection (through the same key path as ↑/↓, so Outcome
    // handling can't diverge), never whatever sits underneath.
    if app.file_picker.is_some() {
        let code = match ev.kind {
            MouseEventKind::ScrollUp => crossterm::event::KeyCode::Up,
            MouseEventKind::ScrollDown => crossterm::event::KeyCode::Down,
            _ => return,
        };
        crate::tui::input::file_picker_key(
            app,
            crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
        );
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
    // The wheel scrolls whatever is active — position-aware across the home's
    // panes, whole-screen on a single-list overlay. Clicks only mean something
    // on the home screen for now.
    let delta = match ev.kind {
        MouseEventKind::ScrollUp => -1isize,
        MouseEventKind::ScrollDown => 1isize,
        MouseEventKind::Down(_) if picker_screen(app.screen) => {
            // Click selects the row under the pointer; clicking the already-
            // selected row activates it — the tree/messages contract, now on
            // every picker (the hit map comes from the shared skeleton).
            if let Some(item) = crate::tui::view::widgets::picker_row_at(ev.column, ev.row) {
                picker_click(app, item);
            }
            return;
        }
        _ => {
            if app.screen == Screen::Inbox {
                handle_home(app, ev);
            }
            return;
        }
    };
    match app.screen {
        Screen::Inbox => handle_home(app, ev), // position-aware pane scroll
        Screen::Teams => {
            app.teams.selected = clamp_move(app.teams.selected, delta, app.teams.list.len());
        }
        Screen::ChannelBrowser => chat::channel_browser_move(app, delta),
        Screen::Members => chat::members_move(app, delta),
        Screen::SearchGlobal => {
            let len = app.global_search.results.len();
            app.global_search.selected = clamp_move(app.global_search.selected, delta, len);
        }
        Screen::React => {
            let len = app.emoji.filtered().len();
            app.react_selected = clamp_move(app.react_selected, delta, len);
        }
        Screen::QuickSwitcher => {
            let len = app.switcher_selectable().len();
            app.switcher.selected = clamp_move(app.switcher.selected, delta, len);
        }
        Screen::CommandPalette => {
            let len = crate::tui::flows::palette::filtered_commands(app).len();
            app.palette.selected = clamp_move(app.palette.selected, delta, len);
        }
        Screen::ConvSearch => {
            let len = app.conv_search.results.len();
            app.conv_search.selected = clamp_move(app.conv_search.selected, delta, len);
        }
        Screen::Help => {
            app.help_scroll = if delta < 0 {
                app.help_scroll.saturating_sub(3)
            } else {
                app.help_scroll.saturating_add(3)
            };
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
    match ev.kind {
        MouseEventKind::Down(_) => {
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
            // Tree pane (mouse_areas.source): focus + select/activate the row.
            if hit_test(c, r, app.mouse_areas.source) {
                app.focus = Focus::Tree;
                let len = app.tree_rows().len();
                if let Some(idx) = table_row_at(app.mouse_areas.source, r, app.list_scroll, len) {
                    app.tree_selected = idx;
                    chat::tree_activate(app);
                }
                return;
            }
            // Chat pane (mouse_areas.list): focus + click-to-select a message.
            if hit_test(c, r, app.mouse_areas.list) {
                app.focus = Focus::Chat;
                let clicked = app
                    .mouse_areas
                    .message_rows
                    .iter()
                    .find(|(rect, _)| hit_test(c, r, *rect))
                    .map(|(_, idx)| *idx);
                if let Some(idx) = clicked {
                    // Click selects; clicking the already-selected message
                    // *activates* it (a reply jumps to its quoted parent) —
                    // the same select-then-activate the tree click uses.
                    if app.select.cursor == Some(idx) {
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
        MouseEventKind::ScrollUp if hit_test(c, r, app.mouse_areas.source) => {
            chat::tree_move(app, -1);
        }
        MouseEventKind::ScrollDown if hit_test(c, r, app.mouse_areas.source) => {
            chat::tree_move(app, 1);
        }
        MouseEventKind::ScrollUp if hit_test(c, r, app.mouse_areas.cmd_log) => {
            app.cmdlog.move_cursor(-1);
        }
        MouseEventKind::ScrollDown if hit_test(c, r, app.mouse_areas.cmd_log) => {
            app.cmdlog.move_cursor(1);
        }
        MouseEventKind::ScrollUp if hit_test(c, r, app.mouse_areas.messages) => {
            wheel_messages(app, -1);
        }
        MouseEventKind::ScrollDown if hit_test(c, r, app.mouse_areas.messages) => {
            wheel_messages(app, 1);
        }
        // Column fallback — the wheel must not die on borders, the compose
        // box, the header row or the status strip: anywhere in the chat
        // column scrolls the history, anywhere else scrolls the tree.
        MouseEventKind::ScrollUp => {
            if app.mouse_areas.messages.width > 0 && c >= app.mouse_areas.messages.x {
                wheel_messages(app, -1);
            } else {
                chat::tree_move(app, -1);
            }
        }
        MouseEventKind::ScrollDown => {
            if app.mouse_areas.messages.width > 0 && c >= app.mouse_areas.messages.x {
                wheel_messages(app, 1);
            } else {
                chat::tree_move(app, 1);
            }
        }
        _ => {}
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
