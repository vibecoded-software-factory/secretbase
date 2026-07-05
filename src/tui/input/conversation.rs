//! Input handling for the conversation detail screen.
//!
//! Two interaction modes share the screen:
//!
//! * **Compose** (default) — every printable key extends the draft;
//!   vertical arrows scroll messages; Enter sends.
//! * **Select** — entered via `Alt+V` or via a message-action
//!   shortcut. A cursor highlights one message; `e`/`p`/`r`/`+`
//!   edit / pin / reply / react / delete (`x`, confirm-gated); arrows move
//!   the cursor; `i` / Enter / Esc return to Compose mode.

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
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        // ── lifecycle ───────────────────────────────────────────────────
        // With the @-mention popup open, Esc dismisses *the popup* (for the
        // current token) — it must not fall through to the conversation
        // escape chain while a completion is on screen.
        KeyCode::Esc if app.mention_popup_active() => app.dismiss_mention_popup(),
        KeyCode::Esc => chat::escape_conversation(app),
        // Alt+Enter — or Shift+Enter, the Discord/Slack reflex, delivered
        // distinctly on terminals with the kitty keyboard protocol —
        // inserts a newline (multi-line message); plain Enter sends.
        KeyCode::Enter if alt || shift => app.compose.insert('\n'),
        // Enter with the mention popup open accepts the highlighted
        // suggestion (the Discord/Slack contract) — sending mid-completion
        // was the accident, not the intent. Esc first if you really meant
        // to send an unfinished @token.
        KeyCode::Enter if app.mention_popup_active() => {
            let matches = app.mention_matches();
            let sel = app.mention_selected.min(matches.len().saturating_sub(1));
            if let Some(u) = matches.get(sel).cloned() {
                chat::accept_mention(app, &u);
            }
        }
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
        // With an empty compose, Home/End act on the **history** (jump to the
        // oldest loaded / the latest message) — this is what makes the
        // "▼ N new · End" cue true in the default mode. With draft text they
        // stay editor keys (cursor to start/end of the line), so nothing is
        // lost for typing.
        KeyCode::End if app.compose.is_empty() => app.messages_scroll = 0,
        KeyCode::Home if app.compose.is_empty() => {
            app.messages_scroll = app.messages_max_back;
            maybe_queue_older(app);
        }

        // ── Ctrl shortcuts (don't collide with printable text) ─────────
        KeyCode::F(5) => chat::request_load_messages(app),
        KeyCode::Char('r') if ctrl => chat::request_load_messages(app),
        KeyCode::Char('y') if ctrl => chat::do_copy_conversation_label(app),
        KeyCode::Char('f') if ctrl => chat::open_conv_search(app),

        // ── Alt shortcuts ──────────────────────────────────────────────
        // Per-message actions (edit / delete / react / pin / reply) live in
        // Select mode (Alt+V) — Compose stays for composing only.
        KeyCode::Char('u') | KeyCode::Char('U') if alt => chat::request_unpin_conversation(app),
        // Local hide of the pin banner — the quiet sibling of Alt+U.
        KeyCode::Char('h') | KeyCode::Char('H') if alt => chat::dismiss_pin_banner(app),
        // Slack's up-to-edit fast path: jump straight to editing your most
        // recent own message.
        KeyCode::Char('e') | KeyCode::Char('E') if alt => chat::edit_last_own_message(app),
        KeyCode::Char('v') | KeyCode::Char('V') if alt => chat::enter_select_mode(app),
        KeyCode::Char('r') | KeyCode::Char('R') if alt => chat::request_resend_message(app),
        KeyCode::Char('a') | KeyCode::Char('A') if alt => open_attach_picker(app),
        // Emoji into the draft — the picker in insert mode (also the compose
        // bar's clickable chip).
        KeyCode::Char('i') | KeyCode::Char('I') if alt => chat::open_emoji_for_compose(app),
        // GIF search (giphy, user API key) — also a compose-bar chip.
        KeyCode::Char('g') | KeyCode::Char('G') if alt => chat::open_giphy_search(app),
        // Jump to the `new messages` divider (catch-up entry point).
        KeyCode::Char('n') | KeyCode::Char('N') if alt => chat::jump_to_new_messages(app),
        // Members of the open conversation (team channels only).
        KeyCode::Char('p') | KeyCode::Char('P') if alt => chat::open_members_from_conversation(app),

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
    // $HOME, not the process cwd: over SSH the cwd is wherever the binary
    // was launched — rarely where the user's files live. (The download
    // picker's Downloads default has the same spirit.)
    let start = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    app.picker_action = crate::tui::app::PickerAction::Upload;
    app.file_picker = Some(crate::tui::file_picker::FilePicker::new(&start));
}

