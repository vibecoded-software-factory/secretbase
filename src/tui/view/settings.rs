//! Settings overlay renderer — a section sidebar plus the active
//! section's panel, centered over the originating screen. Today the only
//! section is Theme (a live-previewing preset picker); new sections slot
//! into the sidebar without changing the layout. Mirrors jewel's
//! Settings overlay verbatim.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::tui::App;
use crate::tui::app::{SettingsFocus, SettingsSection};
use crate::tui::theme;

pub fn draw_popup(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let t = &app.theme;
    let accent = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);

    let w = area.width.saturating_sub(6).clamp(50, 72);
    let h = area.height.saturating_sub(4).clamp(12, 18);
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
        SettingsFocus::Panel => "↑/↓ preview · Enter apply+save · ←/Tab back · Esc cancel",
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
    match SettingsSection::ALL[app.settings_section] {
        SettingsSection::Theme => draw_theme_panel(frame, app, area),
    }
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
        "Live preview — Enter saves to config.toml",
        Style::default().fg(t.dim),
    )));
    frame.render_widget(Paragraph::new(lines), body);
}
