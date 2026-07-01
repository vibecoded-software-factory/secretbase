//! Shared widgets — the single list/table renderer, identity bar,
//! command-log panel, status strip, search box, line-editor spans and
//! the y/n confirm overlay — the shared chrome every screen reuses.

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

/// Standard centered-modal geometry — the shape every list / picker overlay
/// imitates so they all line up: global search (Ctrl+G), the quick switcher
/// (Ctrl+K), the reaction picker, the file picker and the settings overlay
/// share it. `MODAL_WIDTH_PCT` is a percentage of the terminal width;
/// `MODAL_HEIGHT` is a fixed row height (the settings box keeps its own
/// content-sized width but adopts this height + centered position).
pub const MODAL_WIDTH_PCT: u16 = 80;
pub const MODAL_HEIGHT: u16 = 22;

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

/// Width of the conversation-tree pane, responsive to the terminal width
/// instead of a magic fixed `28`: ~28% of the width, clamped to `[22, 40]`.
/// It **shrinks** toward the floor on a narrow terminal (so the chat keeps
/// room) and **grows** on a wide one (so long DM/team names aren't always
/// truncated). The Home's header filter and body tree must pass the same
/// `total` so their columns line up.
pub fn tree_pane_width(total: u16) -> u16 {
    ((total as u32 * 28 / 100) as u16).clamp(22, 40)
}

