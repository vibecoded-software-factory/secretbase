//! A tiny single-line text editor with a cursor.
//!
//! Backs every text input in the TUI (search box, compose pane, popup
//! inputs) so they all support cursor movement and mid-string editing
//! instead of append-only typing. UTF-8 safe: the cursor is a byte
//! index kept on a char boundary.
//!
//! **secretbase-specific:** unlike the sibling TUIs' `LineEditor`, this
//! one derives `ZeroizeOnDrop` so the buffer is overwritten with zeroes
//! when it is dropped. Every text input here can carry sensitive chat
//! content (a message draft, a participant username, a global-search
//! query), so the buffer should not linger on the heap after the editor
//! is dropped. This restores — and extends — the hygiene the
//! `compose`/`new-conversation` drafts had before they moved onto the
//! shared editor. (jewel doesn't need this and keeps its editor
//! plain — not every decision applies to every app equally.)

use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct LineEditor {
    text: String,
    /// Byte offset into `text`, always on a char boundary, `0..=len`.
    cursor: usize,
}

impl LineEditor {
    pub fn from_text(s: impl Into<String>) -> Self {
        let text = s.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Inserts `c` at the cursor and advances past it.
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Deletes the char before the cursor (Backspace).
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.prev_boundary();
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    /// Deletes the char at the cursor (Delete).
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_boundary();
        self.text.replace_range(self.cursor..next, "");
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.prev_boundary();
        }
    }

    pub fn right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor = self.next_boundary();
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        // Overwrite the old contents before truncating: `String::clear`
        // sets len to 0 but leaves the (sensitive) bytes in the backing
        // capacity until the buffer is dropped or reallocated.
        self.text.zeroize();
        self.text.clear();
        self.cursor = 0;
    }

    /// Replaces the whole buffer and parks the cursor at the end.
    pub fn set(&mut self, s: impl Into<String>) {
        // Zeroize the previous buffer before it's dropped by the
        // reassignment — `ZeroizeOnDrop` only fires when the whole
        // `LineEditor` drops, not on a field overwrite.
        self.text.zeroize();
        self.text = s.into();
        self.cursor = self.text.len();
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_at_cursor() {
        let mut e = LineEditor::from_text("ac");
        e.cursor = 1;
        e.insert('b');
        assert_eq!(e.text(), "abc");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn backspace_and_delete_mid_string() {
        let mut e = LineEditor::from_text("abc");
        e.cursor = 2;
        e.backspace(); // removes 'b'
        assert_eq!(e.text(), "ac");
        assert_eq!(e.cursor(), 1);
        e.delete(); // removes 'c'
        assert_eq!(e.text(), "a");
        assert_eq!(e.cursor(), 1);
    }

    #[test]
    fn movement_clamps_and_home_end() {
        let mut e = LineEditor::from_text("abc");
        e.home();
        assert_eq!(e.cursor(), 0);
        e.left(); // no-op at start
        assert_eq!(e.cursor(), 0);
        e.right();
        assert_eq!(e.cursor(), 1);
        e.end();
        assert_eq!(e.cursor(), 3);
        e.right(); // no-op at end
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn utf8_safe() {
        let mut e = LineEditor::from_text("áé"); // each is 2 bytes
        assert_eq!(e.cursor(), 4);
        e.left();
        assert_eq!(e.cursor(), 2);
        e.insert('x');
        assert_eq!(e.text(), "áxé");
        e.backspace();
        assert_eq!(e.text(), "áé");
    }
}
