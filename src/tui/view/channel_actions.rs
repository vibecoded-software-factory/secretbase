//! Per-channel action menu (`Screen::ChannelActions`) — opened by right-clicking
//! a channel-browser row. A thin wrapper over the shared [`action_menu`]
//! renderer listing the actions that target the selected channel.
//!
//! [`action_menu`]: crate::tui::view::action_menu

use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::flows::chat::CHANNEL_ACTIONS;

pub fn draw(frame: &mut Frame, app: &App) {
    let labels: Vec<&str> = CHANNEL_ACTIONS.iter().map(|(l, _)| *l).collect();
    crate::tui::view::action_menu::draw(
        frame,
        &app.theme,
        "Channel",
        &labels,
        app.channel_actions_selected,
    );
}
