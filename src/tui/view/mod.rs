//! Per-screen Ratatui renderers.
//!
//! `draw` is the single entry point called from the run loop. It draws
//! the active base screen and overlays any popup on top, via the
//! `split_main` stack + `titled_block` layout system.

pub mod action;
pub mod channels;
pub mod conv_search;
pub mod conversation;
pub mod help;
pub mod inbox;
pub mod login;
pub mod logo;
pub mod members;
pub mod new_conversation;
pub mod palette;
pub mod popups;
pub mod search_global;
pub mod settings;
pub mod splash;
pub mod starfield;
pub mod teams;
pub mod unhide;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::App;
use crate::tui::screens::Screen;

/// Minimum terminal size before the TUI falls back to a "resize me"
/// notice. identity(3) + search(3) + body(5) + cmdlog(6) + status(1).
pub const MIN_W: u16 = 70;
pub const MIN_H: u16 = 18;

/// Per-frame entry point.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // Reset hit-test rects every frame; per-screen draws populate the
    // slots they care about. Stamp the size so the input layer can
    // drop clicks whose coordinates predate a resize.
    app.mouse_areas.reset(area.width, area.height);

    if area.width < MIN_W || area.height < MIN_H {
        draw_too_small(frame, area, &app.theme);
        return;
    }

    // Pick the base (non-overlay) screen to render underneath any popup.
    let base = match app.screen {
        Screen::ConfirmLogout
        | Screen::ConfirmConvAction
        | Screen::NewConversation
        | Screen::UnhideConversation
        | Screen::ChannelBrowser
        | Screen::Members
        | Screen::SearchGlobal
        | Screen::ConfirmDeleteMessage
        | Screen::ConvSearch
        | Screen::React => Screen::Inbox,
        // The quick switcher / command palette float over wherever opened.
        Screen::QuickSwitcher => app.switcher_from,
        Screen::CommandPalette => app.palette_from,
        // Help is scoped to (and renders over) the screen it was opened
        // from — use `help_from`, not an open-conversation heuristic, so
        // e.g. Teams isn't drawn as Inbox underneath.
        Screen::Help => app.help_from,
        Screen::Settings => app.settings_from,
        other => other,
    };
    draw_screen(frame, app, base);

    match app.screen {
        Screen::Help => help::draw(frame, app),
        Screen::Settings => settings::draw_popup(frame, app),
        Screen::ConfirmLogout => widgets::draw_confirm_popup(
            frame,
            frame.area(),
            &app.theme,
            " Log out of Keybase? ",
            vec![Line::from(Span::styled(
                "This ends your TUI session (runs `keybase logout`).",
                Style::default().fg(app.theme.dim),
            ))],
            app.logout_yes,
        ),
        Screen::ConfirmDeleteMessage => {
            let n = crate::tui::flows::chat::delete_selection_count(app);
            let (title, label) = if n > 1 {
                (
                    format!(" Delete {n} messages? "),
                    "This can't be undone.".to_string(),
                )
            } else {
                let one = app
                    .selected_msg_idx
                    .and_then(|i| app.messages.get(i))
                    .map(|m| format!("msg #{} by {}", m.id, m.sender))
                    .unwrap_or_else(|| "(none)".to_string());
                (" Delete this message? ".to_string(), one)
            };
            widgets::draw_confirm_popup(
                frame,
                frame.area(),
                &app.theme,
                &title,
                vec![Line::from(Span::styled(
                    label,
                    Style::default().fg(app.theme.dim),
                ))],
                app.delete_msg_yes,
            );
        }
        Screen::ConfirmConvAction => {
            if let Some(action) = app.pending_conv_action {
                let label = app
                    .selected_conversation()
                    .and_then(|c| {
                        app.conversations
                            .iter()
                            .position(|x| x.id == c.id)
                            .and_then(|i| app.conversations_lowered.get(i))
                    })
                    .map(|l| l.display_label.clone())
                    .unwrap_or_else(|| "(conversation)".to_string());
                widgets::draw_confirm_popup(
                    frame,
                    frame.area(),
                    &app.theme,
                    action.title(),
                    vec![
                        Line::from(Span::styled(
                            label,
                            Style::default()
                                .fg(app.theme.foreground)
                                .add_modifier(ratatui::style::Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            action.note(),
                            Style::default().fg(app.theme.dim),
                        )),
                    ],
                    app.conv_action_yes,
                );
            }
        }
        Screen::NewConversation => new_conversation::draw(frame, app),
        Screen::UnhideConversation => unhide::draw(frame, app),
        Screen::ChannelBrowser => channels::draw(frame, app),
        Screen::Members => members::draw(frame, app),
        Screen::SearchGlobal => search_global::draw(frame, app),
        Screen::React => popups::react_input(frame, app),
        Screen::QuickSwitcher => popups::quick_switcher(frame, app),
        Screen::CommandPalette => palette::draw(frame, app),
        Screen::ConvSearch => conv_search::draw(frame, app),
        _ => {}
    }

    // The embedded file picker sits above everything else as a modal.
    if app.file_picker.is_some() {
        let t = app.theme.clone();
        // Scrollable modal — shares the standard modal geometry with the quick
        // switcher / global search (`widgets::MODAL_*`).
        let area = widgets::center_rect(
            widgets::MODAL_WIDTH_PCT,
            widgets::MODAL_HEIGHT,
            frame.area(),
        );
        if let Some(picker) = app.file_picker.as_mut() {
            picker.render(frame, area, &t);
        }
    }
}

