//! Shared widgets — the single list/table renderer, identity bar,
//! command-log panel, status strip, search box, line-editor spans and
//! the y/n confirm overlay. Ported from jewel so all three TUIs share
//! the same chrome and implementation.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};

use crate::domain::LineEditor;
use crate::tui::App;
use crate::tui::action::ActionState;
use crate::tui::theme::Theme;
use crate::tui::view::titled_block;

const SPINNER: &[&str] = &["⠋", "⠙", "⠸", "⠴"];

/// Title for a filtered list block: `"{subject} · {filtered} of {total}"`.
/// Every list screen builds its `list_table` title this way.
pub fn list_title(subject: &str, filtered: usize, total: usize) -> String {
    format!("{subject} · {filtered} of {total}")
}

/// Rounded-border [`Block`] with the supplied border style. Used by the
/// few centered popups that aren't the shared confirm overlay.
pub fn rounded_block(border_style: Style) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
}

/// Returns a sub-rectangle centered horizontally and vertically inside
/// `area`. `width_pct` is a percentage (0–100), `height` is in rows.
pub fn center_rect(width_pct: u16, height: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width_pct) / 2),
        Constraint::Percentage(width_pct),
        Constraint::Percentage((100 - width_pct) / 2),
    ])
    .split(v[1])[1]
}

/// One row of the help popup (key + description).
pub fn help_line<'a>(key: &'a str, desc: &'a str, t: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{key:<16}"), Style::default().fg(t.accent)),
        Span::styled(desc, Style::default().fg(t.foreground)),
    ])
}

/// The one list/table renderer every list screen uses. Draws a bordered
/// block titled `title`, a dim header row, the rows with content columns,
/// the `▶ ` selection highlight, and persists the scroll offset through
/// `*scroll`.
///
/// `widths` must match `headers` in length. Size content columns with
/// [`col_width`] — never a stretching `Min` on a non-final content
/// column.
#[allow(clippy::too_many_arguments)]
pub fn list_table(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    focused: bool,
    headers: &[&str],
    widths: &[Constraint],
    rows: Vec<Row<'static>>,
    selected: usize,
    scroll: &mut usize,
) {
    let header = Row::new(
        headers
            .iter()
            .map(|h| {
                Cell::from(*h).style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD))
            })
            .collect::<Vec<_>>(),
    );
    let len = rows.len();
    let border_style = if focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.inactive)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title.to_string(), border_style))
        .border_style(border_style);
    let table = Table::new(rows, widths.to_vec())
        .header(header)
        .column_spacing(2)
        .row_highlight_style(
            Style::default()
                .bg(theme.selected_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .block(block);

    let sel = (len > 0).then(|| selected.min(len - 1));
    let mut state = TableState::default()
        .with_offset(*scroll)
        .with_selected(sel);
    frame.render_stateful_widget(table, area, &mut state);
    *scroll = state.offset();
}

/// Width for a content column, sized to the *visible* rows (`indices`),
/// clamped to `[lo, hi]`. `f` maps a row index to its content length.
pub fn col_width(indices: &[usize], lo: u16, hi: u16, f: impl Fn(usize) -> usize) -> u16 {
    (indices.iter().map(|&i| f(i)).max().unwrap_or(0) as u16).clamp(lo, hi)
}

/// Truncates `s` to `max` columns with a `…` in the middle, **biased to
/// the head**: identifiers carry their discriminator near the front, so
/// the head keeps ~⅔ and a short tail preserves the suffix. Char-based
/// (UTF-8 safe).
pub fn middle_ellipsis(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let keep = max - 1; // room for the ellipsis
    let tail = (keep / 3).max(1); // short suffix
    let head = keep - tail; // ~⅔ to the front
    let head_str: String = s.chars().take(head).collect();
    let tail_str: String = s.chars().skip(len - tail).collect();
    format!("{head_str}…{tail_str}")
}

/// Maps a click row `y` to a filtered-row index for a [`list_table`] in
/// `rect` scrolled by `scroll`. Accounts for the block border (+1) and
/// the header row (+1). `None` on the border/header or past the last row.
pub fn table_row_at(rect: Rect, y: u16, scroll: usize, len: usize) -> Option<usize> {
    let top = rect.y.saturating_add(2);
    let bottom = rect.y.saturating_add(rect.height).saturating_sub(1);
    if y < top || y >= bottom {
        return None;
    }
    let idx = scroll + (y - top) as usize;
    (idx < len).then_some(idx)
}

/// Spans for one `[x]`/`[ ] <label>` checkbox row: a leading `▶ `
/// (or two-space gutter) cursor, the `[x]`/`[ ]` mark, and the label.
pub fn checkbox_spans(
    theme: &Theme,
    checked: bool,
    label: &str,
    focused: bool,
) -> Vec<Span<'static>> {
    let (mark, mark_style) = if checked {
        (
            "[x]",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("[ ]", Style::default().fg(theme.inactive))
    };
    let leading: Span = if focused {
        Span::styled(
            "▶ ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let label_style = if focused {
        Style::default()
            .fg(theme.foreground)
            .add_modifier(Modifier::BOLD)
    } else if checked {
        Style::default().fg(theme.foreground)
    } else {
        Style::default().fg(theme.dim)
    };
    vec![
        leading,
        Span::styled(mark, mark_style),
        Span::raw(" "),
        Span::styled(label.to_string(), label_style),
    ]
}

/// A single tab/chip span: `" {label} "`. When `active` it's `accent`
/// on `selected_bg` + BOLD; enabled-inactive is `dim`; disabled is
/// `muted`.
pub fn chip_span(theme: &Theme, label: &str, active: bool, enabled: bool) -> Span<'static> {
    let style = if !enabled {
        Style::default().fg(theme.muted)
    } else if active {
        Style::default()
            .fg(theme.accent)
            .bg(theme.selected_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dim)
    };
    Span::styled(format!(" {label} "), style)
}

/// The shared y/n confirmation overlay — a centered, double-bordered
/// popup with `title`, the caller's `body` lines, and the
/// confirm/cancel buttons (the highlighted one follows `confirmed`).
pub fn draw_confirm_popup(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    body: Vec<Line<'static>>,
    confirmed: bool,
) {
    let w = 66u16.min(area.width);
    let h = (body.len() as u16 + 4).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);
    let inner = Rect {
        x: popup.x + 2,
        y: popup.y + 1,
        width: popup.width.saturating_sub(4),
        height: popup.height.saturating_sub(2),
    };
    let block = Block::default()
        .title(Span::styled(
            title.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.accent));
    frame.render_widget(block, popup);

    let selected = Style::default()
        .fg(theme.accent)
        .bg(theme.selected_bg)
        .add_modifier(Modifier::BOLD);
    let unselected = Style::default().fg(theme.dim);
    let (confirm_style, cancel_style) = if confirmed {
        (selected, unselected)
    } else {
        (unselected, selected)
    };
    let mut lines = body;
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" confirm ", confirm_style),
        Span::raw("   "),
        Span::styled(" cancel ", cancel_style),
        Span::styled("   (←/→ · Enter · Esc)", Style::default().fg(theme.muted)),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Top identity bar — always visible above the per-screen header on
