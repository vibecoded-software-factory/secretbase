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
    if app.conv_search_active {
        return handle_conv_search(app, key);
    }
    if app.selected_msg_idx.is_some() {
        return handle_select(app, key);
    }
    handle_compose(app, key);
}

/// In-conversation search box (Ctrl+F): type a query, Enter runs
/// `searchregexp`; once results exist, ↑/↓ pick one and Enter jumps to it.
fn handle_conv_search(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => chat::close_conv_search(app),
        KeyCode::Enter => {
            if app.conv_search_results.is_empty() {
                chat::request_conv_search(app);
            } else {
                chat::conv_search_jump_selected(app);
            }
        }
        KeyCode::F(5) => chat::request_conv_search(app),
        KeyCode::Up => {
            let len = app.conv_search_results.len();
            app.conv_search_selected = common::clamp_move(app.conv_search_selected, -1, len);
        }
        KeyCode::Down => {
            let len = app.conv_search_results.len();
            app.conv_search_selected = common::clamp_move(app.conv_search_selected, 1, len);
        }
        _ => {
            let before = app.conv_search.text().to_string();
            common::route_line_editor(&mut app.conv_search, key);
            // Editing the query invalidates the old results so the next
            // Enter re-runs the search instead of jumping to a stale hit.
            if app.conv_search.text() != before {
                app.conv_search_results.clear();
                app.conv_search_selected = 0;
            }
        }
    }
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
        // Alt+Enter inserts a newline (multi-line message); plain Enter sends.
        KeyCode::Enter if alt => app.compose.insert('\n'),
        KeyCode::Enter => submit_compose(app),

        // ── @-mention autocomplete (when its popup is open) ────────────
        // Tab accepts the highlighted suggestion; ↑/↓ pick.
        KeyCode::Tab if app.mention_popup_active() => {
            let matches = app.mention_matches();
            let sel = app.mention_selected.min(matches.len().saturating_sub(1));
            if let Some(u) = matches.get(sel).cloned() {
                chat::accept_mention(app, &u);
            }
        }

        // ── history viewport scroll (↑/↓ pick a suggestion while the
        // mention popup is open) ───────────────────────────────────────
        KeyCode::Up if app.mention_popup_active() => {
            app.mention_selected = app.mention_selected.saturating_sub(1);
        }
        KeyCode::Down if app.mention_popup_active() => {
            let n = app.mention_matches().len();
            app.mention_selected = (app.mention_selected + 1).min(n.saturating_sub(1));
        }
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
        KeyCode::Char('f') if ctrl => chat::open_conv_search(app),

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
            // A changed prefix re-filters the suggestions — restart at the top.
            app.mention_selected = 0;
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
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc | KeyCode::Char('i') | KeyCode::Enter => chat::leave_select_mode(app),
        // Shade a contiguous range with Alt+Shift+↑/↓ or Alt+Shift+K/J — kept
        // consistent because many terminals only deliver Shift+arrows with Alt.
        KeyCode::Char('K') if alt => chat::select_extend(app, -1),
        KeyCode::Char('J') if alt => chat::select_extend(app, 1),
        KeyCode::Up if shift => chat::select_extend(app, -1),
        KeyCode::Down if shift => chat::select_extend(app, 1),
        KeyCode::Up | KeyCode::Char('k') => chat::select_move_up(app),
        KeyCode::Down | KeyCode::Char('j') => chat::select_move_down(app),
        KeyCode::Home | KeyCode::Char('g') => app.selected_msg_idx = Some(0),
        KeyCode::End | KeyCode::Char('G') => {
            app.selected_msg_idx = Some(app.messages.len().saturating_sub(1));
        }
        // Multi-select + copy (reduced action set).
        KeyCode::Char(' ') => chat::msg_toggle_mark(app),
        KeyCode::Char('y') => chat::do_copy_messages(app, true), // author + time + body
        KeyCode::Char('c') => chat::do_copy_content(app),        // image, or body text
        KeyCode::Char('o') => chat::do_open_url(app),            // open first link
        KeyCode::Char('l') => chat::do_copy_url(app),            // copy first link
        KeyCode::Char('+') => chat::open_react_for_selected(app), // react (emoji)
        // Single-message actions (operate on the cursor message).
        KeyCode::Char('e') => chat::open_edit_for_selected(app),
        KeyCode::Char('d') => chat::open_delete_for_selected(app),
        KeyCode::Char('p') => chat::request_pin_selected_message(app),
        KeyCode::Char('r') => chat::start_reply_for_selected(app),
        KeyCode::Char('s') => chat::open_download_for_selected(app),
        _ => {}
    }
}
