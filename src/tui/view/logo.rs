//! Logo renderer — FIGlet wordmark overlaid on the starfield.

use figlet_rs::FIGfont;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::app::App;
use crate::tui::view::starfield::{build_star_line, star_char_at};

/// FIGlet font shipped inside the binary. The `slant.flf` file lives at
/// `src/tui/assets/slant.flf` and is embedded at build time so the binary
/// has no external file dependency.
const SLANT_FONT: &str = include_str!("../assets/slant.flf");

/// Crate version string for the centered subtitle.
const VERSION: &str = "v0.1.0";

/// Renders the logo + version + surrounding starfield into `area`.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let w = area.width as usize;
    let h = area.height as usize;

    let style_dim = Style::default().fg(t.star_dim);
    let style_inactive = Style::default().fg(t.inactive);
    let style_accent = Style::default().fg(t.accent);

    // Render the FIGlet text into two stacked words. Fall back to plain
    // text if the font fails to load (highly unlikely with embedded
    // data) — a panic here would crash the TUI on the very first frame.
    let (fig_top, fig_bottom) = {
        let font = FIGfont::from_content(SLANT_FONT)
            .ok()
            .or_else(|| FIGfont::standard().ok());
        match font {
            Some(font) => (
                font.convert("secret")
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "secret".into()),
                font.convert("base")
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "base".into()),
            ),
            None => ("secret".to_string(), "base".to_string()),
        }
    };

    let trim = |s: &str| -> Vec<String> {
        let ls: Vec<&str> = s.lines().collect();
        let a = ls.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
        let b = ls
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map(|i| i + 1)
            .unwrap_or(ls.len());
        ls[a..b].iter().map(|l| l.to_string()).collect()
    };

    let r1_owned = trim(&fig_top);
    let r2_owned = trim(&fig_bottom);
    let r1: Vec<&str> = r1_owned.iter().map(String::as_str).collect();
    let r2: Vec<&str> = r2_owned.iter().map(String::as_str).collect();

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
