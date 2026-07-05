//! Settings overlay renderer — a section sidebar plus the active section's
//! panel, centered over the originating screen. Identity is read-only; the
//! other sections expose editable rows (toggle / number / choice). Theme keeps
//! its preset list. New sections slot into the sidebar without changing the
//! layout.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::tui::App;
use crate::tui::app::{SettingKind, SettingsFocus, SettingsSection};
use crate::tui::theme;
use crate::tui::view::widgets::MODAL_HEIGHT;

/// Footer hint shown while the section sidebar holds focus.
const HINT_SIDEBAR: &str = "↑/↓ section · →/Enter open · Esc close";
/// Footer hint shown while the active section's panel holds focus.
const HINT_PANEL: &str = "↑/↓ row · ←/→ change · Esc/Tab sections";
/// Gap (in spaces) between a row's label column and its value.
const LABEL_GAP: usize = 2;
/// Cap on the value column width when sizing the box. Longer values **wrap**
/// onto continuation lines (never truncated) instead of widening the popup.
const VALUE_CAP: usize = 34;

pub fn draw_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let t = &app.theme;
    let accent = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);

    // Height + position follow the standard modal geometry (centered,
    // `MODAL_HEIGHT` — same as global search / the quick switcher); the width is
    // content-driven (compact, stable). Clamped to the terminal (responsive).
    let (w, h, sidebar_w) = popup_dims(app, area);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, popup);
    let outer = Block::default()
        .title(Span::styled(" Settings ", accent))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.accent));
    let inner = outer.inner(popup);
    frame.render_widget(outer, popup);
    // A column of breathing room each side so the Sections / panel blocks don't
    // glue to the outer frame. (`popup_dims` reserves the extra width.)
    let inner = Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_w), Constraint::Min(10)])
        .split(rows[0]);

    draw_sidebar(frame, app, cols[0]);
    draw_panel(frame, app, cols[1]);

    let hint = match app.settings_focus {
        SettingsFocus::Sidebar => HINT_SIDEBAR,
        SettingsFocus::Panel => HINT_PANEL,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(t.dim)))),
        rows[1],
    );
}

/// The widest content line a section's panel needs: the widest of a value row
/// (per-section label + gap + value, value capped) or the focused-row hint.
fn section_width(app: &App, section: SettingsSection) -> usize {
    match section {
        SettingsSection::Theme => {
            let preset_w = theme::Preset::ALL
                .iter()
                .map(|p| p.label().chars().count() + 4) // "  ▶ " prefix
                .max()
                .unwrap_or(8);
            let note = "Applies live — saved to config.toml".chars().count();
            preset_w.max(note)
        }
        section => {
            let rows = section.rows();
            // Per-section label column (compact — sized to *this* section's
            // longest label, not a global fixed column).
            let label_w = rows
                .iter()
                .map(|id| id.label().chars().count())
                .max()
                .unwrap_or(0);
            // Value column target; longer values wrap rather than widen the box.
            let value_w = rows
                .iter()
                .map(|&id| app.setting_value(id).chars().count())
                .max()
                .unwrap_or(0)
                .min(VALUE_CAP);
            let hint_w = rows
                .iter()
                .map(|&id| id.hint().chars().count())
                .max()
                .unwrap_or(0);
            // marker (2) + label + gap + value, or the wider focused-row hint.
            (2 + label_w + LABEL_GAP + value_w).max(hint_w)
        }
    }
}

/// Popup dimensions, returned as `(width, height, sidebar_width)`.
///
/// **Height + position follow the standard modal geometry** (`MODAL_HEIGHT`,
/// vertically centered — the same as global search / the quick switcher), so
/// overlays line up. The **width stays content-driven**: sized once to the
/// widest section (compact per-section label columns, never resizing as you
/// navigate), clamped to the terminal.
fn popup_dims(app: &App, area: Rect) -> (u16, u16, u16) {
    // Sidebar: marker (2) + longest label + block borders (2).
    let sidebar_label = SettingsSection::ALL
        .iter()
        .map(|s| s.label().chars().count())
        .max()
        .unwrap_or(8);
    let sidebar_w = sidebar_label + 4;

    // Worst-case panel width across every section → one stable width for all.
    let max_content = SettingsSection::ALL
        .iter()
        .map(|&s| section_width(app, s))
        .max()
        .unwrap_or(20);

    // Panel block adds its own borders (2).
    let panel_w = max_content + 2;
    let footer_w = HINT_SIDEBAR.chars().count().max(HINT_PANEL.chars().count());
    let inner_w = (sidebar_w + panel_w).max(footer_w);

    // + outer double border (2) + a column of horizontal padding each side (2).
    let want_w = (inner_w + 4) as u16;
    let w = want_w.clamp(40, area.width.saturating_sub(2));
    let h = MODAL_HEIGHT.min(area.height.saturating_sub(2));
    (w, h, sidebar_w as u16)
}

/// Splits `s` into chunks of at most `width` characters (UTF-8 safe), so an
/// over-long value **wraps** onto continuation lines instead of being trimmed.
/// Always returns at least one chunk.
fn wrap_chars(s: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if width == 0 || chars.len() <= width {
        return vec![s.to_string()];
    }
    chars.chunks(width).map(|c| c.iter().collect()).collect()
}

