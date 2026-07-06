//! Shared widgets — the single list/table renderer, identity bar,
//! command-log panel, status strip, search box, line-editor spans and
//! the y/n confirm overlay — the shared chrome every screen reuses.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
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

/// The Home shell's **section tab bar** as a title `Line` — `Messages · Teams ·
/// Find` with the active section (derived from the screen) in accent+bold and
/// the others dim. Woven into a right-pane panel's *top border* (the app's
/// title grammar), never a floating row. Used by every section renderer so the
/// whole right pane reads as one tabbed surface.
pub(crate) fn section_tabs_line(app: &App) -> Line<'static> {
    use crate::tui::screens::Screen;
    let t = &app.theme;
    let on_teams = matches!(app.screen, Screen::Teams | Screen::ChannelBrowser);
    let on_find = app.screen == Screen::Inbox && app.open_conv_id.is_none();
    let active = [!on_teams && !on_find, on_teams, on_find];
    let tab = |label: &str, active: bool| {
        let style = if active {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.dim)
        };
        Span::styled(format!(" {label} "), style)
    };
    let sep = || Span::styled("·", Style::default().fg(t.muted));
    Line::from(vec![
        Span::raw(" "),
        tab(SECTION_TAB_LABELS[0], active[0]),
        sep(),
        tab(SECTION_TAB_LABELS[1], active[1]),
        sep(),
        tab(SECTION_TAB_LABELS[2], active[2]),
    ])
}

/// The three section-tab labels, in border order. Both the rendered tabs
/// ([`section_tabs_line`]) and their clickable hit rects
/// ([`section_tab_rects`]) are driven by this array so the glyphs and the
/// mouse targets can never drift apart.
const SECTION_TAB_LABELS: [&str; 3] = ["Messages", "Teams", "Find"];

/// Screen rects of the three section tabs within a right-pane panel whose top
/// border carries [`section_tabs_line`]. The title starts one cell in from the
/// left corner; each tab is its label padded with a space on each side, the
/// tabs joined by single `·` separators after one leading space. Returns
/// `(messages, teams, find)` — the mouse layer hit-tests these to switch
/// sections.
pub(crate) fn section_tab_rects(area: Rect) -> (Rect, Rect, Rect) {
    let y = area.y;
    let mut x = area.x + 2; // left border corner + the leading raw space
    let mut rect_for = |label: &str| {
        let w = label.len() as u16 + 2; // the `" {label} "` padding
        let r = Rect {
            x,
            y,
            width: w,
            height: 1,
        };
        x += w + 1; // step past the label and its `·` separator
        r
    };
    (
        rect_for(SECTION_TAB_LABELS[0]),
        rect_for(SECTION_TAB_LABELS[1]),
        rect_for(SECTION_TAB_LABELS[2]),
    )
}

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
pub fn cmdlog_height(total: u16, cap: u64) -> u16 {
    if cap == 0 {
        return 0; // hidden by the `cmdlog_rows` setting
    }
    let responsive = if total >= 28 {
        6
    } else if total >= 22 {
        5
    } else if total >= 19 {
        4
    } else {
        3
    };
    responsive.min(cap.min(u16::MAX as u64) as u16)
}

/// One row of the help popup (key + description).
pub fn help_line<'a>(key: &'a str, desc: &'a str, t: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{key:<16}"), key_style(t)),
        Span::styled(desc, Style::default().fg(t.foreground)),
    ])
}

/// The one list/table renderer every list screen uses. Draws a bordered
/// block titled `title`, a dim header row, the rows with content columns,
/// the `▶ ` selection highlight, and persists the scroll offset through
/// `*scroll`.
///
/// `widths` must match `headers` in length. Size content columns to the
/// visible rows — never a stretching `Min` on a non-final content column.
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
    // Position cue on the right border when the list overflows — every
    // list_table surface (tree, Teams) gets it for free.
    let viewport = area.height.saturating_sub(3) as usize; // borders + header
    draw_scrollbar(frame, theme, area, len, viewport, *scroll);
}

