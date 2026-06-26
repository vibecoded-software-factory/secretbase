//! Mouse-event translation.
//!
//! Maps screen-space clicks to semantic focus changes / list-row
//! selection via [`crate::tui::mouse_areas::MouseAreas`] rectangles
//! populated by the view layer.

use crossterm::event::{MouseEvent, MouseEventKind};

use crate::tui::app::App;
use crate::tui::input::common;
use crate::tui::screens::{Focus, Screen};

pub fn handle(app: &mut App, ev: MouseEvent) {
    // Only the inbox screen has interactive panels worth clicking on.
    if app.screen != Screen::Inbox {
        return;
    }
    // Reject clicks whose coordinates predate the most recent resize.
    if app.mouse_areas.frame_size != app.last_terminal_size {
        return;
    }

    match ev.kind {
        MouseEventKind::Down(_) => {
            let scroll = app.list_scroll;
            let len = app.filtered_cache.len();
            common::list_click(app, ev.column, ev.row, scroll, len, |app, idx| {
                app.list_selected = idx;
            });
            // Clicking the filters / command-log panels just moves focus.
            if let Some(target) = app.mouse_areas.focus_for(ev.column, ev.row)
                && matches!(target, Focus::Filters | Focus::CmdLog)
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
