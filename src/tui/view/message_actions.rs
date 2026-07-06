//! Per-message action menu (`Screen::MessageActions`) — opened by right-clicking
//! a message. A thin wrapper over the shared [`action_menu`] renderer listing
//! the actions that operate on the message under the Select cursor.
//!
//! [`action_menu`]: crate::tui::view::action_menu

use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::flows::chat::MESSAGE_ACTIONS;

pub fn draw(frame: &mut Frame, app: &App) {
    let labels: Vec<&str> = MESSAGE_ACTIONS.iter().map(|(l, _)| *l).collect();
    crate::tui::view::action_menu::draw(
        frame,
        &app.theme,
        "Message",
        &labels,
        app.msg_actions_selected,
    );
}
