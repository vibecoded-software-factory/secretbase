//! Per-message action menu (`Screen::MessageActions`) — opened by right-clicking
//! a message. A compact centered picker on the shared skeleton (so it inherits
//! click-to-run, click-outside-to-dismiss and the app's modal grammar) listing
//! the actions that operate on the message under the Select cursor.

use ratatui::{Frame, text::Line};

use crate::tui::app::App;
use crate::tui::flows::chat::MESSAGE_ACTIONS;
use crate::tui::view::widgets::{PickerModal, PickerRow, draw_picker_modal};

pub fn draw(frame: &mut Frame, app: &App) {
    let rows: Vec<PickerRow> = MESSAGE_ACTIONS
        .iter()
        .map(|(label, _)| PickerRow::Item(vec![Line::from((*label).to_string())]))
        .collect();
    draw_picker_modal(
        frame,
        &app.theme,
        PickerModal {
            title: "Message".to_string(),
            query: None,
            selected: app
                .msg_actions_selected
                .min(MESSAGE_ACTIONS.len().saturating_sub(1)),
            rows,
            empty: Vec::new(),
            legend: &[("↑/↓", "pick"), ("Enter", "do"), ("Esc", "close")],
            footer: None,
            scroll_target: None,
        },
    );
}
