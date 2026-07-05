//! Logo renderer — the pre-rendered wordmark overlaid on the starfield.
//!
//! The two blocks below are the FIGlet *slant* rendering of
//! "secret" / "base", kerned (each glyph slid left until just before a
//! non-space collision) so every word reads as one unit. They were
//! generated once and embedded verbatim — the wordmark is constant, so
//! shipping a FIGlet font + parser to recompute it at runtime bought
//! nothing.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::app::App;
use crate::tui::view::starfield::{build_star_line, star_char_at};

/// Crate version string for the centered subtitle.
const VERSION: &str = "v0.1.0";

/// The wordmark's top block ("secret"), one row per line.
const WORDMARK_TOP: [&str; 5] = [
    r"                                  __ ",
    r"   _____ ___   _____ _____ ___   / /_",
    r"  / ___// _ \ / ___// ___// _ \ / __/",
    r" (__  )/  __// /__ / /   /  __// /_  ",
    r"/____/ \___/ \___//_/    \___/ \__/  ",
];

/// The wordmark's bottom block ("base"), one row per line.
const WORDMARK_BOTTOM: [&str; 5] = [
    r"    __                     ",
    r"   / /_   ____ _ _____ ___ ",
    r"  / __ \ / __ `// ___// _ \",
    r" / /_/ // /_/ /(__  )/  __/",
    r"/_.___/ \__,_//____/ \___/ ",
];

/// Renders the logo + version + surrounding starfield into `area`.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let w = area.width as usize;
    let h = area.height as usize;

    let style_dim = Style::default().fg(t.star_dim);
    let style_inactive = Style::default().fg(t.inactive);
    let style_accent = Style::default().fg(t.accent);

    // The two stacked words — pre-rendered, kerned FIGlet blocks (see the
    // module docs).
    let r1: Vec<&str> = WORDMARK_TOP.to_vec();
    let r2: Vec<&str> = WORDMARK_BOTTOM.to_vec();

    // Vertical layout — first FIG word at row 1, second below it, then
    // the version label centered in the leftover space.
    let r1_start = 1usize;
    let r2_start = r1_start + r1.len();
    let text_end = r2_start + r2.len();
    let version_row = text_end + (h.saturating_sub(text_end + 1)) / 2;

    let f1_w = r1.iter().map(|l| l.len()).max().unwrap_or(40);
    let f2_w = r2.iter().map(|l| l.len()).max().unwrap_or(40);
    let f1_col = if w > f1_w { (w - f1_w) / 2 } else { 0 };
    let f2_col = if w > f2_w { (w - f2_w) / 2 } else { 0 };

    let spans_from = |row: usize, fig: &str, fc: usize, fw: usize| -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut cur_style = style_dim;
        let mut cur_text = String::new();
        for col in 0..w {
            let fi = col.wrapping_sub(fc);
            let (ch, st) = if col >= fc && fi < fw {
                let c = fig.chars().nth(fi).unwrap_or(' ');
                if c != ' ' {
                    (c, style_accent)
                } else {
                    let (ch, color) = star_char_at(row, col, t);
                    (ch, Style::default().fg(color))
                }
            } else {
                let (ch, color) = star_char_at(row, col, t);
                (ch, Style::default().fg(color))
            };
            if st == cur_style {
                cur_text.push(ch);
            } else {
                if !cur_text.is_empty() {
                    spans.push(Span::styled(cur_text.clone(), cur_style));
                    cur_text.clear();
                }
                cur_style = st;
                cur_text.push(ch);
            }
        }
        if !cur_text.is_empty() {
            spans.push(Span::styled(cur_text, cur_style));
        }
        Line::from(spans)
    };

    let mut lines: Vec<Line> = Vec::with_capacity(h);
    for row in 0..h {
        if row == version_row {
            lines.push(
                Line::from(Span::styled(VERSION, style_inactive)).alignment(Alignment::Center),
            );
            continue;
        }
        let r = row.wrapping_sub(r1_start);
        let s = row.wrapping_sub(r2_start);
        if row >= r1_start && r < r1.len() {
            lines.push(spans_from(row, r1[r], f1_col, f1_w));
        } else if row >= r2_start && s < r2.len() {
            lines.push(spans_from(row, r2[s], f2_col, f2_w));
        } else {
            lines.push(build_star_line(w, row, t));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}