/// helix-style **which-key** popup for an armed leader: a small
/// bottom-right overlay listing every continuation. Rendered per frame
/// while the leader is pending — the footer hint says the same thing in
/// one line, this makes it glanceable without reading.
pub fn draw_which_key(frame: &mut Frame, theme: &Theme, entries: &[(&str, &str)]) {
    let area = frame.area();
    let w = entries
        .iter()
        .map(|(k, l)| k.chars().count() + l.chars().count() + 3)
        .max()
        .unwrap_or(10)
        .max("Ctrl+W …".len()) as u16
        + 4;
    let h = entries.len() as u16 + 2;
    if area.width <= w + 2 || area.height <= h + 2 {
        return;
    }
    let rect = Rect {
        x: area.x + area.width - w - 1,
        y: area.y + area.height - h - 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    let block = rounded_block(Style::default().fg(theme.accent)).title(Span::styled(
        " Ctrl+W … ",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines: Vec<Line<'static>> = entries
        .iter()
        .map(|(k, l)| {
            Line::from(vec![
                Span::styled(format!(" {k} "), key_style(theme)),
                Span::styled((*l).to_string(), Style::default().fg(theme.dim)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A dim vertical scrollbar on `area`'s right border, drawn only when
/// `content_len` overflows `viewport` — the shared position cue for every
/// scrollable region (lists, pickers, the message history).
pub fn draw_scrollbar(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    content_len: usize,
    viewport: usize,
    offset: usize,
) {
    if content_len <= viewport || area.height <= 2 || area.width == 0 {
        return;
    }
    use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
    let mut state = ScrollbarState::new(content_len.saturating_sub(viewport))
        .position(offset.min(content_len.saturating_sub(viewport)))
        .viewport_content_length(viewport);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("┃")
            .track_style(Style::default().fg(theme.inactive))
            .thumb_style(Style::default().fg(theme.dim)),
        area.inner(ratatui::layout::Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
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

/// A focus/selection-highlighted action button rendered as `[ label ]` — the
/// single button span every screen reuses (the Login form's two buttons, the
/// confirm popup's confirm/cancel). `active` = focused / highlighted (accent on
/// `selected_bg`, bold); otherwise a recessive `dim`. A new button anywhere
/// must use this rather than hand-rolling a styled span.
pub fn button(label: &str, active: bool, theme: &Theme) -> Span<'static> {
    let style = if active {
        Style::default()
            .fg(theme.accent)
            .bg(theme.selected_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dim)
    };
    Span::styled(format!("[ {label} ]"), style)
}

/// The **keybind-letter** style — `accent` + BOLD. Every place that shows a
/// shortcut glyph (the `F1` help popup, the select-mode action bar, the command
/// palette) styles the key with this, so keys read identically everywhere and
/// match the gradient convention ("keybind letters = accent"). Labels/descriptions
/// keep their own per-context style (help = foreground, action bar = dim, …) —
/// only the key styling is unified.
pub fn key_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD)
}

/// The single focused/unfocused chrome style: accent + bold when focused,
/// the `inactive` tint otherwise. Every focus-styled border/label routes
/// here (`titled_block`, the Settings block, the Login inputs, the search
/// box) so "what focus looks like" is decided exactly once.
pub fn focus_style(theme: &Theme, focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.inactive)
    }
}

/// Builds a hint/legend line from `(key, label)` pairs: keys in
/// [`key_style`] (the "keybind letters = accent" rule), labels dim,
/// ` · `-separated — **fitted** to `width` by whole segments (the rest
/// lives in F1), so no hint can silently clip at the terminal edge. An
/// empty `key` renders a label-only segment.
pub fn legend_line<'a>(items: &[(&'a str, &'a str)], width: usize, theme: &Theme) -> Line<'static> {
    let sep_w = " · ".chars().count();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for (i, (key, label)) in items.iter().enumerate() {
        let seg_w = if key.is_empty() {
            label.chars().count()
        } else {
            key.chars().count() + 1 + label.chars().count()
        };
        let extra = if i == 0 { seg_w } else { sep_w + seg_w };
        // Reserve room for a trailing " …" if this wouldn't be the last one.
        let reserve = if i + 1 < items.len() { 2 } else { 0 };
        if used + extra + reserve > width && i > 0 {
            spans.push(Span::styled(
                " …".to_string(),
                Style::default().fg(theme.dim),
            ));
            return Line::from(spans);
        }
        if i > 0 {
            spans.push(Span::styled(
                " · ".to_string(),
                Style::default().fg(theme.muted),
            ));
        }
        if !key.is_empty() {
            spans.push(Span::styled((*key).to_string(), key_style(theme)));
            spans.push(Span::styled(
                format!(" {label}"),
                Style::default().fg(theme.dim),
            ));
        } else {
            spans.push(Span::styled(
                (*label).to_string(),
                Style::default().fg(theme.dim),
            ));
        }
        used += extra;
    }
    Line::from(spans)
}

/// A row of [`draw_picker_modal`]'s list: a selectable **item** (possibly
/// multi-line — e.g. a two-row search hit) or a fixed, non-selectable
/// section **header** (the switcher's DRAFTS/UNREAD/RECENT, the palette's
/// categories).
pub enum PickerRow {
    Item(Vec<Line<'static>>),
    Header(Line<'static>),
}

/// Parameters for [`draw_picker_modal`] — the **one** implementation of the
/// centered picker skeleton that eight overlays used to hand-roll (query +
/// windowed list + selection + legend), each with its own scroll math and
/// selection styling.
pub struct PickerModal<'a> {
    /// Block title (spaces added around it; rendered in [`Theme::emphasis`]).
    pub title: String,
    /// Query editor + its placeholder; `None` = browse-only modal.
    pub query: Option<(&'a LineEditor, &'a str)>,
    pub rows: Vec<PickerRow>,
    /// Index among the **Item** rows (headers aren't selectable).
    pub selected: usize,
    /// Body when `rows` is empty ([`empty_state_lines`] or a dim message).
    pub empty: Vec<Line<'static>>,
    /// Bottom legend, rendered through [`legend_line`] (fitted, centered).
    pub legend: &'a [(&'a str, &'a str)],
    /// When set, replaces the legend row — the inline **action row** the
    /// channel browser / members view swap in for their create/rename/add
    /// input modes and inline confirms ([`inline_input_line`] /
    /// [`inline_confirm_line`]).
    pub footer: Option<Line<'static>>,
    /// What the wheel scrolls when the pointer is over this picker. `None` for
    /// non-scrollable uses (e.g. the confirm popup). Registered by the skeleton
    /// itself, so every picker is wheel-scrollable for free.
    pub scroll_target: Option<ScrollTarget>,
}

/// Inner content width of the standard picker modal — for callers that
/// right-align within their rows (the palette's keybinding column).
pub fn modal_inner_width(frame: &Frame) -> usize {
    center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, frame.area())
        .width
        .saturating_sub(4) as usize // borders + the `▶ ` gutter
}

thread_local! {
    /// Frame-local hit map for the picker modal: the list viewport rect +
    /// one `Option<item index>` per visible display line (None = header /
    /// spill line of a multi-line item). Written by [`draw_picker_modal`]
    /// each frame, read by the mouse handler — the same frame-scoped
    /// pattern as the message-row map, without threading `&mut App` into
    /// an otherwise pure widget.
    static PICKER_HITS: std::cell::RefCell<(Rect, Vec<Option<usize>>)> =
        const { std::cell::RefCell::new((Rect { x: 0, y: 0, width: 0, height: 0 }, Vec::new())) };

    /// Frame-local **scroll registry**: every scrollable component records its
    /// viewport rect + logical [`ScrollTarget`] here as it draws, so the mouse
    /// wheel dispatches by pointer position — one generic path, no per-screen
    /// `match` in the input layer. Cleared each frame by [`reset_scroll_regions`].
    static SCROLL_REGIONS: std::cell::RefCell<Vec<(Rect, ScrollTarget)>> =
        const { std::cell::RefCell::new(Vec::new()) };

    /// Frame-local rect of the **active centered overlay** (picker / confirm /
    /// input / settings / help), if one is drawn. The mouse layer uses it for
    /// click-outside-to-dismiss — one generic close path for every modal.
    static MODAL_RECT: std::cell::RefCell<Option<Rect>> = const { std::cell::RefCell::new(None) };
}

/// What the mouse wheel moves when it's over a registered region. The widget
/// that draws a scrollable surface tags it with one of these; the input layer
/// owns the single table that maps a tag to the state it scrolls. Adding a new
/// scrollable list is one `register_scroll` call — the wheel handler never
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollTarget {
    /// The conversation tree (left pane).
    Tree,
    /// The command-log panel.
    CmdLog,
    /// The open conversation's message viewport.
    Messages,
    /// The help overlay's scroll offset.
    Help,
    /// The Teams section list.
    Teams,
    /// The Find landing's conversation list.
    Find,
    /// The channel-browser list.
    ChannelBrowser,
    /// The team-members list.
    Members,
    /// The reaction picker.
    React,
    /// The command palette.
    Palette,
    /// The quick switcher.
    Switcher,
    /// The in-conversation search results.
    ConvSearch,
    /// The global search results.
    GlobalSearch,
    /// The GIF search results.
    Giphy,
}

/// Clears the frame-local widget registries (scroll regions + the active modal
/// rect). Called once per frame before drawing, alongside
/// [`crate::tui::mouse_areas::MouseAreas::reset`].
pub fn reset_scroll_regions() {
    SCROLL_REGIONS.with(|s| s.borrow_mut().clear());
    MODAL_RECT.with(|m| *m.borrow_mut() = None);
}

/// Records the active centered-overlay rect for this frame. Every modal drawer
/// (`draw_picker_modal`, `draw_input_popup`, the settings/help popups) calls
/// this so the mouse layer can dismiss the overlay on a click outside it.
pub fn register_modal(rect: Rect) {
    MODAL_RECT.with(|m| *m.borrow_mut() = Some(rect));
}

/// The active centered-overlay rect, if one is drawn this frame.
pub fn active_modal_rect() -> Option<Rect> {
    MODAL_RECT.with(|m| *m.borrow())
}

/// Records a scrollable region for this frame. Overlays draw after the base
/// screen, so later registrations win on overlap (the modal captures the wheel).
pub fn register_scroll(rect: Rect, target: ScrollTarget) {
    if rect.width > 0 && rect.height > 0 {
        SCROLL_REGIONS.with(|s| s.borrow_mut().push((rect, target)));
    }
}

/// The scroll target under `(column, row)`, if any — the last-registered
/// (top-most) region that contains the point.
pub fn scroll_target_at(column: u16, row: u16) -> Option<ScrollTarget> {
    SCROLL_REGIONS.with(|s| {
        s.borrow()
            .iter()
            .rev()
            .find(|(r, _)| {
                column >= r.x && column < r.x + r.width && row >= r.y && row < r.y + r.height
            })
            .map(|(_, t)| *t)
    })
}

/// The selectable item under `(column, row)` in the last-drawn picker
/// modal, if any.
pub fn picker_row_at(column: u16, row: u16) -> Option<usize> {
    PICKER_HITS.with(|h| {
        let (rect, ref map) = *h.borrow();
        if rect.width == 0
            || column < rect.x
            || column >= rect.x + rect.width
            || row < rect.y
            || row >= rect.y + rect.height
        {
            return None;
        }
        map.get((row - rect.y) as usize).copied().flatten()
    })
}

/// Draws the standard centered picker modal: `Clear`, rounded accent block,
/// emphasized title, optional `⌕` query row (+spacer), a **windowed** list
/// that keeps the whole selected item visible, the shared `▶` + `selected_bg`
/// row treatment, and a width-fitted [`legend_line`] footer. Callers style
/// their content spans; the widget owns geometry, cursor, shading, windowing
/// and the footer grammar — so they can't drift apart again.
pub fn draw_picker_modal(frame: &mut Frame, theme: &Theme, m: PickerModal<'_>) {
    let area = center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, frame.area());
    frame.render_widget(Clear, area);
    register_modal(area); // click outside dismisses it
    draw_picker_into(frame, theme, area, true, m);
}

/// Renders the picker skeleton **into `area`** (no `Clear`) — the in-pane form,
/// for a picker that lives in a pane of the Home shell rather than floating as
/// an overlay. `focused` accents the border (accent when the pane holds focus,
/// else the unfocused `inactive` tint). [`draw_picker_modal`] is the
/// centered-overlay wrapper around this.
pub fn draw_picker_into(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    focused: bool,
    m: PickerModal<'_>,
) {
    let border = if focused {
        theme.accent
    } else {
        theme.inactive
    };
    let block = rounded_block(Style::default().fg(border)).title(Span::styled(
        format!(" {} ", m.title.trim()),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    draw_picker_body(frame, theme, area, inner, m);
}

/// The **tabbed** in-pane picker (a Home-shell section): the section tab bar
/// `tabs` woven into the top border as the title, the picker's own `title` as
/// the right-aligned detail — so the section reads as one bordered panel with
/// its tabs *in the border* (the app's grammar), never a floating row.
pub(crate) fn draw_picker_tabbed(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    focused: bool,
    tabs: Line<'static>,
    m: PickerModal<'_>,
) {
    let border = if focused {
        theme.accent
    } else {
        theme.inactive
    };
    let block = rounded_block(Style::default().fg(border))
        .title(tabs)
        .title(
            Line::from(Span::styled(
                format!(" {} ", m.title.trim()),
                Style::default().fg(theme.dim),
            ))
            .right_aligned(),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    draw_picker_body(frame, theme, area, inner, m);
}

/// A rounded panel [`Block`] carrying the `Messages · Teams · Find` section
/// tabs woven into its top border (left-aligned) with `detail` as the
/// right-aligned dim caption — the same border grammar as
/// [`draw_picker_tabbed`], but returned for panels that render their own body
/// (the conversation's messages pane). `focused` accents the border.
pub(crate) fn tabbed_block(app: &App, detail: &str, focused: bool) -> Block<'static> {
    let border = if focused {
        app.theme.accent
    } else {
        app.theme.inactive
    };
    let mut block = rounded_block(Style::default().fg(border)).title(section_tabs_line(app));
    let detail = detail.trim();
    if !detail.is_empty() {
        block = block.title(
            Line::from(Span::styled(
                format!(" {detail} "),
                Style::default().fg(app.theme.dim),
            ))
            .right_aligned(),
        );
    }
    block
}

/// The picker body — query row + windowed list + scrollbar + legend/footer —
/// rendered into `inner`, with the scrollbar hugging `area`'s right border.
/// Shared by the modal, plain in-pane and tabbed forms.
fn draw_picker_body(frame: &mut Frame, theme: &Theme, area: Rect, inner: Rect, m: PickerModal<'_>) {
    // The wheel scrolls this picker anywhere inside its panel.
    if let Some(target) = m.scroll_target {
        register_scroll(area, target);
    }
    let has_query = m.query.is_some();
    let chunks = Layout::vertical([
        Constraint::Length(if has_query { 1 } else { 0 }), // query
        Constraint::Length(if has_query { 1 } else { 0 }), // spacer
        Constraint::Min(1),                                // list
        Constraint::Length(1),                             // legend
    ])
    .split(inner);

    if let Some((editor, placeholder)) = m.query {
        let line = if editor.is_empty() {
            Line::from(vec![
                Span::styled("⌕ ", Style::default().fg(theme.accent)),
                Span::styled(
                    placeholder.to_string(),
                    Style::default().fg(theme.placeholder),
                ),
            ])
        } else {
            let mut spans = vec![Span::styled("⌕ ", Style::default().fg(theme.accent))];
            spans.extend(editor_spans(editor, true, theme));
            Line::from(spans)
        };
        frame.render_widget(Paragraph::new(line), chunks[0]);
    }

    let vh = chunks[2].height.max(1) as usize;
    if m.rows.is_empty() {
        frame.render_widget(Paragraph::new(m.empty), chunks[2]);
        PICKER_HITS.with(|h| *h.borrow_mut() = (chunks[2], Vec::new()));
    } else {
        let mut display: Vec<Line<'static>> = Vec::new();
        // Parallel to `display`: which selectable item each line belongs to.
        let mut line_items: Vec<Option<usize>> = Vec::new();
        let mut sel_start = 0usize;
        let mut sel_len = 1usize;
        let mut item_i = 0usize;
        for row in m.rows {
            match row {
                PickerRow::Header(l) => {
                    display.push(l);
                    line_items.push(None);
                }
                PickerRow::Item(ls) => {
                    let selected = item_i == m.selected;
                    if selected {
                        sel_start = display.len();
                        sel_len = ls.len().max(1);
                    }
                    for (li, l) in ls.into_iter().enumerate() {
                        let prefix = if li == 0 && selected { "▶ " } else { "  " };
                        let mut spans = vec![Span::styled(prefix.to_string(), theme.emphasis())];
                        spans.extend(l.spans);
                        let mut l = Line::from(spans);
                        if selected {
                            for s in l.spans.iter_mut() {
                                s.style = s.style.bg(theme.selected_bg);
                            }
                        }
                        display.push(l);
                        line_items.push(Some(item_i));
                    }
                    item_i += 1;
                }
            }
        }
        // Keep every line of the selected item inside the viewport.
        let sel_end = sel_start + sel_len;
        let scroll = sel_end.saturating_sub(vh);
        let visible: Vec<Line<'static>> = display.into_iter().skip(scroll).take(vh).collect();
        let total = line_items.len();
        let visible_items: Vec<Option<usize>> =
            line_items.into_iter().skip(scroll).take(vh).collect();
        frame.render_widget(Paragraph::new(visible), chunks[2]);
        PICKER_HITS.with(|h| *h.borrow_mut() = (chunks[2], visible_items));
        // The picker's list lives inside the modal block: hug its right
        // edge (area is the bordered modal; chunks[2] is the inner list).
        draw_scrollbar(
            frame,
            theme,
            Rect {
                x: area.x,
                y: chunks[2].y.saturating_sub(1),
                width: area.width,
                height: chunks[2].height + 2,
            },
            total,
            vh,
            scroll,
        );
    }

    match m.footer {
        Some(line) => frame.render_widget(Paragraph::new(line), chunks[3]),
        None => frame.render_widget(
            Paragraph::new(legend_line(m.legend, chunks[3].width as usize, theme))
                .alignment(Alignment::Center),
            chunks[3],
        ),
    }
}

/// The inline bottom-row **input mode** (channel create/rename, member add):
/// dim label + the editor + a muted `(Enter <verb> · Esc cancel)` hint.
pub fn inline_input_line(
    label: &str,
    editor: &LineEditor,
    verb: &str,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!(" {label}"),
        Style::default().fg(theme.dim),
    )];
    spans.extend(editor_spans(editor, true, theme));
    spans.push(Span::styled(
        format!("   (Enter {verb} · Esc cancel)"),
        Style::default().fg(theme.muted),
    ));
    Line::from(spans)
}

/// The inline bottom-row **confirm**: danger prompt + optional dim note +
/// the shared [`button`] pair (default highlight = cancel) + the one
/// canonical hint — including `y`/`n`, which [`confirm_key`] has always
/// accepted (the popup and the inline copies used to disagree about
/// advertising it).
///
/// [`confirm_key`]: crate::tui::input::common::confirm_key
pub fn inline_confirm_line(
    prompt: &str,
    note: &str,
    confirm_verb: &str,
    yes: bool,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!(" {prompt} "),
        Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
    )];
    if !note.is_empty() {
        spans.push(Span::styled(
            format!("{note}  "),
            Style::default().fg(theme.dim),
        ));
    }
    spans.push(button(confirm_verb, yes, theme));
    spans.push(Span::raw(" "));
    spans.push(button("cancel", !yes, theme));
    spans.push(Span::styled(
        "   (←/→ · Enter · y/n · Esc)".to_string(),
        Style::default().fg(theme.muted),
    ));
    Line::from(spans)
}

/// A small centered **single-input popup** (new conversation, unhide):
/// emphasized title, a labeled editor row, and a fitted legend — the same
/// popup twice was the whole pattern, now parameterized.
pub fn draw_input_popup(
    frame: &mut Frame,
    theme: &Theme,
    title: &str,
    label: &str,
    editor: &LineEditor,
    legend: &[(&str, &str)],
) {
    let area = center_rect(60, 7, frame.area());
    frame.render_widget(Clear, area);
    register_modal(area); // click outside dismisses it
    let mut field = vec![Span::styled(
        format!("  {label}"),
        Style::default().fg(theme.dim),
    )];
    field.extend(editor_spans(editor, true, theme));
    let lines = vec![
        Line::from(Span::styled(format!(" {title} "), theme.emphasis()))
            .alignment(Alignment::Center),
        Line::from(""),
        Line::from(field),
        Line::from(""),
        legend_line(legend, area.width.saturating_sub(2) as usize, theme)
            .alignment(Alignment::Center),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(rounded_block(Style::default().fg(theme.accent))),
        area,
    );
}

/// The shared empty-state body: a bold headline + dim hint lines, indented
/// two spaces — the pattern the inbox tree notices established. Place it in
/// a panel or a modal body; the caller owns the surrounding block.
pub fn empty_state_lines(head: &str, hints: &[&str], theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  {head}"),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    for h in hints {
        lines.push(Line::from(Span::styled(
            format!("  {h}"),
            Style::default().fg(theme.dim),
        )));
    }
    lines
}

/// The shared **unread / attention** emphasis style: the golden `conv_unread`
/// colour plus BOLD. Every unread affordance uses it (the dot, the count, the
/// favourite star, the identity bar's "N unread"), so the emphasis reads
/// identically everywhere instead of being copy-pasted per call site.
pub fn unread_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.conv_unread)
        .add_modifier(Modifier::BOLD)
}

