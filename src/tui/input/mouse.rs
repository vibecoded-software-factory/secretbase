//! Mouse-event translation.
//!
//! Maps screen-space clicks to semantic focus changes / row selection via
//! [`crate::tui::mouse_areas::MouseAreas`] rectangles populated by the view.

use crossterm::event::{MouseEvent, MouseEventKind};

use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::input::conversation;
use crate::tui::mouse_areas::hit_test;
use crate::tui::screens::Focus;
use crate::tui::view::widgets::table_row_at;

pub fn handle(app: &mut App, ev: MouseEvent) {
    // Reject clicks whose coordinates predate the most recent resize.
    if app.mouse_areas.frame_size != app.last_terminal_size {
        return;
    }
    // Everything lives on the unified inbox/home screen now.
    handle_home(app, ev);
}

fn handle_home(app: &mut App, ev: MouseEvent) {
    let (c, r) = (ev.column, ev.row);
    match ev.kind {
        MouseEventKind::Down(_) => {
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
                    app.selected_msg_idx = Some(idx);
                }
                return;
            }
            // Other panels just take focus.
            if let Some(target) = app.mouse_areas.focus_for(c, r) {
                app.focus = target;
            }
        }
        MouseEventKind::ScrollUp if hit_test(c, r, app.mouse_areas.source) => {
            chat::tree_move(app, -1);
        }
        MouseEventKind::ScrollDown if hit_test(c, r, app.mouse_areas.source) => {
            chat::tree_move(app, 1);
        }
        MouseEventKind::ScrollUp if hit_test(c, r, app.mouse_areas.messages) => {
            app.messages_scroll = app.messages_scroll.saturating_add(3);
            conversation::maybe_queue_older(app);
        }
        MouseEventKind::ScrollDown if hit_test(c, r, app.mouse_areas.messages) => {
            app.messages_scroll = app.messages_scroll.saturating_sub(3);
        }
        _ => {}
    }
}
