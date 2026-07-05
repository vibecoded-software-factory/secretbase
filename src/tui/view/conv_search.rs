//! In-conversation search modal (`Ctrl+F` → `searchregexp`): a query box over
//! the match list, jumping to the picked message. Sibling of the global-search
//! modal; shares the standard modal geometry. Each hit is two rows — the sender
//! + snippet, then a dim day/time line for context.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::domain::message_time;
use crate::tui::App;
use crate::tui::view::widgets::{
    MODAL_HEIGHT, MODAL_WIDTH_PCT, center_rect, editor_spans, rounded_block, trim_end_ellipsis,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let area = center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, frame.area());
    frame.render_widget(Clear, area);
    let block = rounded_block(Style::default().fg(t.accent))
        .title(Span::styled(
            " Search conversation ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " Enter search/jump · ↑↓ pick · Esc cancel ",
                Style::default().fg(t.muted),
            ))
            .right_aligned(),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // query
        Constraint::Length(1), // spacer
        Constraint::Min(1),    // results
    ])
    .split(inner);
    let (query_area, list_area) = (rows[0], rows[2]);

    let query_line = if app.conv_search.text().is_empty() {
        Line::from(vec![
            Span::styled("⌕ ", Style::default().fg(t.accent)),
            Span::styled("search this chat…", Style::default().fg(t.placeholder)),
        ])
    } else {
        let mut spans = vec![Span::styled("⌕ ", Style::default().fg(t.accent))];
        spans.extend(editor_spans(&app.conv_search, true, t));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(query_line), query_area);

    let results = &app.conv_search_results;
    if results.is_empty() {
        let msg = if app.conv_search.text().trim().is_empty() {
            "  type a query, then Enter"
        } else {
            "  no matches — Enter to search"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(t.dim)))),
            list_area,
        );
        return;
    }

    // Each result spans two rows; window the list around the selection.
    let sel = app
        .conv_search_selected
        .min(results.len().saturating_sub(1));
    let per_view = (list_area.height.max(1) as usize / 2).max(1);
    let scroll = if sel >= per_view {
        sel + 1 - per_view
    } else {
        0
    };
    let max_w = list_area.width.saturating_sub(6).max(8) as usize;

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, hit) in results.iter().enumerate().skip(scroll).take(per_view) {
        let selected = i == sel;
        let snippet = trim_end_ellipsis(hit.body_summary.lines().next().unwrap_or(""), max_w);
        let mut spans = vec![
            Span::styled(
                if selected { "▶ " } else { "  " }.to_string(),
                Style::default().fg(t.accent),
            ),
            Span::styled(format!("{}: ", hit.sender), Style::default().fg(t.dim)),
            Span::styled(snippet, Style::default().fg(t.foreground)),
        ];
        let when = if hit.sent_at > 0 {
            message_time(hit.sent_at, now_s)
        } else {
            "—".to_string()
        };
        let mut time_spans = vec![Span::styled(
            format!("      {when}"),
            // dim, not placeholder: the time is why this row exists —
            // secondary-but-needed content never goes in the recessive band.
            Style::default().fg(t.dim),
        )];
        if selected {
            for s in &mut spans {
                s.style = s.style.bg(t.selected_bg);
            }
            for s in &mut time_spans {
                s.style = s.style.bg(t.selected_bg);
            }
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(time_spans));
    }
    frame.render_widget(Paragraph::new(lines), list_area);
}
