//! Input handling for the conversation detail screen.
//!
//! Two interaction modes share the screen:
//!
//! * **Compose** (default) — every printable key extends the draft;
//!   vertical arrows scroll messages; Enter sends.
//! * **Select** — entered via `Alt+V` or via a message-action
//!   shortcut. A cursor highlights one message; `e`/`d`/`:`/`p`
//!   trigger edit / delete / react / pin; arrows move the cursor;
//!   `i` / Enter / Esc return to Compose mode.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows::chat;
use crate::tui::input::common;

/// Queues a pagination fetch when the user has scrolled past the top
/// of the currently-loaded history and more is available.
pub(crate) fn maybe_queue_older(app: &mut App) {
    if app.messages_scroll <= app.messages_max_back {
        return;
    }
    if app.messages_next.is_none() || app.messages_loading_older {
        app.messages_scroll = app.messages_max_back;
        return;
    }
    app.messages_scroll = app.messages_max_back;
    app.messages_loading_older = true;
    chat::request_load_older_messages(app);
}

pub fn handle(app: &mut App, key: KeyEvent) {
    if app.selected_msg_idx.is_some() {
        return handle_select(app, key);
    }
    handle_compose(app, key);
}

fn handle_compose(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        // ── lifecycle ───────────────────────────────────────────────────
        KeyCode::Esc => {
            if app.edit_target_id.is_some() {
                chat::cancel_edit(app);
            } else if app.compose.is_empty() {
                chat::close_conversation(app);
            } else {
                app.compose_clear();
            }
        }
        KeyCode::Enter => submit_compose(app),

        // ── history viewport scroll ────────────────────────────────────
        KeyCode::Up => {
            app.messages_scroll = app.messages_scroll.saturating_add(1);
            maybe_queue_older(app);
        }
        KeyCode::Down => app.messages_scroll = app.messages_scroll.saturating_sub(1),
        KeyCode::PageUp => {
            app.messages_scroll = app.messages_scroll.saturating_add(10);
            maybe_queue_older(app);
        }
        KeyCode::PageDown => app.messages_scroll = app.messages_scroll.saturating_sub(10),

        // ── Ctrl shortcuts (don't collide with printable text) ─────────
        KeyCode::F(5) => chat::request_load_messages(app),
        KeyCode::Char('r') if ctrl => chat::request_load_messages(app),
        KeyCode::Char('y') if ctrl => chat::do_copy_conversation_label(app),

        // ── Alt shortcuts ──────────────────────────────────────────────
        // Per-message actions (edit / delete / react / pin / reply) live in
        // Select mode (Alt+V) — Compose stays for composing only.
        KeyCode::Char('u') | KeyCode::Char('U') if alt => chat::request_unpin_conversation(app),
        KeyCode::Char('v') | KeyCode::Char('V') if alt => chat::enter_select_mode(app),
        KeyCode::Char('r') | KeyCode::Char('R') if alt => chat::request_resend_message(app),
        KeyCode::Char('a') | KeyCode::Char('A') if alt => open_attach_picker(app),

        // ── text input (cursor moves + edits) ──────────────────────────
        _ => {
            common::route_line_editor(&mut app.compose, key);
        }
    }
}

/// Sends the compose buffer (or saves the edit). Shared by the `Enter` key,
/// the Send button (keyboard activate) and the Send button click.
pub(crate) fn submit_compose(app: &mut App) {
    if app.compose.text().trim().is_empty() {
        app.set_action(crate::tui::action::ActionState::Error(
            "Message is empty".into(),
        ));
        return;
    }
    if app.edit_target_id.is_some() {
        chat::request_save_edit(app);
    } else {
        chat::request_send_message(app);
    }
}

/// Opens the file picker to attach a file. Shared by `Alt+A`, the Attach
/// button (keyboard activate) and the Attach button click.
pub(crate) fn open_attach_picker(app: &mut App) {
    let start = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    app.picker_action = crate::tui::app::PickerAction::Upload;
    app.file_picker = Some(crate::tui::file_picker::FilePicker::new(&start));
}

fn handle_select(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('i') | KeyCode::Enter => chat::leave_select_mode(app),
        KeyCode::Up | KeyCode::Char('k') => chat::select_move_up(app),
        KeyCode::Down | KeyCode::Char('j') => chat::select_move_down(app),
        KeyCode::Home | KeyCode::Char('g') => app.selected_msg_idx = Some(0),
        KeyCode::End | KeyCode::Char('G') => {
            app.selected_msg_idx = Some(app.messages.len().saturating_sub(1));
        }
        KeyCode::Char('e') => chat::open_edit_for_selected(app),
        KeyCode::Char('d') => chat::open_delete_for_selected(app),
        KeyCode::Char(':') => chat::open_react_for_selected(app),
        KeyCode::Char('p') => chat::request_pin_selected_message(app),
        KeyCode::Char('r') => chat::start_reply_for_selected(app),
        KeyCode::Char('s') => chat::open_download_for_selected(app),
        _ => {}
    }
}
