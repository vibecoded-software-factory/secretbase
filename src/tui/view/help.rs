//! Help popup — a scrollable overlay scoped to the screen it was opened
//! from (`App::help_from`) — context-aware help.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::tui::app::App;
use crate::tui::screens::Screen;
use crate::tui::theme::Theme;
use crate::tui::view::widgets::{center_rect, help_line};

/// Renders the help overlay. Takes `&mut App` because the renderer owns
/// the viewport: it clamps `app.help_scroll` against the real overflow so
/// the input handler can bump the offset without its own bookkeeping.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let t = app.theme.clone();
    // On the unified Home, the help follows the focused pane: the chat pane
    // shows the conversation shortcuts, everything else the inbox ones.
    let from = app.help_from;
    let chat = from == Screen::Inbox && app.focus == crate::tui::screens::Focus::Chat;
    let lines = build_lines(from, chat, &t);

    let h = (frame.area().height.saturating_mul(82) / 100).max(8);
    // `center_rect` width is a percentage, so the popup already scales with
    // the terminal (72% wide, ~82% tall — see `h`).
    let popup = center_rect(72, h, frame.area());
    frame.render_widget(Clear, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    // Clamp the scroll offset against the actual content overflow.
    let content_h = lines.len() as u16;
    let max_y = content_h.saturating_sub(inner.height);
    app.help_scroll = app.help_scroll.min(max_y);
    let scroll_y = app.help_scroll;

    let block = Block::default()
        .title(Span::styled(
            " Help ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent));
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), inner);

    draw_scroll_indicators(frame, popup, scroll_y, max_y, &t);
}

fn section(title: &str, t: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(t.foreground)
            .add_modifier(Modifier::BOLD),
    ))
}