/// every signed-in screen. Renders `user <name> · device <dev>` plus the
/// inbox-wide unread total so the user keeps the big picture everywhere.
pub fn draw_identity_bar(frame: &mut Frame, app: &App, area: Rect) {
    let block = titled_block("Identity", false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let t = &app.theme;
    if !app.identity.logged_in || app.identity.username.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "(not logged in)",
            Style::default().fg(t.dim),
        )));
        frame.render_widget(p, inner);
        return;
    }

    let mut spans = vec![
        Span::styled("user ", Style::default().fg(t.dim)),
        Span::styled(
            app.identity.username.clone(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
    ];
    if !app.identity.device_name.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.muted)));
        spans.push(Span::styled("device ", Style::default().fg(t.dim)));
        spans.push(Span::styled(
            app.identity.device_name.clone(),
            Style::default().fg(t.foreground),
        ));
    }
    let unread = app.unread_total();
    if unread > 0 {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.muted)));
        spans.push(Span::styled(
            format!("{unread} unread"),
            Style::default()
                .fg(t.conv_unread)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

/// Number of content rows the identity bar renders — always 1 for the
/// Keybase identity. Add 2 (block borders) to get the slot height.
pub fn identity_content_rows(_app: &App, _total_width: u16) -> u16 {
    1
}

/// Shared single-line search/filter box: a titled block holding the live
/// query (or a dim placeholder) plus a block cursor when focused.
pub fn draw_search_box(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    title: &str,
    placeholder: &str,
    editor: &LineEditor,
    focused: bool,
) {
    let line = if editor.is_empty() && !focused {
        Line::from(Span::styled(
            placeholder.to_string(),
            Style::default().fg(app.theme.placeholder),
        ))
    } else {
        Line::from(editor_spans(editor, focused, &app.theme))
    };
    let p = Paragraph::new(line).block(titled_block(title, focused, app));
    frame.render_widget(p, area);
}

/// Renders a [`LineEditor`]'s content as spans, drawing a block cursor
/// (reverse-video) at the cursor position when `focused`. The one
/// text-input renderer.
pub fn editor_spans(editor: &LineEditor, focused: bool, theme: &Theme) -> Vec<Span<'static>> {
    let text = editor.text();
    let base = Style::default().fg(theme.foreground);
    if !focused {
        return vec![Span::styled(text.to_string(), base)];
    }
    let cursor = Style::default().add_modifier(Modifier::REVERSED);
    let cur = editor.cursor().min(text.len());
    let before = text[..cur].to_string();
    if cur >= text.len() {
        return vec![Span::styled(before, base), Span::styled(" ", cursor)];
    }
    let next = text[cur..]
        .char_indices()
        .nth(1)
        .map(|(i, _)| cur + i)
        .unwrap_or(text.len());
    vec![
        Span::styled(before, base),
        Span::styled(text[cur..next].to_string(), cursor),
        Span::styled(text[next..].to_string(), base),
    ]
}

/// Renders the command-log panel (`✓ cmd  →  detail`), newest at the
/// bottom. `cmd_log_scroll` walks back through history.
///
/// Takes `&mut App` because the renderer owns the viewport: it clamps
/// `app.cmd_log_scroll` against the real overflow and writes it back, so
/// the input handler can use a `usize::MAX` "jump to oldest" sentinel
/// without the title showing a 20-digit number or the scroll getting
/// stuck above the bottom.
pub fn draw_cmd_log(frame: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    // Inner height = block area minus the two borders.
    let visible_rows = (area.height as usize).saturating_sub(2);
    let total = app.cmd_log.len();
    let max_back = total.saturating_sub(visible_rows);
    app.cmd_log_scroll = app.cmd_log_scroll.min(max_back);
    let scroll = app.cmd_log_scroll;

    let scroll_tag = if scroll == 0 {
        String::new()
    } else {
        format!(" ↑{scroll}")
    };
    let title = format!("Command log{scroll_tag}");
    let block = titled_block(&title, focused, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if visible_rows == 0 {
        return;
    }
    if app.cmd_log.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no commands yet",
                Style::default().fg(app.theme.dim),
            ))),
            inner,
        );
        return;
    }
    let end = total - scroll;
    let start = end.saturating_sub(visible_rows);
    let lines: Vec<Line> = app.cmd_log[start..end]
        .iter()
        .map(|e| {
            let mark = if e.ok { "✓" } else { "✗" };
            let mark_style = if e.ok {
                Style::default().fg(app.theme.success)
            } else {
                Style::default().fg(app.theme.error)
            };
            let mut spans = vec![
                Span::styled(format!("{mark} "), mark_style),
                Span::styled(e.cmd.clone(), Style::default().fg(app.theme.foreground)),
                Span::styled(
                    format!("  →  {}", e.detail),
                    Style::default().fg(app.theme.dim),
                ),
            ];
            // How long the operation took (request → response).
            if let Some(d) = e.duration {
                spans.push(Span::styled(
                    format!("  ({})", crate::domain::format_duration(d)),
                    Style::default().fg(app.theme.placeholder),
                ));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the bottom status / feedback strip. While an action is in
/// flight (or just finished) it shows the spinner/✓/✗ message full
/// width; idle, it shows `footer_hint` on the left with `F1 help`
/// anchored right.
pub fn draw_status_strip(frame: &mut Frame, app: &App, area: Rect, footer_hint: &str) {
    let feedback = match &app.action_state {
        ActionState::Idle => None,
        ActionState::Running(msg) => {
            let spin = SPINNER[app.action_tick as usize % SPINNER.len()];
            Some((
                format!("{spin} {msg}"),
                Style::default().fg(app.theme.accent),
            ))
        }
        ActionState::Done(msg) => Some((msg.clone(), Style::default().fg(app.theme.success))),
        ActionState::Error(msg) => Some((msg.clone(), Style::default().fg(app.theme.error))),
    };

    if let Some((text, style)) = feedback {
        let trimmed: String = text.chars().take(area.width as usize).collect();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(trimmed, style))),
            area,
        );
        return;
    }

    const HELP_ANCHOR: &str = "F1 help · F9 settings";
    let anchor_block = HELP_ANCHOR.chars().count() + 2;
    let avail = (area.width as usize).saturating_sub(anchor_block);
    let hint: String = footer_hint.chars().take(avail).collect();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(app.theme.dim),
        ))),
        area,
    );
    frame.render_widget(
        Paragraph::new(
            Line::from(Span::styled(
                HELP_ANCHOR,
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_title_formats() {
        assert_eq!(list_title("Inbox", 3, 10), "Inbox · 3 of 10");
        assert_eq!(list_title("Teams", 0, 0), "Teams · 0 of 0");
    }

    #[test]
    fn middle_ellipsis_is_head_biased() {
        assert_eq!(middle_ellipsis("short", 10), "short");
        let out = middle_ellipsis("team.subteam.channel.very.long.name", 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.contains('…'));
    }

    #[test]
    fn col_width_clamps() {
        assert_eq!(col_width(&[0, 1], 4, 10, |_| 2), 4);
        assert_eq!(col_width(&[0, 1], 4, 10, |_| 7), 7);
        assert_eq!(col_width(&[0, 1], 4, 10, |_| 99), 10);
        assert_eq!(col_width(&[], 4, 10, |_| 99), 4);
    }
}
