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
    // The wheel scrolls whatever is active — position-aware across the home's
    // panes, whole-screen on a single-list overlay. Clicks only mean something
    // on the home screen for now.
    let delta = match ev.kind {
        MouseEventKind::ScrollUp => -1isize,
        MouseEventKind::ScrollDown => 1isize,
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
            app.teams_selected = clamp_move(app.teams_selected, delta, app.teams.len());
        }
        Screen::ChannelBrowser => chat::channel_browser_move(app, delta),
        Screen::Members => chat::members_move(app, delta),
        Screen::SearchGlobal => {
            let len = app.search_global_results.len();
            app.search_global_selected = clamp_move(app.search_global_selected, delta, len);
        }
        Screen::React => {
            let len = app.filtered_emoji_indices().len();
            app.react_selected = clamp_move(app.react_selected, delta, len);
        }
        Screen::QuickSwitcher => {
            let len = app.switcher_selectable().len();
            app.switcher_selected = clamp_move(app.switcher_selected, delta, len);
        }
        Screen::CommandPalette => {
            let len = crate::tui::flows::palette::filtered_commands(app).len();
            app.palette_selected = clamp_move(app.palette_selected, delta, len);
        }
        Screen::ConvSearch => {
            let len = app.conv_search_results.len();
            app.conv_search_selected = clamp_move(app.conv_search_selected, delta, len);
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

fn handle_home(app: &mut App, ev: MouseEvent) {
    let (c, r) = (ev.column, ev.row);
    match ev.kind {
        MouseEventKind::Down(_) => {
            // Compose-bar chips: emoji picker (insert mode) and the attach
            // file picker — the compose's clickable buttons.
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
                    if app.selected_msg_idx == Some(idx) {
                        chat::select_activate(app);
                    } else {
                        app.selected_msg_idx = Some(idx);
                    }
                }
                return;
            }
            // Other panels just take focus; the command log also seats its
            // visual-select cursor on focus.
            if let Some(target) = app.mouse_areas.focus_for(c, r) {
                app.focus = target;
                if target == crate::tui::screens::Focus::CmdLog {
                    app.enter_cmdlog();
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
            app.cmdlog_move(-1);
        }
        MouseEventKind::ScrollDown if hit_test(c, r, app.mouse_areas.cmd_log) => {
            app.cmdlog_move(1);
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
    if app.selected_msg_idx.is_some() {
        for _ in 0..3 {
            if dir < 0 {
                chat::select_move_up(app);
            } else {
                chat::select_move_down(app);
            }
        }
    } else if dir < 0 {
        app.messages_scroll = app.messages_scroll.saturating_add(3);
        conversation::maybe_queue_older(app);
    } else {
        app.messages_scroll = app.messages_scroll.saturating_sub(3);
    }
}