/// Height of the command-log panel, responsive to the terminal height so the
/// body (chat / list) never starves on a short terminal. Full 6 rows when
/// there's room; it yields rows to the body as height gets tight, monotonically
/// (a taller terminal never shrinks the body). Used by every signed-in stack.
pub fn cmdlog_height(total: u16) -> u16 {
    if total >= 28 {
        6
    } else if total >= 22 {
        5
    } else if total >= 19 {
        4
    } else {
        3
    }
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
    // The `subject · X of Y` count goes in the bottom-right border (dim),
    // leaving the top title as the section name + its `─[N]-` tag.
    let (top_title, count) = match title.split_once(" · ") {
        Some((t, c)) => (t, c),
        None => (title, ""),
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(top_title.to_string(), border_style))
        .border_style(border_style);
    if !count.is_empty() {
        block = block.title_bottom(
            Line::from(Span::styled(
                count.to_string(),
                Style::default().fg(theme.dim),
            ))
            .right_aligned(),
        );
    }
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

    // In-range `selected` highlights that row; an out-of-range value (e.g.
    // `usize::MAX`) renders with no highlight — used by panels that share one
    // logical selection across several tables (only the active one shows it).
    let sel = (selected < len).then_some(selected);
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

/// Trims `s` to at most `max` characters, appending a trailing `…` when it
/// overflows (UTF-8 safe). For one-line previews where the **start** carries
/// the meaning — reply quotes, search-result snippets — as opposed to
/// [`middle_ellipsis`], which preserves the tail (a file extension, a `#id`).
pub fn trim_end_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let head: String = s.chars().take(max - 1).collect();
    format!("{head}…")
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

    // Fit to width like the footer hint does (never a hard clip mid-word):
    // the username is mandatory, then add `unread` (it outranks `device` when
    // space is tight — a count you must notice beats a device label), then
    // `device`. The username truncates only as a last resort on a very narrow
    // terminal. Widths are char-based (each includes its leading separator).
    let avail = inner.width as usize;
    let uname = app.identity.username.clone();
    let dev = app.identity.device_name.clone();
    let unread = app.unread_total();
    let unread_txt = if unread > 0 {
        format!("{unread} unread")
    } else {
        String::new()
    };
    let user_w = "user ".len() + uname.chars().count();
    let dev_w = if dev.is_empty() {
        0
    } else {
        "  ·  device ".chars().count() + dev.chars().count()
    };
    let unread_w = if unread > 0 {
        "  ·  ".chars().count() + unread_txt.chars().count()
    } else {
        0
    };
    let mut show_dev = dev_w > 0;
    let mut show_unread = unread_w > 0;
    if user_w + dev_w + unread_w > avail {
        show_dev = false;
    }
    if user_w + if show_dev { dev_w } else { 0 } + unread_w > avail {
        show_unread = false;
    }

    let mut spans = vec![Span::styled("user ", Style::default().fg(t.dim))];
    let fixed = if show_dev { dev_w } else { 0 } + if show_unread { unread_w } else { 0 };
    let uname_budget = avail.saturating_sub("user ".len() + fixed).max(1);
    spans.push(Span::styled(
        trim_end_ellipsis(&uname, uname_budget),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ));
    if show_dev {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.muted)));
        spans.push(Span::styled("device ", Style::default().fg(t.dim)));
        spans.push(Span::styled(dev, Style::default().fg(t.foreground)));
    }
    if show_unread {
        spans.push(Span::styled("  ·  ", Style::default().fg(t.muted)));
        spans.push(Span::styled(
            unread_txt,
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
#[allow(clippy::too_many_arguments)]
pub fn draw_search_box(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    tag: &str,
    title: &str,
    placeholder: &str,
    editor: &LineEditor,
    focused: bool,
    disabled: bool,
) {
    let line = if disabled {
        // Unreachable box (e.g. the in-chat search with no conversation open):
        // muted placeholder + muted border so it reads as unavailable.
        Line::from(Span::styled(
            placeholder.to_string(),
            Style::default().fg(app.theme.muted),
        ))
    } else if editor.is_empty() && !focused {
        Line::from(Span::styled(
            placeholder.to_string(),
            Style::default().fg(app.theme.placeholder),
        ))
    } else {
        Line::from(editor_spans(editor, focused, &app.theme))
    };
    // `─[tag]-` panel border tag — the key that focuses this box, mirroring
    // the numbered list-section borders (e.g. `/`, `^f`).
    let title = format!("─[{tag}]-{title}");
    let block = if disabled {
        crate::tui::view::disabled_block(&title, app)
    } else {
        titled_block(&title, focused, app)
    };
    let p = Paragraph::new(line).block(block);
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

/// Multi-line variant of [`editor_spans`]: splits the editor text on `\n`
/// into one [`Line`] per row, with the reversed block cursor on the row +
/// column matching `editor.cursor()`. Used by the compose box, which allows
/// newlines (Alt+Enter).
pub fn editor_lines(editor: &LineEditor, theme: &Theme) -> Vec<Line<'static>> {
    let text = editor.text();
    let base = Style::default().fg(theme.foreground);
    let cursor = Style::default().add_modifier(Modifier::REVERSED);
    let cur = editor.cursor().min(text.len());

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut start = 0usize;
    loop {
        // `end` is the next '\n' byte (or end of text); the cursor sits on
        // this row when it falls in [start, end] (end = just before the \n).
        let end = text[start..]
            .find('\n')
            .map(|i| start + i)
            .unwrap_or(text.len());
        let seg = &text[start..end];
        if cur >= start && cur <= end {
            let col = cur - start;
            if col >= seg.len() {
                out.push(Line::from(vec![
                    Span::styled(seg.to_string(), base),
                    Span::styled(" ".to_string(), cursor),
                ]));
            } else {
                let next = seg[col..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| col + i)
                    .unwrap_or(seg.len());
                out.push(Line::from(vec![
                    Span::styled(seg[..col].to_string(), base),
                    Span::styled(seg[col..next].to_string(), cursor),
                    Span::styled(seg[next..].to_string(), base),
                ]));
            }
        } else {
            out.push(Line::from(Span::styled(seg.to_string(), base)));
        }
        if end >= text.len() {
            break;
        }
        start = end + 1;
    }
    out
}

/// Renders the command-log panel (`✓ cmd  →  detail`), newest at the
/// bottom. `cmd_log_scroll` walks back through history.
///
/// Takes `&mut App` because the renderer owns the viewport: it clamps
/// `app.cmd_log_scroll` against the real overflow and writes it back, so
/// the input handler can use a `usize::MAX` "jump to oldest" sentinel
/// without the title showing a 20-digit number or the scroll getting
/// stuck above the bottom.
pub fn draw_cmd_log(frame: &mut Frame, app: &mut App, area: Rect, focused: bool, tag: &str) {
    // Inner height = block area minus the two borders.
    let visible_rows = (area.height as usize).saturating_sub(2);
    let total = app.cmd_log.len();

    // When focused, the window follows the visual-select cursor (so you can
    // scroll the whole history); otherwise it stays pinned to the newest.
    let cursor = app.cmdlog_cursor.min(total.saturating_sub(1));
    let (start, end) = if focused && total > visible_rows {
        let end = (cursor + 1).max(visible_rows).min(total);
        (end - visible_rows, end)
    } else {
        (total.saturating_sub(visible_rows), total)
    };
    app.cmd_log_scroll = total - end; // keep the field consistent for clicks

    // Title: show the cursor position + selection while focused.
    let pos = if focused && total > 0 {
        let marks = app.cmdlog_marks.len();
        let sel = if marks > 0 {
            format!(" · {marks} sel")
        } else {
            String::new()
        };
        format!(" {}/{total}{sel}", cursor + 1)
    } else if total > end {
        format!(" ↓{}", total - end)
    } else {
        String::new()
    };
    let title = format!("─[{tag}]-Command log{pos}");
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
    let lines: Vec<Line> = (start..end)
        .map(|i| {
            let e = &app.cmd_log[i];
            let is_cursor = focused && i == cursor;
            let is_marked = app.cmdlog_marks.contains(&i);
            // Left gutter: ▶ cursor · ● marked · two-space pad otherwise.
            let (gutter, gutter_style) = if is_cursor {
                (
                    "▶ ",
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )
            } else if is_marked {
                ("● ", Style::default().fg(app.theme.accent))
            } else {
                ("  ", Style::default())
            };
            let mark = if e.ok { "✓" } else { "✗" };
            let mark_style = if e.ok {
                Style::default().fg(app.theme.success)
            } else {
                Style::default().fg(app.theme.error)
            };
            let mut spans = vec![
                Span::styled(gutter, gutter_style),
                Span::styled(format!("{mark} "), mark_style),
                Span::styled(e.cmd.clone(), Style::default().fg(app.theme.foreground)),
                Span::styled(
                    format!("  →  {}", e.detail),
                    Style::default().fg(app.theme.dim),
                ),
            ];
            if let Some(d) = e.duration {
                spans.push(Span::styled(
                    format!("  ({})", crate::domain::format_duration(d)),
                    Style::default().fg(app.theme.placeholder),
                ));
            }
            let mut line = Line::from(spans);
            if is_cursor {
                line = line.style(Style::default().bg(app.theme.selected_bg));
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Trims a ` · `-separated hint to the whole segments that fit `max` columns,
/// appending ` …` when some are dropped (the rest lives in F1). If even the
/// first segment is too wide it hard-trims with a trailing `…`.
fn fit_segments(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for seg in s.split(" · ") {
        let candidate = if out.is_empty() {
            seg.to_string()
        } else {
            format!("{out} · {seg}")
        };
        if candidate.chars().count() + 2 > max {
            break; // leave room for a trailing " …"
        }
        out = candidate;
    }
    if out.is_empty() {
        let mut head: String = s.chars().take(max.saturating_sub(1)).collect();
        head.push('…');
        return head;
    }
    out.push_str(" …");
    out
}

/// Renders the bottom status / feedback strip. While an action is in
/// flight (or just finished) it shows the spinner/✓/✗ message full
/// width; idle, it shows `footer_hint` on the left with `F1 help`
/// anchored right.
pub fn draw_status_strip(frame: &mut Frame, app: &App, full_area: Rect, footer_hint: &str) {
    use crate::tui::app::UiMode;
    // nvim-style mode badge on the far left — always visible, so the user knows
    // what a keystroke will do.
    let mode = app.ui_mode();
    let mode_color = match mode {
        UiMode::Normal => app.theme.accent,
        UiMode::Compose => app.theme.success,
        UiMode::Select => app.theme.conv_unread,
        UiMode::Search => app.theme.conv_dm,
    };
    let badge = format!("-- {} -- ", mode.label());
    let badge_w = (badge.chars().count() as u16).min(full_area.width);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            badge,
            Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
        ))),
        full_area,
    );
    // Everything else lives to the right of the badge.
    let area = Rect {
        x: full_area.x + badge_w,
        y: full_area.y,
        width: full_area.width.saturating_sub(badge_w),
        height: full_area.height,
    };
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

    const HELP_ANCHOR: &str = "F1 help · F10 settings";
    let anchor_block = HELP_ANCHOR.chars().count() + 2;
    let avail = (area.width as usize).saturating_sub(anchor_block);
    // Show only the hint segments that fully fit — the rest lives in F1 (don't
    // cut a keybinding in half).
    let hint = fit_segments(footer_hint, avail);
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
    fn fit_segments_keeps_whole_items() {
        let hint = "↑/↓ nav · Enter open · Alt+N new · Tab cycle";
        // Fits → unchanged.
        assert_eq!(fit_segments(hint, 100), hint);
        // Doesn't fit → only whole segments + a trailing " …", never a half item.
        let out = fit_segments(hint, 20);
        assert!(out.ends_with(" …"));
        assert!(!out.contains("Alt+N ne")); // no mid-item cut
        assert!(out.chars().count() <= 20);
    }

    #[test]
    fn list_title_formats() {
        assert_eq!(list_title("Inbox", 3, 10), "Inbox · 3 of 10");
        assert_eq!(list_title("Teams", 0, 0), "Teams · 0 of 0");
    }

    #[test]
    fn editor_lines_splits_rows_and_keeps_trailing_empty() {
        let t = Theme::default();
        // Two rows from one newline.
        assert_eq!(
            editor_lines(&LineEditor::from_text("hello\nworld"), &t).len(),
            2
        );
        // A trailing newline yields an extra (empty) row where the cursor sits.
        assert_eq!(editor_lines(&LineEditor::from_text("a\n"), &t).len(), 2);
        // No newline → a single row.
        assert_eq!(editor_lines(&LineEditor::from_text("solo"), &t).len(), 1);
    }

    #[test]
    fn middle_ellipsis_is_head_biased() {
        assert_eq!(middle_ellipsis("short", 10), "short");
        let out = middle_ellipsis("team.subteam.channel.very.long.name", 20);
        assert_eq!(out.chars().count(), 20);
        assert!(out.contains('…'));
    }

    #[test]
    fn trim_end_ellipsis_appends_and_is_utf8_safe() {
        assert_eq!(trim_end_ellipsis("hello", 10), "hello"); // fits, untouched
        assert_eq!(trim_end_ellipsis("hello world", 5), "hell…"); // 4 chars + …
        assert_eq!(trim_end_ellipsis("áéíóú", 3), "áé…"); // multibyte boundary safe
        assert_eq!(trim_end_ellipsis("toolong", 1), "…"); // degenerate width
    }

    #[test]
    fn tree_pane_width_shrinks_and_grows_clamped() {
        assert_eq!(tree_pane_width(70), 22); // narrow → floor
        assert_eq!(tree_pane_width(100), 28); // typical → the familiar 28
        assert_eq!(tree_pane_width(300), 40); // very wide → cap
        // Monotonic non-decreasing in width.
        let mut prev = 0;
        for w in (70..=300).step_by(7) {
            let v = tree_pane_width(w);
            assert!(v >= prev, "tree width must not shrink as width grows");
            prev = v;
        }
    }

    #[test]
    fn cmdlog_height_yields_to_body_without_starving_it() {
        assert_eq!(cmdlog_height(18), 3); // floor → smallest log
        assert_eq!(cmdlog_height(40), 6); // roomy → full log
        // The body (height − 4 fixed chrome − cmdlog) must never shrink as the
        // terminal grows — the regression a naive two-tier split would cause.
        let body = |h: u16| h.saturating_sub(4).saturating_sub(cmdlog_height(h));
        let mut prev = 0;
        for h in 18..=60 {
            let b = body(h);
            assert!(b >= prev, "body shrank at height {h}: {b} < {prev}");
            prev = b;
        }
    }

    #[test]
    fn col_width_clamps() {
        assert_eq!(col_width(&[0, 1], 4, 10, |_| 2), 4);
        assert_eq!(col_width(&[0, 1], 4, 10, |_| 7), 7);
        assert_eq!(col_width(&[0, 1], 4, 10, |_| 99), 10);
        assert_eq!(col_width(&[], 4, 10, |_| 99), 4);
    }
}