/// One bordered block whose title + border go accent+bold when focused,
/// else inactive — the same focus affordance as the main screens.
fn focus_block(app: &App, title: &str, focused: bool) -> Block<'static> {
    let style = crate::tui::view::widgets::focus_style(&app.theme, focused);
    Block::default()
        .title(Span::styled(format!(" {title} "), style))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        // The border keeps the colour but not the bold (a full bold border
        // reads heavier than the shared chrome).
        .border_style(Style::default().fg(style.fg.unwrap_or(app.theme.inactive)))
}

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = app.settings_focus == SettingsFocus::Sidebar;
    let block = focus_block(app, "Sections", focused);
    let body = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = SettingsSection::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let selected = i == app.settings_section;
            let marker = if selected { "▶ " } else { "  " };
            let style = if selected && focused {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else if selected {
                Style::default()
                    .fg(t.foreground)
                    .add_modifier(Modifier::BOLD)
            } else {
                // Unselected sections are navigable items, not secondary text —
                // keep them readable (foreground), let the marker + accent mark
                // the selection. (lazygit: list rows are normal text.)
                Style::default().fg(t.foreground)
            };
            Line::from(Span::styled(format!("{marker}{}", s.label()), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);
}

fn draw_panel(frame: &mut Frame, app: &App, area: Rect) {
    match app.settings_section_obj() {
        SettingsSection::Theme => draw_theme_panel(frame, app, area),
        section => draw_rows_panel(frame, app, area, section),
    }
}

/// Splits a panel body into `(content, hint)` rects — the hint is **pinned to
/// the bottom row**, the content fills everything above it. Shared by both
/// panel renderers so the description always sits at the bottom, never floating
/// in the middle.
fn panel_split(body: Rect) -> (Rect, Rect) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(body);
    (parts[0], parts[1])
}

/// Renders a section whose options are a list of labelled rows: a per-section
/// label column, the value beside it (wrapping onto continuation lines when it
/// doesn't fit — never truncated), and the focused row's hint pinned to the
/// bottom.
fn draw_rows_panel(frame: &mut Frame, app: &App, area: Rect, section: SettingsSection) {
    let t = &app.theme;
    let focused = app.settings_focus == SettingsFocus::Panel;
    let block = focus_block(app, section.label(), focused);
    let body = block.inner(area);
    frame.render_widget(block, area);
    let (content_area, hint_area) = panel_split(body);

    let rows = section.rows();
    let item = app.settings_item.min(rows.len().saturating_sub(1));
    // Compact, per-section label column (sized to this section's longest label).
    let label_w = rows
        .iter()
        .map(|id| id.label().chars().count())
        .max()
        .unwrap_or(0);
    let indent = 2 + label_w + LABEL_GAP; // marker + label + gap
    let value_avail = (content_area.width as usize).saturating_sub(indent).max(4);

    let mut lines: Vec<Line> = Vec::new();
    for (i, &id) in rows.iter().enumerate() {
        let selected = i == item;
        let marker = if selected { "▶ " } else { "  " };
        let label_style = if selected && focused {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default()
                .fg(t.foreground)
                .add_modifier(Modifier::BOLD)
        } else {
            // Row labels are readable content, not secondary text.
            Style::default().fg(t.foreground)
        };
        let value_style = match id.kind() {
            // Read-only identity values are still data you want to read — keep
            // them at foreground; the bottom "read-only" hint signals they
            // can't be edited.
            SettingKind::Info => Style::default().fg(t.foreground),
            _ => Style::default().fg(if selected { t.accent } else { t.foreground }),
        };
        let label = id.label();
        let gap = " ".repeat(LABEL_GAP);
        let chunks = wrap_chars(&app.setting_value(id), value_avail);
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{label:<label_w$}{gap}"), label_style),
            Span::styled(chunks[0].clone(), value_style),
        ]));
        // Continuation lines align under the value column.
        for cont in chunks.iter().skip(1) {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(cont.clone(), value_style),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), content_area);

    if let Some(&id) = rows.get(item) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                id.hint(),
                Style::default().fg(t.dim),
            ))),
            hint_area,
        );
    }
}

fn draw_theme_panel(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = app.settings_focus == SettingsFocus::Panel;
    let block = focus_block(app, "Theme", focused);
    let body = block.inner(area);
    frame.render_widget(block, area);
    let (content_area, hint_area) = panel_split(body);

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "Preset",
        Style::default().fg(t.dim),
    ))];
    for (i, p) in theme::Preset::ALL.iter().enumerate() {
        let selected = i == app.settings_theme_idx;
        let marker = if selected { "▶ " } else { "  " };
        let style = if selected {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.foreground)
        };
        lines.push(Line::from(Span::styled(
            format!("  {marker}{}", p.label()),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), content_area);

    // The note is pinned to the bottom row, like the rows panel's hint.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Applies live — saved to config.toml",
            Style::default().fg(t.dim),
        ))),
        hint_area,
    );
}
