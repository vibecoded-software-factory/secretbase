//! Command palette overlay (`Ctrl+P`) — a query box over a context-aware,
//! categorized list of actions, each with its keybinding right-aligned. Shares
//! the standard modal geometry with the quick switcher / global search.

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::tui::App;
use crate::tui::flows::palette::{PaletteRow, palette_rows};
use crate::tui::view::widgets::{
    MODAL_HEIGHT, MODAL_WIDTH_PCT, center_rect, editor_spans, key_style, rounded_block,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, frame.area());
    frame.render_widget(Clear, area);
    let block = rounded_block(Style::default().fg(t.accent))
        .title(Span::styled(
            " Command palette ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " ↑↓ select · Enter run · Esc cancel ",
                Style::default().fg(t.muted),
            ))
            .right_aligned(),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // query
        Constraint::Length(1), // spacer
        Constraint::Min(1),    // list
    ])
    .split(inner);
    let (query_area, list_area) = (rows[0], rows[2]);

    // Query line: a leading ⌕ then the editor (or a dim placeholder).
    let query_line = if app.palette.text().is_empty() {
        Line::from(vec![
            Span::styled("⌕ ", Style::default().fg(t.accent)),
            Span::styled("type a command…", Style::default().fg(t.placeholder)),
        ])
    } else {
        let mut spans = vec![Span::styled("⌕ ", Style::default().fg(t.accent))];
        spans.extend(editor_spans(&app.palette, true, t));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(query_line), query_area);

    // Build the list; remember the selected command's line so we can scroll it
    // into view. `cmd_idx` counts only selectable rows, matching what
    // `App::palette_selected` indexes (`filtered_commands`).
    let model = palette_rows(app);
    let width = list_area.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut sel_line: Option<usize> = None;
    let mut cmd_idx = 0usize;
    for row in &model {
        match row {
            PaletteRow::Header(cat) => lines.push(Line::from(Span::styled(
                (*cat).to_string(),
                Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
            ))),
            PaletteRow::Cmd(c) => {
                let selected = cmd_idx == app.palette_selected;
                if selected {
                    sel_line = Some(lines.len());
                }
                let prefix = if selected { "▶ " } else { "  " };
                let label_style = if selected {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.foreground)
                };
                // Right-align the keybinding within the row width.
                let used = prefix.chars().count() + c.label.chars().count();
                let gap = width.saturating_sub(used + c.keys.chars().count()).max(1);
                let mut line = Line::from(vec![
                    Span::styled(prefix.to_string(), Style::default().fg(t.accent)),
                    Span::styled(c.label.to_string(), label_style),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(c.keys.to_string(), key_style(t)),
                ]);
                if selected {
                    line = line.style(Style::default().bg(t.selected_bg));
                }
                lines.push(line);
                cmd_idx += 1;
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching command",
            Style::default().fg(t.dim),
        )));
    }

    let viewport = list_area.height as usize;
    let scroll_y = match sel_line {
        Some(l) if l >= viewport => (l + 1 - viewport) as u16,
        _ => 0,
    };
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), list_area);
}