fn handle_select(app: &mut App, key: KeyEvent) {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        // vim exits: `i` / `a` return to insert — the compose.
        KeyCode::Char('i') | KeyCode::Char('a') => chat::leave_select_mode(app),
        // Enter activates the cursor row (list semantics): a reply jumps to
        // the message it quotes; anything else exits to Compose as before.
        KeyCode::Enter => chat::select_activate(app),
        // Esc is layered like the cmdlog: clear the selection first; with
        // nothing selected it closes the conversation (normal → out), never
        // silently dropping marks *and* the conversation in one press.
        KeyCode::Esc => {
            if !app.msg_marks.is_empty() || app.select_anchor.is_some() {
                app.msg_marks.clear();
                app.select_anchor = None;
            } else {
                chat::close_conversation(app);
            }
        }
        // `:` = the command line (the palette), vim-style.
        KeyCode::Char(':') => crate::tui::flows::palette::open_command_palette(app),
        // vim buffer search: `/` in Select opens the in-conversation search;
        // `n` / `N` cycle the retained hits (wrapping) without reopening it.
        KeyCode::Char('/') => chat::open_conv_search(app),
        // Alt+N must precede the bare `n`/`N` arms: an unguarded
        // `Char('n')` pattern matches regardless of modifiers.
        KeyCode::Char('n') | KeyCode::Char('N') if alt => chat::jump_to_new_messages(app),
        KeyCode::Char('n') => chat::conv_search_cycle(app, 1),
        KeyCode::Char('N') => chat::conv_search_cycle(app, -1),
        // Shade a contiguous range with Alt+Shift+↑/↓ or Alt+Shift+K/J — kept
        // consistent because many terminals only deliver Shift+arrows with Alt.
        KeyCode::Char('K') if alt => chat::select_extend(app, -1),
        KeyCode::Char('J') if alt => chat::select_extend(app, 1),
        // The banner is visible in Select mode too — same hide as Compose.
        KeyCode::Char('h') | KeyCode::Char('H') if alt => chat::dismiss_pin_banner(app),
        // Mention motions: the messages you owe a response to.
        KeyCode::Char('[') => chat::select_jump_mention(app, -1),
        KeyCode::Char(']') => chat::select_jump_mention(app, 1),
        KeyCode::Up if shift => chat::select_extend(app, -1),
        KeyCode::Down if shift => chat::select_extend(app, 1),
        KeyCode::Up | KeyCode::Char('k') => chat::select_move_up(app),
        KeyCode::Down | KeyCode::Char('j') => chat::select_move_down(app),
        // vim half-page over the message cursor (PgUp/PgDn aliases).
        KeyCode::Char('u') | KeyCode::Char('U')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if let Some(i) = app.selected_msg_idx {
                app.selected_msg_idx = Some(i.saturating_sub(crate::tui::app::PAGE_STEP));
                chat::select_resync_anchor_marks(app);
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            let max = app.messages.len().saturating_sub(1);
            if let Some(i) = app.selected_msg_idx {
                app.selected_msg_idx = Some((i + crate::tui::app::PAGE_STEP).min(max));
                chat::select_resync_anchor_marks(app);
            }
        }
        // `v` anchors a visual range; every motion then extends it (vim).
        KeyCode::Char('v') => chat::select_toggle_anchor(app),
        // `{` / `}` — previous / next speaker run (paragraph motion).
        KeyCode::Char('{') => chat::select_jump_run(app, -1),
        KeyCode::Char('}') => chat::select_jump_run(app, 1),
        KeyCode::PageUp => {
            if let Some(i) = app.selected_msg_idx {
                app.selected_msg_idx = Some(i.saturating_sub(crate::tui::app::PAGE_STEP));
                chat::select_resync_anchor_marks(app);
            }
        }
        KeyCode::PageDown => {
            let max = app.messages.len().saturating_sub(1);
            if let Some(i) = app.selected_msg_idx {
                app.selected_msg_idx = Some((i + crate::tui::app::PAGE_STEP).min(max));
                chat::select_resync_anchor_marks(app);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.selected_msg_idx = Some(0);
            chat::select_resync_anchor_marks(app);
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.selected_msg_idx = Some(app.messages.len().saturating_sub(1));
            chat::select_resync_anchor_marks(app);
        }
        // Multi-select + copy (reduced action set).
        KeyCode::Char(' ') => chat::msg_toggle_mark(app),
        KeyCode::Char('y') => chat::do_copy_messages(app, true), // author + time + body
        KeyCode::Char('c') => chat::do_copy_content(app),        // image, or body text
        KeyCode::Char('o') => chat::do_open_url(app),            // open first link
        // `u` copies the URL — `l` stays motion-vocabulary everywhere (it was
        // the one place the app overloaded its own universal `l` = open/right).
        KeyCode::Char('u') => chat::do_copy_url(app),
        KeyCode::Char('+') => chat::open_react_for_selected(app), // react (emoji)
        // Delete ALL marked (or the cursor) — bare `x` like every other
        // select-mode verb; the navigable confirm (default = cancel) is the
        // guard. `X` stays as a muscle-memory alias.
        KeyCode::Char('x') | KeyCode::Char('X') => chat::open_delete_for_selected(app),
        // Single-message actions (operate on the cursor message).
        KeyCode::Char('e') => chat::open_edit_for_selected(app),
        KeyCode::Char('p') => chat::request_pin_selected_message(app),
        KeyCode::Char('r') => chat::start_reply_for_selected(app),
        KeyCode::Char('s') => chat::open_download_for_selected(app),
        _ => {}
    }
}
