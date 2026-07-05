//! Command palette overlay (`Ctrl+P` / `:`) — a query box over a
//! context-aware, categorized list of actions, each with its keybinding
//! right-aligned. Rendered on the shared [`draw_picker_modal`] skeleton.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::App;
use crate::tui::flows::palette::{PaletteRow, palette_rows};
use crate::tui::view::widgets::{
    PickerModal, PickerRow, draw_picker_modal, key_style, modal_inner_width,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let t = &app.theme;
    let width = modal_inner_width(frame);
    let model = palette_rows(app);
    let mut cmd_idx = 0usize;
    let rows: Vec<PickerRow> = model
        .iter()
        .map(|row| match row {
            PaletteRow::Header(cat) => PickerRow::Header(Line::from(Span::styled(
                (*cat).to_string(),
                Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
            ))),
            PaletteRow::Cmd(c) => {
                let selected = cmd_idx == app.palette_selected;
                cmd_idx += 1;
                let label_style = if selected {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.foreground)
                };
                // Right-align the keybinding within the row width.
                let gap = width
                    .saturating_sub(c.label.chars().count() + c.keys.chars().count())
                    .max(1);
                PickerRow::Item(vec![Line::from(vec![
                    Span::styled(c.label.to_string(), label_style),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(c.keys.to_string(), key_style(t)),
                ])])
            }
        })
        .collect();

    draw_picker_modal(
        frame,
        t,
        PickerModal {
            title: format!(
                "Command palette · {}",
                crate::tui::flows::palette::filtered_commands(app).len()
            ),
            query: Some((&app.palette, "type a command…")),
            selected: app.palette_selected,
            rows,
            empty: vec![Line::from(Span::styled(
                "  no matching command",
                Style::default().fg(t.dim),
            ))],
            legend: &[("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")],
            footer: None,
        },
    );
}
