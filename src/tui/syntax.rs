//! Syntax highlighting for fenced code blocks, via [`syntect`].
//!
//! The heavy `SyntaxSet` / `ThemeSet` (the bundled Sublime grammars + themes)
//! are loaded lazily (and pre-warmed from a background worker at startup —
//! [`preload`]). The per-block result is memoized by a hash of
//! `(dark, lang, code)` so rebuilding a message's lines (on a history /
//! width / theme change) doesn't re-highlight — the memo is keyed on
//! content, not panel width, so the cheap hard-wrap re-runs and a resize
//! needs no invalidation here.
//!
//! Only the **foreground** color of each token is used; the terminal background
//! shows through, so highlighted code blends with any secretbase theme (and we
//! pick a dark vs light syntect theme from the active theme's brightness).

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::LazyLock;

use ratatui::style::Color;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Bundled Sublime grammars (newline variants, required by `highlight_line`).
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
/// Bundled syntect themes (we use one dark + one light entry from this set).
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Highlighted code: one inner vec per source line, each a list of
/// `(foreground colour, text)` segments (newlines stripped).
pub type Highlighted = Vec<Vec<(Color, String)>>;

/// syntect theme used on dark terminals.
const DARK_THEME: &str = "base16-ocean.dark";
/// syntect theme used on light terminals.
const LIGHT_THEME: &str = "InspiredGitHub";

thread_local! {
    /// `hash(dark, lang, code)` → highlighted segments per source line. The
    /// render thread is the only caller, so a `thread_local` cache needs no
    /// locking (and `Rc` hits are a pointer bump, not a deep clone of every
    /// token string). Cleared wholesale when it grows past [`MEMO_CAP`].
    static MEMO: RefCell<HashMap<u64, Rc<Highlighted>>> = RefCell::new(HashMap::new());
}

/// Upper bound on cached code blocks before the memo is dropped (a session
/// viewing thousands of distinct snippets shouldn't grow unbounded).
const MEMO_CAP: usize = 256;

/// Forces the lazy `SyntaxSet`/`ThemeSet` loads. Called from a background
/// worker thread at startup so the first *rendered* code block doesn't pay
/// the dump-decompress hitch on the render thread (the `LazyLock`s are
/// thread-safe; whoever gets there first does the load, everyone else waits).
pub fn preload() {
    let _ = &*SYNTAXES;
    let _ = &*THEMES;
}

fn memo_key(dark: bool, lang: &str, code: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dark.hash(&mut h);
    lang.hash(&mut h);
    code.hash(&mut h);
    h.finish()
}

fn to_ratatui(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Highlights `code` for fence language token `lang` (e.g. `"rust"`, `"py"`).
///
/// Returns one inner vec per source line — `(foreground color, text)` segments
/// with the trailing newline stripped. `None` when `lang` is empty/unknown (the
/// caller then renders the block in a single flat color), so an unrecognised
/// fence never throws away the code. `dark` selects the dark vs light theme.
pub fn highlight(code: &str, lang: &str, dark: bool) -> Option<Rc<Highlighted>> {
    if lang.is_empty() {
        return None;
    }
    let ss = &*SYNTAXES;
    let syntax = ss.find_syntax_by_token(lang)?;

    let key = memo_key(dark, lang, code);
    if let Some(hit) = MEMO.with(|m| m.borrow().get(&key).cloned()) {
        return Some(hit);
    }

    let theme = THEMES
        .themes
        .get(if dark { DARK_THEME } else { LIGHT_THEME })?;
    let mut hl = HighlightLines::new(syntax, theme);
    let mut out: Highlighted = Vec::new();
    for line in LinesWithEndings::from(code) {
        // A grammar error mid-block shouldn't crash the render; fall back to
        // the plain renderer for the whole block instead.
        let regions = hl.highlight_line(line, ss).ok()?;
        let mut segs: Vec<(Color, String)> = Vec::new();
        for (style, text) in regions {
            let text = text.trim_end_matches(['\n', '\r']);
            if text.is_empty() {
                continue;
            }
            segs.push((to_ratatui(style.foreground), text.to_string()));
        }
        out.push(segs);
    }

    let out = Rc::new(out);
    MEMO.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() >= MEMO_CAP {
            m.clear();
        }
        m.insert(key, Rc::clone(&out));
    });
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_language_returns_none() {
        assert!(highlight("x = 1", "", true).is_none());
        assert!(highlight("x = 1", "definitely-not-a-language", true).is_none());
    }

    #[test]
    fn known_language_produces_colored_segments() {
        let out = highlight("fn main() {}\nlet x = 1;\n", "rust", true).unwrap();
        // One inner vec per source line.
        assert_eq!(out.len(), 2);
        // Reassembling the segments yields the original line text (no chars lost).
        let line0: String = out[0].iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(line0, "fn main() {}");
        // Highlighting assigns more than one distinct color across the snippet
        // (a keyword vs punctuation vs identifier), i.e. it actually ran.
        let colors: std::collections::HashSet<_> = out
            .iter()
            .flatten()
            .map(|(c, _)| format!("{c:?}"))
            .collect();
        assert!(colors.len() > 1, "expected multiple token colors");
    }

    #[test]
    fn second_call_is_served_from_the_memo() {
        // Same inputs twice → identical output (and the second hits the cache).
        let a = highlight("let y = 2;\n", "rust", false).unwrap();
        let b = highlight("let y = 2;\n", "rust", false).unwrap();
        assert_eq!(a, b);
    }
}