/// Renders a single non-overlay base screen.
fn draw_screen(frame: &mut Frame, app: &mut App, screen: Screen) {
    match screen {
        Screen::Splash => splash::draw(frame, app),
        Screen::Login => login::draw(frame, app),
        Screen::Inbox => inbox::draw(frame, app),
        Screen::Teams => teams::draw(frame, app),
        // Overlays are never a base screen — fall back to the inbox.
        _ => inbox::draw(frame, app),
    }
}

/// Renders the centred "terminal too small" notice — the only thing we
/// draw below [`MIN_W`]/[`MIN_H`]: an accent-error title, a dim line
/// stating the required and current size, and a `Ctrl+C to quit` hint,
/// vertically centred.
fn draw_too_small(frame: &mut Frame, area: Rect, theme: &crate::tui::theme::Theme) {
    let header = Line::from(Span::styled(
        "Terminal too small",
        Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
    ))
    .alignment(ratatui::layout::Alignment::Center);
    let detail = Line::from(Span::styled(
        format!(
            "Resize to at least {}×{} (currently {}×{})",
            MIN_W, MIN_H, area.width, area.height,
        ),
        Style::default().fg(theme.dim),
    ))
    .alignment(ratatui::layout::Alignment::Center);
    let hint = Line::from(Span::styled(
        "Ctrl+C to quit",
        Style::default().fg(theme.dim),
    ))
    .alignment(ratatui::layout::Alignment::Center);
    // Vertically centre: pad the top so the header lands mid-screen even
    // on a very short terminal.
    let blanks = (area.height as usize).saturating_sub(3) / 2;
    let mut lines: Vec<Line> = (0..blanks).map(|_| Line::from("")).collect();
    lines.push(header);
    lines.push(detail);
    lines.push(hint);
    frame.render_widget(Paragraph::new(lines), area);
}

/// Splits a vertical area into the standard signed-in stack:
/// `identity` · `header` (3) · `body` (fills) · `cmdlog`
/// ([`widgets::cmdlog_height`], responsive) · `status` (1).
/// `identity_content_rows` comes from [`widgets::identity_content_rows`];
/// +2 for the block borders.
pub fn split_main(area: Rect, identity_content_rows: u16) -> [Rect; 5] {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(identity_content_rows + 2),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(widgets::cmdlog_height(area.height)),
            Constraint::Length(1),
        ])
        .split(area);
    [chunks[0], chunks[1], chunks[2], chunks[3], chunks[4]]
}

/// Common bordered block with a stylised title — accent + bold when
/// focused, else the inactive tint.
pub fn titled_block<'a>(title: &'a str, focused: bool, app: &'a App) -> Block<'a> {
    let style = if focused {
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.theme.inactive)
    };
    Block::default()
        .borders(Borders::ALL)
        // Rounded corners on every section panel — the single place that decides
        // it, so the whole app's chrome stays consistently rounded.
        .border_type(BorderType::Rounded)
        .title(Span::styled(title.to_string(), style))
        .border_style(style)
}

/// A panel that **can't be focused right now** (e.g. the chat and its in-chat
/// search when no conversation is open — they're skipped by Tab and their go-to
/// keys are gated). Rendered with the `muted` (darker-than-`inactive`) border
/// so it clearly reads as *unreachable*, distinct from an available-but-
/// unfocused panel. Same rounded chrome as [`titled_block`].
pub fn disabled_block<'a>(title: &'a str, app: &'a App) -> Block<'a> {
    let style = Style::default().fg(app.theme.muted);
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(title.to_string(), style))
        .border_style(style)
}
