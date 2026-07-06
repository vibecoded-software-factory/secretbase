//! Per-member action menu (`Screen::MemberActions`) — opened by right-clicking
//! a members-view row. A thin wrapper over the shared [`action_menu`] renderer.
//!
//! [`action_menu`]: crate::tui::view::action_menu

use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::flows::chat::MEMBER_ACTIONS;

pub fn draw(frame: &mut Frame, app: &App) {
    let labels: Vec<&str> = MEMBER_ACTIONS.iter().map(|(l, _)| *l).collect();
    crate::tui::view::action_menu::draw(
        frame,
        &app.theme,
        "Member",
        &labels,
        app.member_actions_selected,
    );
}
