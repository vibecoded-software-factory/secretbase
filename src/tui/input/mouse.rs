//! Mouse-event translation.
//!
//! Maps screen-space clicks to semantic focus changes / list-row
//! selection via [`crate::tui::mouse_areas::MouseAreas`] rectangles
//! populated by the view layer.

use crossterm::event::{MouseEvent, MouseEventKind};

use crate::tui::app::App;
use crate::tui::input::{common, conversation};
use crate::tui::mouse_areas::hit_test;
use crate::tui::screens::{Focus, Screen};

pub fn handle(app: &mut App, ev: MouseEvent) {
    // Reject clicks whose coordinates predate the most recent resize.
    if app.mouse_areas.frame_size != app.last_terminal_size {
        return;
    }
    match app.screen {
        Screen::Inbox => handle_inbox(app, ev),
        Screen::Conversation => handle_conversation(app, ev),
        _ => {}
    }
}

fn handle_inbox(app: &mut App, ev: MouseEvent) {
    match ev.kind {
        MouseEventKind::Down(_) => {
            let scroll = app.list_scroll;
            let len = app.filtered_cache.len();
            common::list_click(app, ev.column, ev.row, scroll, len, |app, idx| {
                app.list_selected = idx;
            });
            // Clicking the filters / command-log panels just moves focus.
            if let Some(target) = app.mouse_areas.focus_for(ev.column, ev.row)
                && matches!(target, Focus::Source | Focus::Filters | Focus::CmdLog)
            {
                app.focus = target;
            }
        }
        MouseEventKind::ScrollUp
            if app.mouse_areas.focus_for(ev.column, ev.row) == Some(Focus::List) =>
        {
            crate::tui::input::nav::move_up(app);
        }
        MouseEventKind::ScrollDown
            if app.mouse_areas.focus_for(ev.column, ev.row) == Some(Focus::List) =>
        {
            crate::tui::input::nav::move_down(app);
        }
        _ => {}
    }
}

fn handle_conversation(app: &mut App, ev: MouseEvent) {
    let (c, r) = (ev.column, ev.row);
    match ev.kind {
        MouseEventKind::Down(_) => {
            // Click a message to select it (enters select mode on that row).
            let clicked = app
                .mouse_areas
                .message_rows
                .iter()
                .find(|(rect, _)| hit_test(c, r, *rect))
                .map(|(_, idx)| *idx);
            if let Some(idx) = clicked {
                app.selected_msg_idx = Some(idx);
            }
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
