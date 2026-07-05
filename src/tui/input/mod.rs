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
use crate::tui::screens::{Focus, Screen};

/// Top-level event dispatch. Called once per event loop tick by
/// [`crate::tui::run`].
pub fn handle_events(app: &mut App, event: Event) {
    app.reset_activity();
    match event {
        Event::Key(k) if k.kind == KeyEventKind::Press => handle_key(app, k),
        Event::Mouse(m) => mouse::handle(app, m),
        // Bracketed paste: the whole clipboard arrives as one event instead
        // of a key stream — without this, each embedded newline hit Enter
        // and sent the message mid-paste.
        Event::Paste(text) => handle_paste(app, &text),
        _ => {}
    }
}

/// Routes a bracketed paste to whichever text input currently owns typing,
/// mirroring the per-screen key routing. The compose keeps the newlines
/// (multi-line message); every single-line editor gets them flattened to
/// spaces; query editors run the same changed-query side effects their key
/// handlers apply. Screens with no active input drop the paste.
pub fn handle_paste(app: &mut App, text: &str) {
    // Normalize line endings; strip control chars a hostile paste could
    // carry (keep \n and \t — real content in code blocks).
    let clean: String = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    if clean.is_empty() {
        return;
    }
    let flat = || clean.replace('\n', " ");
    match app.screen {
        Screen::Login => {
            use crate::tui::app::LoginField;
            match app.login.focus {
                LoginField::Username => app.login.username.insert_str(&flat()),
                LoginField::Device => app.login.device.insert_str(&flat()),
                // The paperkey is the paste target on this screen.
                LoginField::PaperKey => app.login.paperkey.insert_str(&flat()),
                _ => {}
            }
        }
        Screen::React => {
            app.react.insert_str(&flat());
            app.react_selected = 0;
            app.rebuild_emoji_filter();
        }
        Screen::GiphySearch => {
            app.giphy_input.insert_str(&flat());
            app.giphy_results.clear();
            app.giphy_selected = 0;
        }
        Screen::ConvSearch => {
            app.conv_search.insert_str(&flat());
            app.conv_search_results.clear();
            app.conv_search_selected = 0;
        }
        Screen::SearchGlobal => app.search_global_input.insert_str(&flat()),
        Screen::QuickSwitcher => {
            app.switcher.query.insert_str(&flat());
            app.switcher.selected = 0;
        }
        Screen::CommandPalette => {
            app.palette.query.insert_str(&flat());
            app.palette.selected = 0;
        }
        Screen::NewConversation => app.new_conv.insert_str(&flat()),
        Screen::UnhideConversation => app.unhide_input.insert_str(&flat()),
        Screen::ChannelBrowser
            if app.channel_browser.creating || app.channel_browser.renaming.is_some() =>
        {
            app.channel_browser.new_name.insert_str(&flat());
        }
        Screen::Members if app.members.adding => app.members.add_input.insert_str(&flat()),
        Screen::Settings if app.settings_ui.editing.is_some() => {
            app.settings_ui.input.insert_str(&flat());
        }
        Screen::Inbox | Screen::Teams => match app.focus {
            Focus::Search => {
                app.search.insert_str(&flat());
                app.rebuild_filter();
            }
            // The compose is the one multi-line input: newlines survive.
            Focus::Chat if app.open_conv_id.is_some() && app.select.cursor.is_none() => {
                app.compose.insert_str(&clean);
            }
            _ => {}
        },
        _ => {}
    }
}

