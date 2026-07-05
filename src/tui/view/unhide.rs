//! Unhide popup (`Shift+H` from the inbox) — restore a blocked/reported
//! conversation by the other user's name
//! (`keybase chat api setstatus … unfiled`). The shared
//! [`draw_input_popup`]; Enter confirms, Esc cancels.

use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::view::widgets::draw_input_popup;

pub fn draw(frame: &mut Frame, app: &App) {
    draw_input_popup(
        frame,
        &app.theme,
        "Unhide a conversation",
        "username: ",
        &app.unhide_input,
        &[
            ("Enter", "restore"),
            ("Esc", "cancel"),
            ("", "(the other user's name)"),
        ],
    );
}
