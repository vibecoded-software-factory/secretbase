//! Clipboard-helper flows.
//!
//! Centralised here so the chat / future-screen flows don't all
//! duplicate the `write_with_clear` boilerplate.

use crate::tui::action::ActionState;
use crate::tui::app::App;

/// Writes `text` to the clipboard with the user's configured auto-clear
/// timeout, surfacing success / failure through the feedback strip.
///
/// `label_for_log` is the human-readable description that lands in the
/// command-log panel (e.g. "Conversation label", "Username").
pub fn write_to_clipboard(app: &mut App, text: &str, label_for_log: &str) {
    let secs = app.settings_cache.clipboard_clear_secs;
    match app.clipboard.write_with_clear(text, secs) {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("{label_for_log} copied")));
            app.push_cmd(format!("clipboard {label_for_log}"), true, "ok");
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.clone()));
            app.push_cmd(format!("clipboard {label_for_log}"), false, e);
        }
    }
}