fn build_lines(from: Screen, chat: bool, t: &Theme) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if chat {
        lines.push(section("Conversation — Compose", t));
        for (k, d) in [
            ("(typing)", "extend draft"),
            ("Enter", "send / save edit"),
            ("Alt+Enter / Shift+Enter", "new line (multi-line message)"),
            ("@… then Tab", "mention autocomplete (↑/↓ pick)"),
            (
                "*b* _i_ ~s~ `c`",
                "markdown: bold/italic/strike/code (> quote, ``` code)",
            ),
            ("↑/↓ PgUp/PgDn", "scroll history"),
            ("←/→ Home/End", "compose cursor"),
            ("F5 / Ctrl+R", "refresh"),
            ("Ctrl+Y", "copy label"),
            ("Ctrl+F", "search this conversation (jump to a match)"),
            ("Ctrl+K", "quick switcher — jump to a conversation"),
            ("Ctrl+P", "command palette — run any action"),
            ("mouse", "click a message to select, scroll to page history"),
            ("Alt+V", "select mode — act on a message"),
            ("Alt+I", "insert an emoji into the draft (picker)"),
            ("Alt+U", "unpin channel"),
            (
                "Alt+H",
                "hide the pin banner (local only — unpins for no one)",
            ),
            ("Alt+R", "resend failed message"),
            ("Alt+A", "attach a file (opens the file picker)"),
            ("Alt+P", "members (team channels) — add / remove"),
            ("Esc", "cancel / back"),
        ] {
            lines.push(help_line(k, d, t));
        }
        lines.push(Line::raw(""));
        lines.push(section("File picker", t));
        for (k, d) in [
            ("↑/↓ k/j", "move"),
            ("Enter / →", "open dir / pick file"),
            ("⌫ / ←", "parent dir"),
            ("/", "filter · . hidden · ~ home"),
            ("Esc", "cancel"),
        ] {
            lines.push(help_line(k, d, t));
        }
        lines.push(Line::raw(""));
        lines.push(section("Conversation — Select", t));
        for (k, d) in [
            ("↑/↓ k/j · PgUp/Dn", "move cursor"),
            ("g / G · Home / End", "first / latest message"),
            ("v", "visual anchor — motions then extend the range"),
            ("{ / }", "previous / next speaker run"),
            ("Alt+Shift+K / J", "shade a range (or Alt+Shift+↑/↓)"),
            ("Space", "mark / unmark (multi-select)"),
            ("x / +", "delete / react — ALL marked (or the cursor)"),
            (
                "+ then Enter",
                "fast react: empty query sends your most-used",
            ),
            ("y", "copy selection (author + time + body)"),
            (
                "c",
                "copy body — or the image itself, for an image attachment",
            ),
            ("o / u", "open / copy the first link in the message"),
            ("e / p", "edit / pin the cursor message"),
            ("r / s", "reply / download"),
            ("i / a", "back to compose (insert)"),
            (
                "Enter",
                "on a reply: jump to the quoted message · else back to compose",
            ),
            ("/", "search this conversation"),
            (
                "n / N",
                "next / previous search hit (retained after Ctrl+F)",
            ),
            ("Esc", "clear selection · then close the conversation"),
        ] {
            lines.push(help_line(k, d, t));
        }
    } else {
        match from {
            Screen::Teams => {
                lines.push(section("Teams", t));
                for (k, d) in [
                    ("↑/↓ k/j", "navigate"),
                    ("Enter / → / l", "browse the team's channels"),
                    ("r / F5", "refresh"),
                    ("Esc", "back to inbox"),
                ] {
                    lines.push(help_line(k, d, t));
                }
            }
            Screen::ChannelBrowser => {
                lines.push(section("Channel browser (c)", t));
                for (k, d) in [
                    ("↑/↓ k/j · PgUp/Dn", "navigate"),
                    (
                        "Enter / → / l",
                        "open a joined channel · join one you're not in",
                    ),
                    ("n", "create a new channel"),
                    ("r", "rename the channel"),
                    ("t", "toggle as a team default (new members auto-join)"),
                    ("m", "members of the channel"),
                    ("Shift+L", "leave a joined channel"),
                    ("x", "delete the channel"),
                    ("F5 / Esc", "refresh / close"),
                ] {
                    lines.push(help_line(k, d, t));
                }
            }
            Screen::Members => {
                lines.push(section("Members (m / Alt+P)", t));
                for (k, d) in [
                    ("↑/↓ k/j · PgUp/Dn", "navigate"),
                    ("a", "add member(s) (comma/space-separated)"),
                    ("x", "remove the selected member"),
                    ("F5 / Esc", "refresh / back"),
                ] {
                    lines.push(help_line(k, d, t));
                }
            }
            Screen::Login => {
                lines.push(section("Login", t));
                for (k, d) in [
                    ("Tab / ↑↓", "move between fields / buttons"),
                    ("(typing)", "edit the focused field"),
                    ("F2", "reveal / hide the paper key"),
                    (
                        "Enter",
                        "Log in — paper key (new device) · terminal button (already provisioned)",
                    ),
                    ("F5", "retry the status check"),
                    ("Esc", "clear the focused field"),
                    ("Ctrl+C", "quit"),
                ] {
                    lines.push(help_line(k, d, t));
                }
                let note = Style::default().fg(t.dim);
                for n in [
                    "",
                    "  Paper-key login works only on a device never",
                    "  provisioned for this account. A device you merely",
                    "  logged out of is still provisioned → use \"Log in in",
                    "  terminal\", which hands off to keybase for the passphrase.",
                ] {
                    lines.push(Line::from(Span::styled(n.to_string(), note)));
                }
            }
            Screen::Settings => {
                lines.push(section("Settings (F10)", t));
                for (k, d) in [
                    ("↑/↓", "navigate"),
                    ("Tab", "sidebar ↔ panel"),
                    ("←/→", "change setting"),
                    ("Enter / Space", "toggle · next option"),
                    ("Esc", "panel → sidebar → close"),
                    ("F10", "close"),
                ] {
                    lines.push(help_line(k, d, t));
                }
            }
            _ => {
                lines.push(section("Inbox — navigate", t));
                for (k, d) in [
                    ("↑/↓ k/j", "navigate"),
                    ("PgUp/PgDn", "page"),
                    ("g / G", "top / bottom"),
                    ("Enter / → / l", "open conversation / expand group"),
                    ("← / h", "close chat / collapse group"),
                    ("/", "filter chats (or Alt+F)"),
                    ("Tab / Shift+Tab", "cycle focus"),
                    ("Ctrl+W then h/j/k/l", "move between panels (or arrows)"),
                ] {
                    lines.push(help_line(k, d, t));
                }
                lines.push(Line::raw(""));
                lines.push(section("Inbox — go to panel / global", t));
                for (k, d) in [
                    ("Alt+C / Alt+M", "go to [Alt+C] chats / [Alt+M] messages"),
                    ("Alt+F / Alt+L", "go to [Alt+F] filter / [Alt+L] log"),
                    ("Ctrl+F", "in-chat find (open conversation)"),
                    ("Ctrl+G", "global search"),
                    ("Ctrl+K", "quick switcher — jump to a conversation"),
                    ("Ctrl+P", "command palette — run any action"),
                ] {
                    lines.push(help_line(k, d, t));
                }
                lines.push(Line::raw(""));
                lines.push(section("Inbox — actions (lower = safe · Shift = loud)", t));
                for (k, d) in [
                    ("n", "new conversation"),
                    ("r", "refresh inbox (or F5)"),
                    ("y", "yank (copy) label"),
                    ("e", "mark as seen (read)"),
                    ("u", "mute (local only — toggle; hides unread badge)"),
                    ("s", "★ favorite (local only — toggle)"),
                    ("t", "teams"),
                    (
                        "c",
                        "channels (team) — join/leave/new/rename/del/default/members",
                    ),
                    ("Shift+I", "ignore conversation"),
                    ("Shift+B", "block conversation"),
                    ("Shift+R", "report conversation"),
                    (
                        "Shift+H",
                        "unhide — restore a blocked/reported chat by name",
                    ),
                    ("Shift+L", "logout"),
                ] {
                    lines.push(help_line(k, d, t));
                }
                lines.push(Line::raw(""));
                lines.push(section("Command log (focused)", t));
                for (k, d) in [
                    ("↑/↓ k/j", "move cursor (scrolls history)"),
                    ("PgUp/Dn · g/G", "page · oldest / newest"),
                    ("v", "visual anchor — j/k extend the range"),
                    ("Alt+Shift+K / J", "shade a range (or Alt+Shift+↑/↓)"),
                    ("Space", "mark / unmark a line (multi-select)"),
                    ("y / Enter", "copy full line(s)"),
                    ("c", "copy detail only"),
                    ("Esc", "clear selection · then leave the panel"),
                ] {
                    lines.push(help_line(k, d, t));
                }
                // Explain ignore — its semantics aren't obvious from the
                // one-liner, and it can't currently be undone from the app.
                let note = Style::default().fg(t.dim);
                for n in [
                    "",
                    "  Ignore hides the chat until its next message.",
                    "  Block / Report hide it for good — they leave the",
                    "  inbox list, so restore them by name with Shift+H.",
                    "  (Mute, u, keeps it visible but silent.)",
                ] {
                    lines.push(Line::from(Span::styled(n.to_string(), note)));
                }
            }
        }
    }

    lines.push(Line::raw(""));
    lines.push(section("Global", t));
    for (k, d) in [
        (
            "-- MODE --",
            "the badge: NORMAL act · COMPOSE type · SELECT act on messages · SEARCH type a query",
        ),
        ("F1", "toggle help"),
        ("F10", "settings"),
        (
            "Ctrl+W / Ctrl+U",
            "in any input: delete word / to line start",
        ),
        ("Ctrl+←/→ · Ctrl+A/E", "in any input: word jump · start/end"),
        ("Ctrl+D / Ctrl+U", "in any list/viewer: half-page down / up"),
        (
            "Ctrl+J / Ctrl+K",
            "in any picker popup: selection down / up",
        ),
        (":", "command palette (from any non-typing surface)"),
        ("Ctrl+N", "next unread conversation (wraps)"),
        ("Ctrl+O", "previous conversation (alternate-buffer toggle)"),
        ("↑/↓ j/k · PgUp/Dn", "scroll this help"),
        ("g / G · Home/End", "top / bottom of help"),
        ("q / Esc", "close help"),
        ("Ctrl+C", "quit"),
    ] {
        lines.push(help_line(k, d, t));
    }
    lines
}

/// Centred ▲ / ▼ marks on the top/bottom border when content is hidden.
fn draw_scroll_indicators(frame: &mut Frame, popup: Rect, scroll_y: u16, max_y: u16, t: &Theme) {
    if popup.width < 4 || popup.height < 4 {
        return;
    }
    let style = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let mid_x = popup.x + popup.width / 2;
    if scroll_y > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("▲", style)),
            Rect {
                x: mid_x,
                y: popup.y,
                width: 1,
                height: 1,
            },
        );
    }
    if scroll_y < max_y {
        frame.render_widget(
            Paragraph::new(Span::styled("▼", style)),
            Rect {
                x: mid_x,
                y: popup.y + popup.height - 1,
                width: 1,
                height: 1,
            },
        );
    }
}