/// The unread `●` dot span — no surrounding spacing (callers pad as they need).
pub fn unread_dot(theme: &Theme) -> Span<'static> {
    Span::styled("●", unread_style(theme))
}

/// The `★` favourite-marker span (our local-only favourite; see
/// `App::favorites`). Same emphasis as the unread dot.
pub fn favorite_star(theme: &Theme) -> Span<'static> {
    Span::styled("★", unread_style(theme))
}

/// The shared y/n confirmation overlay — **the picker-modal skeleton**,
/// not a bespoke strip: same geometry (`MODAL_*` band, full height), same
/// title treatment, same bottom action row (`inline_confirm_line` — the
/// exact confirm the channel browser / members footers show). The caller's
/// `body` lines render as non-selectable content; the highlighted button
/// follows `confirmed`. One confirm look everywhere, popup or inline.
pub fn draw_confirm_popup(
    frame: &mut Frame,
    theme: &Theme,
    title: &str,
    body: Vec<Line<'static>>,
    confirmed: bool,
) {
    let rows: Vec<PickerRow> = body.into_iter().map(PickerRow::Header).collect();
    draw_picker_modal(
        frame,
        theme,
        PickerModal {
            title: title.trim().to_string(),
            query: None,
            selected: 0,
            rows,
            empty: Vec::new(),
            legend: &[],
            footer: Some(inline_confirm_line("", "", "confirm", confirmed, theme)),
            scroll_target: None,
        },
    );
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

/// Like [`editor_spans`] but renders every character as `●` — for a secret
/// field (the Login paper key) shown masked unless the user reveals it. The
/// block cursor still tracks the real cursor position so editing feels normal.
pub fn editor_spans_masked(
    editor: &LineEditor,
    focused: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let text = editor.text();
    let total = text.chars().count();
    let base = Style::default().fg(theme.foreground);
    if !focused {
        return vec![Span::styled("●".repeat(total), base)];
    }
    let cursor = Style::default().add_modifier(Modifier::REVERSED);
    let cur_byte = editor.cursor().min(text.len());
    let cur = text[..cur_byte].chars().count(); // cursor as a char index
    let before = "●".repeat(cur);
    if cur >= total {
        return vec![Span::styled(before, base), Span::styled(" ", cursor)];
    }
    vec![
        Span::styled(before, base),
        Span::styled("●".to_string(), cursor),
        Span::styled("●".repeat(total - cur - 1), base),
    ]
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
/// `app.cmdlog.scroll` against the real overflow and writes it back, so
/// the input handler can use a `usize::MAX` "jump to oldest" sentinel
/// without the title showing a 20-digit number or the scroll getting
/// stuck above the bottom.
pub fn draw_cmd_log(frame: &mut Frame, app: &mut App, area: Rect, focused: bool, tag: &str) {
    register_scroll(area, ScrollTarget::CmdLog);
    // Inner height = block area minus the two borders.
    let visible_rows = (area.height as usize).saturating_sub(2);
    let total = app.cmdlog.entries.len();

    // When focused, the window follows the visual-select cursor (so you can
    // scroll the whole history); otherwise it stays pinned to the newest.
    let cursor = app.cmdlog.cursor.min(total.saturating_sub(1));
    let (start, end) = if focused && total > visible_rows {
        let end = (cursor + 1).max(visible_rows).min(total);
        (end - visible_rows, end)
    } else {
        (total.saturating_sub(visible_rows), total)
    };
    app.cmdlog.scroll = total - end; // keep the field consistent for clicks

    // Title: show the cursor position + selection while focused.
    let pos = if focused && total > 0 {
        let marks = app.cmdlog.marks.len();
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
    // An empty tag means the panel has no go-to combo on this screen (e.g.
    // Teams, where the log is display-only) — a tag must never lie.
    let title = if tag.is_empty() {
        format!("Command log{pos}")
    } else {
        format!("─[{tag}]-Command log{pos}")
    };
    let block = titled_block(&title, focused, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if visible_rows == 0 {
        return;
    }
    if app.cmdlog.entries.is_empty() {
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
            let e = &app.cmdlog.entries[i];
            let is_cursor = focused && i == cursor;
            let is_marked = app.cmdlog.marks.contains(&i);
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
                    // dim, not placeholder — the duration is data the user
                    // reads, and there are only two tiers on this line.
                    Style::default().fg(app.theme.dim),
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
    // Condition badges — states that are *true*, not events: they live next
    // to the mode badge and persist for as long as the condition does.
    let mut cond = String::new();
    if app.worker_dead {
        cond.push_str("⚠ WORKER DEAD ");
    }
    if app.listener_down {
        cond.push_str("⇅ reconnecting… ");
    }
    let cond_w = (cond.chars().count() as u16).min(full_area.width.saturating_sub(badge_w));
    if !cond.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                cond,
                if app.worker_dead {
                    Style::default()
                        .fg(app.theme.error)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(app.theme.dim)
                },
            ))),
            Rect {
                x: full_area.x + badge_w,
                y: full_area.y,
                width: cond_w,
                height: full_area.height,
            },
        );
    }
    // Attention hotlist — @N conversations with an unseen mention of you
    // (danger, the strongest pull), ●N with effective unread (dim). States,
    // not events: they persist until the conversations are opened. Drain
    // with Ctrl+N (priority-ordered: mentions → DMs → channels).
    let mut hot: Vec<Span<'static>> = Vec::new();
    let mention_n = app.mentioned.len();
    let unread_n = app
        .conversations
        .iter()
        .filter(|c| c.member_status == crate::domain::MemberStatus::Active)
        .filter(|c| app.conv_is_unread(c))
        .count();
    if mention_n > 0 {
        hot.push(Span::styled(
            format!("@{mention_n} "),
            app.theme.danger_title(),
        ));
    }
    if unread_n > 0 {
        hot.push(Span::styled(
            format!("●{unread_n} "),
            Style::default().fg(app.theme.dim),
        ));
    }
    let hot_w = (hot.iter().map(|s| s.content.chars().count()).sum::<usize>() as u16)
        .min(full_area.width.saturating_sub(badge_w + cond_w));
    if !hot.is_empty() && hot_w > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(hot)),
            Rect {
                x: full_area.x + badge_w + cond_w,
                y: full_area.y,
                width: hot_w,
                height: full_area.height,
            },
        );
    }
    // Everything else lives to the right of the badge(s).
    let area = Rect {
        x: full_area.x + badge_w + cond_w + hot_w,
        y: full_area.y,
        width: full_area.width.saturating_sub(badge_w + cond_w + hot_w),
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
        // The ✗ prefix separates a sticky error from the mode badge it sits
        // next to — "-- NORMAL -- EOF" read as one cryptic token without it.
        ActionState::Error(msg) => Some((format!("✗ {msg}"), Style::default().fg(app.theme.error))),
    };

    if let Some((text, style)) = feedback {
        // Fit-or-degrade like every other text surface: a long message ends
        // in `…`, never a mid-word hard cut.
        let trimmed = trim_end_ellipsis(&text, area.width as usize);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(trimmed, style))),
            area,
        );
        return;
    }

    // Right side: just the help anchor. The signed-in `@username` now lives in
    // the identity chip atop the conversation tree, not the footer.
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