/// Routes a key to the open file picker and acts on its outcome: a
/// pick fires the attachment upload, a cancel just closes it.
pub(crate) fn file_picker_key(app: &mut App, key: KeyEvent) {
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
    // Errors are sticky on the strip until the user acts — the act is now.
    // (Running/Done keep their own lifecycles; worker-death re-raises via
    // `begin`, and its ⚠ badge is independent of the toast.)
    if matches!(app.action_state, crate::tui::action::ActionState::Error(_)) {
        app.set_action(crate::tui::action::ActionState::Idle);
    }
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
        && matches!(app.screen, Screen::Inbox | Screen::Teams)
    {
        crate::tui::flows::chat::open_quick_switcher(app);
        return;
    }
    // Ctrl+N — next unread conversation (the triage key); Ctrl+O — back to
    // the previous conversation (vim's alternate buffer). Global like the
    // switcher, so they work mid-compose. They *load* a conversation, so
    // the busy guard below still applies (begin() refuses while in flight).
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(app.screen, Screen::Inbox | Screen::Teams)
    {
        match key.code {
            KeyCode::Char('n') => {
                crate::tui::flows::chat::open_next_unread(app);
                return;
            }
            KeyCode::Char('o') => {
                crate::tui::flows::chat::open_previous_conversation(app);
                return;
            }
            _ => {}
        }
    }
    // Ctrl+P toggles the command palette (from a base screen) — the sibling of
    // the Ctrl+K switcher, for actions instead of conversations. Pure
    // navigation, allowed even while busy.
    if matches!(key.code, KeyCode::Char('p')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        match app.screen {
            Screen::CommandPalette => flows::palette::close_command_palette(app),
            Screen::Inbox | Screen::Teams => flows::palette::open_command_palette(app),
            _ => {}
        }
        return;
    }
    // While a worker request is in flight, swallow every key but Esc so
    // a second `request_*` can't overwrite `in_flight` / queue a stray
    // `WorkerRequest` (the `busy_blocks` gate). Esc still
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
        // Pre-status: the only meaningful input is retrying a failed boot
        // (`keybase status` timed out / service down) — otherwise the
        // splash was a dead end that only Ctrl+C could leave.
        Screen::Splash => {
            if matches!(key.code, KeyCode::Char('r') | KeyCode::Enter)
                && !app.is_busy()
                && matches!(app.action_state, crate::tui::action::ActionState::Error(_))
            {
                flows::auth::request_status(app);
            }
        }
        Screen::Login => login::handle(app, key),
        Screen::Inbox => inbox::handle(app, key),
        Screen::Teams => teams::handle(app, key),
        Screen::Help => handle_help(app, key),
        Screen::Settings => settings::handle(app, key),
        Screen::ConfirmLogout => common::run_confirm(
            app,
            key,
            |a| &mut a.logout_yes,
            |a| {
                // Don't switch to Login optimistically: return to the
                // inbox and let `handle_logout_response` move to Login
                // only on success. A failed logout (still signed in)
                // must not strand the user on the Login screen.
                a.screen = Screen::Inbox;
                flows::auth::request_logout(a);
            },
            |a| a.screen = Screen::Inbox,
        ),
        Screen::ConfirmConvAction => common::run_confirm(
            app,
            key,
            |a| &mut a.conv_action_yes,
            flows::chat::confirm_conv_action,
            flows::chat::cancel_conv_action,
        ),
        Screen::NewConversation => popups::new_conversation(app, key),
        Screen::UnhideConversation => popups::unhide_conversation(app, key),
        Screen::ChannelBrowser => popups::channel_browser(app, key),
        Screen::Members => popups::members(app, key),
        Screen::SearchGlobal => popups::search_global(app, key),
        Screen::ConfirmDeleteMessage => popups::confirm_delete_message(app, key),
        Screen::React => popups::react(app, key),
        Screen::QuickSwitcher => popups::quick_switcher(app, key),
        Screen::CommandPalette => popups::command_palette(app, key),
        Screen::ConvSearch => popups::conv_search(app, key),
        Screen::GiphySearch => popups::giphy_search(app, key),
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
        KeyCode::Char('d') | KeyCode::Char('D')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.help_scroll = app.help_scroll.saturating_add(PAGE);
        }
        KeyCode::Char('u') | KeyCode::Char('U')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.help_scroll = app.help_scroll.saturating_sub(PAGE);
        }
        KeyCode::Home | KeyCode::Char('g') => app.help_scroll = 0,
        KeyCode::End | KeyCode::Char('G') => app.help_scroll = u16::MAX,
        _ => {}
    }
}
