//! Input dispatch.
//!
//! `handle_events` is the single entry point called from the run loop.
//! It routes the event to the appropriate per-screen handler. Mouse
//! events are translated to semantic targets via
//! [`crate::tui::mouse_areas::MouseAreas`].

pub mod common;
pub mod conversation;
pub mod inbox;
pub mod login;
pub mod mouse;
pub mod popups;
pub mod settings;
pub mod teams;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::tui::app::App;
use crate::tui::flows;
use crate::tui::screens::Screen;

/// Top-level event dispatch. Called once per event loop tick by
/// [`crate::tui::run`].
pub fn handle_events(app: &mut App, event: Event) {
    app.reset_activity();
    match event {
        Event::Key(k) if k.kind == KeyEventKind::Press => handle_key(app, k),
        Event::Mouse(m) => mouse::handle(app, m),
        _ => {}
    }
}

/// Routes a key to the open file picker and acts on its outcome: a
/// pick fires the attachment upload, a cancel just closes it.
fn file_picker_key(app: &mut App, key: KeyEvent) {
    use crate::tui::app::PickerAction;
    use crate::tui::file_picker::Outcome;
    let Some(picker) = app.file_picker.as_mut() else {
        return;
    };
    match picker.handle_key(key) {
        Outcome::Pending => {}
        Outcome::Cancelled => app.file_picker = None,
        Outcome::Selected(path) => {
            app.file_picker = None;
            match app.picker_action.clone() {
                PickerAction::Upload => {
                    crate::tui::flows::chat::request_upload_attachment(app, path)
                }
                PickerAction::Download {
                    message_id,
                    filename,
                } => crate::tui::flows::chat::request_download_to(app, message_id, path, filename),
            }
        }
    }
}

/// Top-level keyboard dispatch. Global shortcuts win; everything else
/// goes to the per-screen handler.
fn handle_key(app: &mut App, key: KeyEvent) {
    // Global shortcuts handled before screen-specific routing.
    if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }
    // The embedded file picker, when open, is a modal overlay that owns
    // every key (except the global Ctrl+C quit above).
    if app.file_picker.is_some() {
        file_picker_key(app, key);
        return;
    }
    // Ctrl+K opens the quick switcher from the inbox or an open
    // conversation — pure navigation, allowed even while busy.
    if matches!(key.code, KeyCode::Char('k'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(app.screen, Screen::Inbox)
    {
        crate::tui::flows::chat::open_quick_switcher(app);
        return;
    }
    // While a worker request is in flight, swallow every key but Esc so
    // a second `request_*` can't overwrite `in_flight` / queue a stray
    // `WorkerRequest` (mirrors jewel's `busy_blocks` gate). Esc still
    // passes so the user can always back out.
    if common::busy_blocks(app.is_busy(), &key) {
        return;
    }

    if matches!(key.code, KeyCode::F(1)) {
        // F1 from any screen opens (or closes) the context-aware help
        // overlay. Opening stamps the originating screen so the help is
        // scoped to it and closing returns there.
        if app.screen == Screen::Help {
            app.screen = app.help_from;
        } else {
            app.help_from = app.screen;
            app.help_scroll = 0;
            app.screen = Screen::Help;
        }
        return;
    }

    if matches!(key.code, KeyCode::F(10)) {
        // F10 toggles the Settings overlay. It opens only from the base
        // screens (never stacked on another overlay) and closes when already
        // open (changes apply+persist live, so closing keeps them).
        if app.screen == Screen::Settings {
            app.close_settings();
        } else if matches!(app.screen, Screen::Login | Screen::Inbox | Screen::Teams) {
            app.open_settings();
        }
        return;
    }

    match app.screen {
        Screen::Splash => {} // pre-status, no input accepted
        Screen::Login => login::handle(app, key),
        Screen::Inbox => inbox::handle(app, key),
        Screen::Teams => teams::handle(app, key),
        Screen::Help => handle_help(app, key),
        Screen::Settings => settings::handle(app, key),
        Screen::ConfirmLogout => {
            use common::ConfirmInput;
            let commit = |app: &mut App| {
                // Don't switch to Login optimistically: return to the
                // inbox and let `handle_logout_response` move to Login
                // only on success. A failed logout (still signed in)
                // must not strand the user on the Login screen.
                app.screen = Screen::Inbox;
                flows::auth::request_logout(app);
            };
            match common::confirm_key(key) {
                ConfirmInput::Commit => commit(app),
                ConfirmInput::Cancel => app.screen = Screen::Inbox,
                ConfirmInput::Activate => {
                    if app.logout_yes {
                        commit(app);
                    } else {
                        app.screen = Screen::Inbox;
                    }
                }
                ConfirmInput::Yes => app.logout_yes = true,
                ConfirmInput::No => app.logout_yes = false,
                ConfirmInput::Toggle => app.logout_yes = !app.logout_yes,
                ConfirmInput::Ignore => {}
            }
        }
        Screen::ConfirmConvAction => {
            use common::ConfirmInput;
            match common::confirm_key(key) {
                ConfirmInput::Commit => flows::chat::confirm_conv_action(app),
                ConfirmInput::Cancel => flows::chat::cancel_conv_action(app),
                ConfirmInput::Activate => {
                    if app.conv_action_yes {
                        flows::chat::confirm_conv_action(app);
                    } else {
                        flows::chat::cancel_conv_action(app);
                    }
                }
                ConfirmInput::Yes => app.conv_action_yes = true,
                ConfirmInput::No => app.conv_action_yes = false,
                ConfirmInput::Toggle => app.conv_action_yes = !app.conv_action_yes,
                ConfirmInput::Ignore => {}
            }
        }
        Screen::NewConversation => popups::new_conversation(app, key),
        Screen::UnhideConversation => popups::unhide_conversation(app, key),
        Screen::ChannelBrowser => popups::channel_browser(app, key),
        Screen::Members => popups::members(app, key),
        Screen::SearchGlobal => popups::search_global(app, key),
        Screen::ConfirmDeleteMessage => popups::confirm_delete_message(app, key),
        Screen::React => popups::react(app, key),
        Screen::QuickSwitcher => popups::quick_switcher(app, key),
    }
}

/// Scrolls the help overlay or closes it. The renderer clamps
/// `help_scroll` against the real overflow, so `End` can use `u16::MAX`.
fn handle_help(app: &mut App, key: KeyEvent) {
    const PAGE: u16 = 10;
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.screen = app.help_from,
        KeyCode::Down | KeyCode::Char('j') => {
            app.help_scroll = app.help_scroll.saturating_add(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.help_scroll = app.help_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(PAGE),
        KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(PAGE),
        KeyCode::Home | KeyCode::Char('g') => app.help_scroll = 0,
        KeyCode::End | KeyCode::Char('G') => app.help_scroll = u16::MAX,
        _ => {}
    }
}
