//! Decorative starfield used by the splash and login screens.
//!
//! A restrained night-sky pattern: ~98% empty space with a sprinkle of
//! dim/mid/bright pinpricks. The mapping is deterministic per
//! `(row, col)` so the field is stable across re-renders and resizes —
//! no shimmer, no caching. This is the only decoration in the app; the
//! working screens carry no background art.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::theme::Theme;

/// Returns a deterministic `(char, color)` pair for a given screen
/// cell.
///
/// Avalanche bit-mixing combines `row` and `col` so the pattern doesn't
/// cluster on large terminals. Density: ~0.3% bright, ~0.7% mid,
/// ~1.0% dim, ~98% empty.
pub fn star_char_at(row: usize, col: usize, t: &Theme) -> (char, Color) {
    let mut h = row
        .wrapping_mul(2_654_435_761)
        .wrapping_add(col.wrapping_mul(2_246_822_519));
    h ^= h >> 13;
    h = h.wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    h ^= col
        .wrapping_mul(374_761_393)
        .wrapping_add(row.wrapping_mul(668_265_263));
    h ^= h >> 15;

    match h % 1000 {
        0..=2 => ('\u{2726}', t.star_bright), // ✦ — very rare
        3..=9 => ('\u{00b7}', t.star_mid),    // ·
        10..=19 => ('\u{22c6}', t.star_dim),  // ⋆
        _ => (' ', t.star_dim),
    }
}

/// Builds a full-width line of pure starfield for `row` and `w` columns.
///
/// Adjacent cells with the same color are merged into a single
/// [`Span`] to keep the resulting widget light.
pub fn build_star_line(w: usize, row: usize, t: &Theme) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cur_color = t.star_dim;
    let mut cur_text = String::new();
    for col in 0..w {
        let (ch, color) = star_char_at(row, col, t);
        if color == cur_color {
            cur_text.push(ch);
        } else {
            if !cur_text.is_empty() {
                spans.push(Span::styled(
                    cur_text.clone(),
                    Style::default().fg(cur_color),
                ));
                cur_text.clear();
            }
            cur_color = color;
            cur_text.push(ch);
        }
    }
    if !cur_text.is_empty() {
        spans.push(Span::styled(cur_text, Style::default().fg(cur_color)));
    }
    Line::from(spans)
}

/// Fills `area` with the pure star pattern (no logo overlay).
pub fn fill_stars(frame: &mut Frame, area: Rect, t: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let w = area.width as usize;
    let h = area.height as usize;
    let lines: Vec<Line> = (0..h)
        .map(|r| build_star_line(w, area.y as usize + r, t))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}
