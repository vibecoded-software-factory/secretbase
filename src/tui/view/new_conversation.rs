//! New-conversation popup (`n` from the inbox) — the shared
//! [`draw_input_popup`] with the `newconv` semantics. Enter confirms,
//! Esc cancels.

use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::view::widgets::draw_input_popup;

pub fn draw(frame: &mut Frame, app: &App) {
    draw_input_popup(
        frame,
        &app.theme,
        "New conversation",
        "participants: ",
        &app.new_conv,
        &[
            ("Enter", "create"),
            ("Esc", "cancel"),
            ("", "(comma-separated usernames)"),
        ],
    );
}