/// A minimal bottom hint bar — `footer_hint` fit to the width (whole `·`
/// segments only, same rule as [`draw_status_strip`]) on the left, an `F1 help`
/// anchor on the right. For signed-out / no-mode screens (the Login form),
/// where the mode badge and action feedback of `draw_status_strip` don't apply.
pub fn draw_hint_bar(frame: &mut Frame, area: Rect, footer_hint: &str, t: &Theme) {
    const HELP_ANCHOR: &str = "F1 help";
    let anchor_block = HELP_ANCHOR.chars().count() + 2;
    let avail = (area.width as usize).saturating_sub(anchor_block);
    let hint = fit_segments(footer_hint, avail);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(t.dim)))),
        area,
    );
    frame.render_widget(
        Paragraph::new(
            Line::from(Span::styled(
                HELP_ANCHOR,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    fn line_text(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn legend_line_keeps_whole_segments_and_ellipsizes() {
        let t = Theme::default();
        let items = [("Enter", "open"), ("n", "new"), ("x", "delete")];
        // Plenty of room: everything present, keys + labels + separators.
        let full = line_text(&legend_line(&items, 80, &t));
        assert_eq!(full, "Enter open · n new · x delete");
        // Tight: whole segments only, then an ellipsis — never a clipped key.
        let tight = line_text(&legend_line(&items, 20, &t));
        assert_eq!(tight, "Enter open · n new …");
        // Even tighter: the ellipsis reservation drops the second segment
        // whole rather than clipping it.
        let tighter = line_text(&legend_line(&items, 18, &t));
        assert_eq!(tighter, "Enter open …");
        // Key spans carry the accent emphasis (the gradient rule).
        let l = legend_line(&items, 80, &t);
        assert_eq!(l.spans[0].style, key_style(&t));
    }

    #[test]
    fn legend_line_label_only_segments() {
        let t = Theme::default();
        let l = line_text(&legend_line(&[("", "(read-only)")], 40, &t));
        assert_eq!(l, "(read-only)");
    }

    #[test]
    fn empty_state_lines_shape() {
        let t = Theme::default();
        let lines = empty_state_lines("No chats", &["n to start one"], &t);
        assert_eq!(lines.len(), 3); // spacer + head + one hint
        assert_eq!(line_text(&lines[1]), "  No chats");
        assert_eq!(line_text(&lines[2]), "  n to start one");
    }

    #[test]
    fn scroll_registry_dispatches_by_position_top_most_wins() {
        use super::{ScrollTarget, register_scroll, reset_scroll_regions, scroll_target_at};
        reset_scroll_regions();
        // Base region, then a smaller overlapping one registered later (as an
        // overlay would draw over the base).
        register_scroll(
            Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 5,
            },
            ScrollTarget::Tree,
        );
        register_scroll(
            Rect {
                x: 2,
                y: 1,
                width: 4,
                height: 2,
            },
            ScrollTarget::React,
        );
        // Only the base covers this point.
        assert_eq!(scroll_target_at(0, 0), Some(ScrollTarget::Tree));
        // Overlap → the later (top-most) registration wins.
        assert_eq!(scroll_target_at(3, 1), Some(ScrollTarget::React));
        // Outside every region.
        assert_eq!(scroll_target_at(50, 50), None);
        // Empty rects are never registered.
        register_scroll(
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 5,
            },
            ScrollTarget::Help,
        );
        assert_eq!(scroll_target_at(0, 0), Some(ScrollTarget::Tree));
        // Reset clears the registry.
        reset_scroll_regions();
        assert_eq!(scroll_target_at(0, 0), None);
    }

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
        assert_eq!(cmdlog_height(18, 6), 3); // floor → smallest log
        assert_eq!(cmdlog_height(40, 6), 6); // roomy → full log
        // The user's cap wins under the responsive size; 0 hides the panel.
        assert_eq!(cmdlog_height(40, 4), 4);
        assert_eq!(cmdlog_height(40, 0), 0);
        // The body (height − 4 fixed chrome − cmdlog) must never shrink as the
        // terminal grows — the regression a naive two-tier split would cause.
        let body = |h: u16| h.saturating_sub(4).saturating_sub(cmdlog_height(h, 6));
        let mut prev = 0;
        for h in 18..=60 {
            let b = body(h);
            assert!(b >= prev, "body shrank at height {h}: {b} < {prev}");
            prev = b;
        }
    }

    #[test]
    fn section_tab_rects_line_up_with_the_rendered_tabs() {
        // The rects must fall exactly on the glyphs `section_tabs_line` paints:
        // the title starts after the border corner + one leading space, each tab
        // is `" {label} "` (label width + 2) and the tabs are `·`-separated.
        let area = Rect {
            x: 10,
            y: 5,
            width: 90,
            height: 20,
        };
        let (m, t, f) = section_tab_rects(area);
        // corner (10) + leading raw space (11) → the Messages tab starts at 12.
        assert_eq!((m.x, m.width), (12, 10)); // " Messages "
        assert_eq!((t.x, t.width), (23, 7)); //  after "·" → " Teams "
        assert_eq!((f.x, f.width), (31, 6)); //  after "·" → " Find "
        for r in [m, t, f] {
            assert_eq!((r.y, r.height), (5, 1));
        }
    }
}
