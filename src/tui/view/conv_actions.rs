//! Per-conversation action menu (`Screen::ConvActions`) — opened by
//! right-clicking a tree row. A thin wrapper over the shared [`action_menu`]
//! renderer listing the actions that target the conversation under the tree
//! cursor.
//!
//! [`action_menu`]: crate::tui::view::action_menu

use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::flows::chat::CONV_ACTIONS;

pub fn draw(frame: &mut Frame, app: &App) {
    let labels: Vec<&str> = CONV_ACTIONS.iter().map(|(l, _)| *l).collect();
    crate::tui::view::action_menu::draw(
        frame,
        &app.theme,
        "Conversation",
        &labels,
        app.conv_actions_selected,
    );
}
