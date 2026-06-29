//! Settings overlay renderer — a section sidebar plus the active section's
//! panel, centered over the originating screen. Identity is read-only; the
//! other sections expose editable rows (toggle / number / choice). Theme keeps
//! its preset list. New sections slot into the sidebar without changing the
//! layout. Mirrors jewel's Settings overlay layout.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::tui::App;
use crate::tui::app::{SettingKind, SettingsFocus, SettingsSection};
use crate::tui::theme;
use crate::tui::view::widgets::trim_end_ellipsis;

pub fn draw_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let t = &app.theme;
    let accent = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);

    let w = area.width.saturating_sub(6).clamp(50, 72);
    let h = area.height.saturating_sub(4).clamp(12, 20);
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
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(t.accent));
    let inner = outer.inner(popup);
    frame.render_widget(outer, popup);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(16), Constraint::Min(10)])
        .split(rows[0]);

    draw_sidebar(frame, app, cols[0]);
    draw_panel(frame, app, cols[1]);

    let hint = match app.settings_focus {
        SettingsFocus::Sidebar => "↑/↓ section · →/Enter open · Esc close",
        SettingsFocus::Panel => "↑/↓ row · ←/→ change · Tab sections · Esc close",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(t.dim)))),
        rows[1],
    );
}

/// One bordered block whose title + border go accent+bold when focused,
/// else inactive — the same focus affordance as the main screens.
fn focus_block(app: &App, title: &str, focused: bool) -> Block<'static> {
    let t = &app.theme;
    let color = if focused { t.accent } else { t.inactive };
    let mut title_style = Style::default().fg(color);
    if focused {
        title_style = title_style.add_modifier(Modifier::BOLD);
    }
    Block::default()
        .title(Span::styled(format!(" {title} "), title_style))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
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
                Style::default().fg(t.foreground)
            } else {
                Style::default().fg(t.dim)
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

/// Renders a section whose options are a list of labelled rows: the label on
/// the left, the current value on the right, and a hint under the focused row.
fn draw_rows_panel(frame: &mut Frame, app: &App, area: Rect, section: SettingsSection) {
    let t = &app.theme;
    let focused = app.settings_focus == SettingsFocus::Panel;
    let block = focus_block(app, section.label(), focused);
    let body = block.inner(area);
    frame.render_widget(block, area);

    let rows = section.rows();
    let item = app.settings_item.min(rows.len().saturating_sub(1));
    // Reserve a label column and trim an over-long value to what's left so it
    // never overflows the panel (e.g. the chafa symbol-set string).
    const LABEL_COL: usize = 20;
    let avail = (body.width as usize)
        .saturating_sub(2 + LABEL_COL + 1)
        .max(4);

    let mut lines: Vec<Line> = Vec::new();
    for (i, &id) in rows.iter().enumerate() {
        let selected = i == item;
        let marker = if selected { "▶ " } else { "  " };
        let label_style = if selected && focused {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default().fg(t.foreground)
        } else {
            Style::default().fg(t.dim)
        };
        let value = trim_end_ellipsis(&app.setting_value(id), avail);
        let value_style = match id.kind() {
            SettingKind::Info => Style::default().fg(t.dim),
            _ => Style::default().fg(if selected { t.accent } else { t.foreground }),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{:<LABEL_COL$} ", id.label()), label_style),
            Span::styled(value, value_style),
        ]));
    }
    lines.push(Line::from(""));
    if let Some(&id) = rows.get(item) {
        lines.push(Line::from(Span::styled(
            id.hint(),
            Style::default().fg(t.dim),
        )));
    }
    frame.render_widget(Paragraph::new(lines), body);
}

fn draw_theme_panel(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let focused = app.settings_focus == SettingsFocus::Panel;
    let block = focus_block(app, "Theme", focused);
    let body = block.inner(area);
    frame.render_widget(block, area);

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
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Applies live — saved to config.toml",
        Style::default().fg(t.dim),
    )));
    frame.render_widget(Paragraph::new(lines), body);
}
