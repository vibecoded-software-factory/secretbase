//! Help popup — a scrollable overlay scoped to the screen it was opened
//! from (`App::help_from`). Mirrors jewel's context-aware help.

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
    let from = app.help_from;
    let lines = build_lines(from, &t);

    let h = (frame.area().height.saturating_mul(82) / 100).max(8);
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

    let title = if max_y > 0 {
        format!(
            " Help — {}  ({}/{})  F1/Esc close ",
            screen_label(from),
            scroll_y + 1,
            max_y + 1,
        )
    } else {
        format!(" Help — {}  ·  F1/Esc close ", screen_label(from))
    };

    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(t.accent));
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), inner);

    draw_scroll_indicators(frame, popup, scroll_y, max_y, &t);
}

fn screen_label(screen: Screen) -> &'static str {
    match screen {
        Screen::Inbox => "Inbox",
        Screen::Conversation => "Conversation",
        Screen::Teams => "Teams",
        Screen::Login => "Login",
        _ => "Global",
    }
}

fn section(title: &str, t: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(t.foreground)
            .add_modifier(Modifier::BOLD),
    ))
}

fn build_lines(from: Screen, t: &Theme) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    match from {
        Screen::Conversation => {
            lines.push(section("Conversation — Compose", t));
            for (k, d) in [
                ("(typing)", "extend draft"),
                ("Enter", "send / save edit"),
                ("↑/↓ PgUp/PgDn", "scroll history"),
                ("←/→ Home/End", "compose cursor"),
                ("F5 / Ctrl+R", "refresh"),
                ("Ctrl+Y", "copy label"),
                ("Alt+V", "select mode"),
                ("Alt+E / Alt+D", "edit / delete last own"),
                ("Alt+J / Alt+P", "react / pin last"),
                ("Alt+U", "unpin channel"),
                ("Esc", "cancel / back"),
            ] {
                lines.push(help_line(k, d, t));
            }
            lines.push(Line::raw(""));
            lines.push(section("Conversation — Select", t));
            for (k, d) in [
                ("↑/↓ k/j", "move cursor"),
                ("e / d / : / p", "edit / del / react / pin"),
                ("r / s", "reply / download"),
                ("i / Enter / Esc", "back to compose"),
            ] {
                lines.push(help_line(k, d, t));
            }
        }
        Screen::Teams => {
            lines.push(section("Teams", t));
            for (k, d) in [
                ("↑/↓ k/j", "navigate"),
                ("Alt+R / F5", "refresh"),
                ("Alt+I / Esc", "back to inbox"),
            ] {
                lines.push(help_line(k, d, t));
            }
        }
        Screen::Login => {
            lines.push(section("Login", t));
            for (k, d) in [("R / F5", "retry status"), ("Q / Esc", "quit")] {
                lines.push(help_line(k, d, t));
            }
        }
        _ => {
            lines.push(section("Inbox", t));
            for (k, d) in [
                ("↑/↓ k/j", "navigate"),
                ("PgUp/PgDn", "page"),
                ("g / G", "top / bottom"),
                ("Tab / Shift+Tab", "cycle focus"),
                ("/", "search"),
                ("Enter / l", "open conversation"),
                ("Alt+N", "new conversation"),
                ("Alt+C", "copy label"),
                ("Alt+M", "mark as read"),
                ("Alt+R / F5", "refresh inbox"),
                ("Alt+U / Alt+O", "mute / unmute"),
                ("Alt+I", "ignore conversation"),
                ("Alt+T", "teams"),
                ("Ctrl+G", "global search"),
                ("Shift+L", "logout"),
            ] {
                lines.push(help_line(k, d, t));
            }
            // Explain ignore — its semantics aren't obvious from the
            // one-liner, and it can't currently be undone from the app.
            let note = Style::default().fg(t.dim);
            for n in [
                "",
                "  Ignore hides the conversation from the inbox.",
                "  Keybase brings it back automatically on the",
                "  next message in that chat. (Mute, Alt+U, keeps",
                "  it visible but silent — and is easy to undo.)",
            ] {
                lines.push(Line::from(Span::styled(n.to_string(), note)));
            }
        }
    }

    lines.push(Line::raw(""));
    lines.push(section("Global", t));
    for (k, d) in [
        ("F1", "toggle help"),
        ("↑/↓ j/k", "scroll help"),
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
