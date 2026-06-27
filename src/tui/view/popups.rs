//! Small input popups: reaction input + attachment download path.
//!
//! The destructive confirmations (logout, delete message) render through
//! the shared `widgets::draw_confirm_popup` from `view::mod`.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::tui::app::App;
use crate::tui::view::widgets::{center_rect, editor_spans, rounded_block};

/// Searchable reaction picker: a `/`-style search box over the cached emoji
/// catalogue, an arrow-navigable list (unicode glyph + `:alias:`), and a
/// custom-`:shortcode:` fallback when nothing matches.
pub fn react_input(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(60, 18, frame.area());
    frame.render_widget(Clear, area);

    let header = app
        .selected_msg_idx
        .and_then(|i| app.messages.get(i))
        .map(|m| format!(" React to #{} · by {} ", m.id, m.sender))
        .unwrap_or_else(|| " React ".to_string());
    let block = rounded_block(Style::default().fg(t.accent))
        .title(Span::styled(header, Style::default().fg(t.accent)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // search
        Constraint::Min(1),    // emoji list
        Constraint::Length(1), // hint
    ])
    .split(inner);

    // Search box.
    let mut search = vec![Span::styled("/ ", Style::default().fg(t.accent))];
    search.extend(editor_spans(&app.react, true, t));
    frame.render_widget(Paragraph::new(Line::from(search)), rows[0]);

    // Filtered list with a viewport window around the selection.
    let filtered = app.filtered_emoji_indices();
    let vh = rows[1].height as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if filtered.is_empty() {
        let msg = if app.emojis_loading {
            "  loading emojis…"
        } else if app.react.text().trim().is_empty() {
            "  type to search, or a :shortcode:"
        } else {
            "  no match — Enter sends it as a custom :shortcode:"
        };
        lines.push(Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(t.dim),
        )));
    } else {
        let sel = app.react_selected.min(filtered.len() - 1);
        let scroll = if vh > 0 && sel >= vh { sel + 1 - vh } else { 0 };
        for (row, &ei) in filtered.iter().enumerate().skip(scroll).take(vh.max(1)) {
            let e = &app.emojis[ei];
            let mut spans = vec![
                Span::styled(
                    if row == sel { "▶ " } else { "  " }.to_string(),
                    Style::default().fg(t.accent),
                ),
                Span::styled(
                    format!("{}  ", e.display),
                    Style::default().fg(t.foreground),
                ),
                Span::styled(format!(":{}:", e.alias), Style::default().fg(t.dim)),
            ];
            if row == sel {
                for s in &mut spans {
                    s.style = s.style.bg(t.selected_bg);
                }
            }
            lines.push(Line::from(spans));
        }
    }
    frame.render_widget(Paragraph::new(lines), rows[1]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑↓ select · Enter react · type to search · Esc cancel",
            Style::default().fg(t.dim),
        )))
        .alignment(Alignment::Center),
        rows[2],
    );
}

/// Quick switcher (Ctrl+K): a centered search box over all conversations,
/// arrow-navigable, Enter jumps to the highlighted one.
pub fn quick_switcher(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(60, 18, frame.area());
    frame.render_widget(Clear, area);

    let block = rounded_block(Style::default().fg(t.accent)).title(Span::styled(
        " Go to a conversation ",
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // search
        Constraint::Min(1),    // results
        Constraint::Length(1), // hint
    ])
    .split(inner);

    let mut search = vec![Span::styled("/ ", Style::default().fg(t.accent))];
    search.extend(editor_spans(&app.switcher, true, t));
    frame.render_widget(Paragraph::new(Line::from(search)), rows[0]);

    let results = app.switcher_results();
    let vh = rows[1].height as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    if results.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no conversation matches".to_string(),
            Style::default().fg(t.dim),
        )));
    } else {
        let sel = app.switcher_selected.min(results.len() - 1);
        let scroll = if vh > 0 && sel >= vh { sel + 1 - vh } else { 0 };
        for (row, &ci) in results.iter().enumerate().skip(scroll).take(vh.max(1)) {
            let label = app
                .conversations_lowered
                .get(ci)
                .map(|l| l.display_label.clone())
                .unwrap_or_default();
            let mut spans = vec![
                Span::styled(
                    if row == sel { "▶ " } else { "  " }.to_string(),
                    Style::default().fg(t.accent),
                ),
                Span::styled(
                    label,
                    if row == sel {
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.foreground)
                    },
                ),
            ];
            if row == sel {
                for s in &mut spans {
                    s.style = s.style.bg(t.selected_bg);
                }
            }
            lines.push(Line::from(spans));
        }
    }
    frame.render_widget(Paragraph::new(lines), rows[1]);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑↓ select · Enter go · type to search · Esc cancel",
            Style::default().fg(t.dim),
        )))
        .alignment(Alignment::Center),
        rows[2],
    );
}
