//! Shared **right-click action menu** renderer — a compact centered picker of
//! action labels. One skeleton for every context menu (per-message,
//! per-conversation, …) so they all inherit click-to-run,
//! click-outside-to-dismiss and the modal grammar, and read identically.

use ratatui::{Frame, text::Line};

use crate::tui::theme::Theme;
use crate::tui::view::widgets::{PickerModal, PickerRow, draw_picker_modal};

/// Draws an action menu titled `title` listing `labels`, highlighting
/// `selected`. Display-only; the input/mouse layers own selection + dispatch.
pub fn draw(frame: &mut Frame, theme: &Theme, title: &str, labels: &[&str], selected: usize) {
    let rows: Vec<PickerRow> = labels
        .iter()
        .map(|l| PickerRow::Item(vec![Line::from((*l).to_string())]))
        .collect();
    draw_picker_modal(
        frame,
        theme,
        PickerModal {
            title: title.to_string(),
            query: None,
            selected: selected.min(labels.len().saturating_sub(1)),
            rows,
            empty: Vec::new(),
            legend: &[("↑/↓", "pick"), ("Enter", "do"), ("Esc", "close")],
            footer: None,
            scroll_target: None,
        },
    );
}
