//! Embedded, headless-safe file picker.
//!
//! A single-pane, desktop-chooser-style file browser rendered entirely in
//! the terminal — no GUI / portal dependency, so it works the same over
//! SSH or on a headless box as it does on a desktop. It is intentionally
//! self-contained (depends only on `ratatui`, the shared [`Theme`] and
//! [`LineEditor`]) so the exact same module can be dropped into the other
//! TUIs unchanged.
//!
//! Usage: the host opens a [`FilePicker`], routes key events to
//! [`FilePicker::handle_key`] while it's open, and reads the returned
//! [`Outcome`] — `Selected(path)` fires the host action (e.g. upload),
//! `Cancelled` closes it, `Pending` keeps it open. The host renders it
//! with [`FilePicker::render`].

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::domain::LineEditor;
use crate::tui::theme::Theme;
use crate::tui::view::widgets::editor_spans;

/// What the picker is choosing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    /// Pick an existing file (e.g. upload). Files are listed and selectable.
    OpenFile,
    /// Pick a directory (e.g. download destination). Only directories are
    /// listed, plus a synthetic "choose this folder" row for the cwd.
    Dir,
}

/// One row in the listing.
struct Entry {
    /// Display name (no trailing slash; the renderer adds it for dirs).
    name: String,
    /// Lowercased name, for the filter match.
    name_lc: String,
    path: PathBuf,
    is_dir: bool,
    /// The synthetic `..` parent row (always sorts first, never filtered).
    is_parent: bool,
    /// The synthetic "choose this folder" row (Dir mode only).
    is_choose: bool,
    size: u64,
    mtime: Option<SystemTime>,
}

/// Result of routing a key to the picker.
pub enum Outcome {
    /// Still open — keep routing keys and rendering.
    Pending,
    /// The user picked this file. The host should act on it and close.
    Selected(PathBuf),
    /// The user cancelled (Esc). The host should close.
    Cancelled,
}

/// State of the embedded file picker.
pub struct FilePicker {
    /// File-pick vs directory-pick.
    mode: PickerMode,
    cwd: PathBuf,
    /// Current directory contents, unfiltered (with `..` first).
    all: Vec<Entry>,
    /// Indices into `all` after the hidden + text filters — the visible rows.
    view: Vec<usize>,
    /// Selected row, indexing `view`.
    selected: usize,
    /// First visible row (bottom-anchored scrolling handled in render).
    scroll: usize,
    /// Whether dotfiles are shown (toggled with `.`).
    show_hidden: bool,
    /// Whether the `/` filter input has focus.
    filtering: bool,
    /// The `/` filter text.
    filter: LineEditor,
    /// Last directory-read error, shown inline.
    error: Option<String>,
}

impl FilePicker {
    /// Opens a **file** picker rooted at `start` (or its parent if `start`
    /// is a file; or `$HOME` / `/` as a last resort).
    pub fn new(start: &Path) -> Self {
        Self::with_mode(start, PickerMode::OpenFile)
    }

    /// Opens a **directory** picker rooted at `start` — only directories
    /// are listed, plus a "choose this folder" row for the current dir.
    pub fn new_dir(start: &Path) -> Self {
        Self::with_mode(start, PickerMode::Dir)
    }

    fn with_mode(start: &Path, mode: PickerMode) -> Self {
        let cwd = resolve_start_dir(start);
        let mut p = Self {
            mode,
            cwd,
            all: Vec::new(),
            view: Vec::new(),
            selected: 0,
            scroll: 0,
            show_hidden: true,
            filtering: false,
            filter: LineEditor::default(),
            error: None,
        };
        p.load();
        p
    }

    /// The directory currently being browsed.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Reads `cwd` into `all` (dirs first, then files, case-insensitive),
    /// prepends the `..` row, and rebuilds the visible view.
    fn load(&mut self) {
        self.all.clear();
        self.error = None;
        // Directory mode: a "choose this folder" affordance for the cwd.
        if self.mode == PickerMode::Dir {
            self.all.push(Entry {
                name: "[ choose this folder ]".into(),
                name_lc: String::new(),
                path: self.cwd.clone(),
                is_dir: true,
                is_parent: false,
                is_choose: true,
                size: 0,
                mtime: None,
            });
        }
        if self.cwd.parent().is_some() {
            self.all.push(Entry {
                name: "..".into(),
                name_lc: "..".into(),
                path: self.cwd.parent().unwrap_or(&self.cwd).to_path_buf(),
                is_dir: true,
                is_parent: true,
                is_choose: false,
                size: 0,
                mtime: None,
            });
        }
        match fs::read_dir(&self.cwd) {
            Ok(rd) => {
                let mut items: Vec<Entry> = Vec::new();
                for dent in rd.flatten() {
                    let path = dent.path();
                    let name = dent.file_name().to_string_lossy().to_string();
                    // `metadata` follows symlinks; fall back to the symlink's
                    // own metadata so a dangling link still lists.
                    let md = dent
                        .metadata()
                        .or_else(|_| fs::symlink_metadata(&path))
                        .ok();
                    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                    // A directory picker only lists directories.
                    if self.mode == PickerMode::Dir && !is_dir {
                        continue;
                    }
                    let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                    let mtime = md.as_ref().and_then(|m| m.modified().ok());
                    items.push(Entry {
                        name_lc: name.to_lowercase(),
                        name,
                        path,
                        is_dir,
                        is_parent: false,
                        is_choose: false,
                        size,
                        mtime,
                    });
                }
                // Dirs first, then files; each group case-insensitive by name.
                items.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name_lc.cmp(&b.name_lc))
                });
                self.all.extend(items);
            }
            Err(e) => {
                self.error = Some(format!("cannot read directory: {e}"));
            }
        }
        self.selected = 0;
        self.scroll = 0;
        self.rebuild_view();
    }

    /// Recomputes `view` from `all` applying the hidden + text filters.
    /// The `..` row is always kept; selection is clamped.
    fn rebuild_view(&mut self) {
        let q = self.filter.text().to_lowercase();
        self.view = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if e.is_parent || e.is_choose {
                    // The synthetic rows show while browsing but drop out once
                    // a filter is active so the first real match is selected.
                    q.is_empty()
                } else {
                    (self.show_hidden || !e.name.starts_with('.'))
                        && (q.is_empty() || subsequence(&e.name_lc, &q))
                }
            })
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.view.len() {
            self.selected = self.view.len().saturating_sub(1);
        }
    }

    /// Routes a key event; returns the resulting [`Outcome`].
    pub fn handle_key(&mut self, key: KeyEvent) -> Outcome {
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                    self.rebuild_view();
                }
                KeyCode::Enter => return self.activate(),
                KeyCode::Up => self.move_by(-1),
                KeyCode::Down => self.move_by(1),
                KeyCode::Backspace => {
                    self.filter.backspace();
                    self.rebuild_view();
                }
                KeyCode::Char(c) => {
                    self.filter.insert(c);
                    self.rebuild_view();
                }
                _ => {}
            }
            return Outcome::Pending;
        }
        match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => return self.activate(),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => self.go_parent(),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = self.view.len().saturating_sub(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Char('~') => {
                if let Some(home) = home_dir() {
                    self.cd(home);
                }
            }
            // `.` toggles dotfile visibility.
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                self.rebuild_view();
            }
            KeyCode::Char('/') => self.filtering = true,
            _ => {}
        }
        Outcome::Pending
    }

    /// Enter the selected directory, or pick the selected file.
    fn activate(&mut self) -> Outcome {
        let Some(&i) = self.view.get(self.selected) else {
            return Outcome::Pending;
        };
        let e = &self.all[i];
        if e.is_choose {
            // Pick the current directory itself.
            Outcome::Selected(e.path.clone())
        } else if e.is_dir {
            let path = e.path.clone();
            self.cd(path);
            Outcome::Pending
        } else {
            Outcome::Selected(e.path.clone())
        }
    }

    fn go_parent(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            let parent = parent.to_path_buf();
            self.cd(parent);
        }
    }

    fn cd(&mut self, dir: PathBuf) {
        self.cwd = dir;
        self.filtering = false;
        self.filter.clear();
        self.load();
    }

    fn move_by(&mut self, delta: isize) {
        if self.view.is_empty() {
            return;
        }
        let last = self.view.len() - 1;
        let next = (self.selected as isize + delta).clamp(0, last as isize);
        self.selected = next as usize;
    }

    /// Renders the picker into `area`.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, t: &Theme) {
        let hints = match (self.filtering, self.mode) {
            (true, _) => " type to filter · ↑↓ move · Enter · Esc clear ",
            (false, PickerMode::OpenFile) => {
                " ↑↓ move · Enter open/pick · ⌫ up · / filter · . hidden · Esc cancel "
            }
            (false, PickerMode::Dir) => {
                " ↑↓ move · Enter open/choose · ⌫ up · / filter · . hidden · Esc cancel "
            }
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                title_for(&self.cwd, self.mode, area.width),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Line::from(Span::styled(hints, Style::default().fg(t.dim))))
            .border_style(Style::default().fg(t.accent));
        let inner = block.inner(area);
        // Clear the area first so the Home underneath doesn't bleed through the
        // modal (every other overlay does this).
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        // Reserve the bottom inner row for the filter input when active.
        let (list_area, filter_area) = if self.filtering {
            let rows = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            (rows[0], Some(rows[1]))
        } else {
            (inner, None)
        };

        if let Some(err) = &self.error {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  {err}"),
                    Style::default().fg(t.error),
                ))),
                list_area,
            );
            return;
        }

        let vh = list_area.height as usize;
        // Keep the selection inside the viewport.
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if vh > 0 && self.selected >= self.scroll + vh {
            self.scroll = self.selected + 1 - vh;
        }

        let now = SystemTime::now();
        let width = list_area.width as usize;
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(vh);
        if self.view.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (empty)",
                Style::default().fg(t.dim),
            )));
        }
        for (row, &i) in self
            .view
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(vh.max(1))
        {
            lines.push(row_line(&self.all[i], row == self.selected, width, now, t));
        }
        frame.render_widget(Paragraph::new(lines), list_area);

        if let Some(fa) = filter_area {
            let mut spans = vec![Span::styled("/ ", Style::default().fg(t.accent))];
            spans.extend(editor_spans(&self.filter, true, t));
            frame.render_widget(Paragraph::new(Line::from(spans)), fa);
        }
    }
}

/// Builds one full-width listing row (cursor · icon · name · right-aligned
/// size + relative mtime), tinted and given the selection background.
fn row_line(e: &Entry, selected: bool, width: usize, now: SystemTime, t: &Theme) -> Line<'static> {
    let cursor = if selected { "▶ " } else { "  " };
    let icon = if e.is_choose {
        "📂 "
    } else if e.is_dir {
        "📁 "
    } else {
        "📄 "
    };
    let mut name = e.name.clone();
    if e.is_dir && !e.is_parent && !e.is_choose {
        name.push('/');
    }
    let meta = if e.is_parent {
        String::new()
    } else if e.is_dir {
        rel_time(e.mtime, now)
    } else {
        let when = rel_time(e.mtime, now);
        if when.is_empty() {
            human_size(e.size)
        } else {
            format!("{}   {}", human_size(e.size), when)
        }
    };

    let meta_w = meta.chars().count();
    // Left budget = everything except the meta column + a gap.
    let left_used = cursor.chars().count() + icon.chars().count() + name.chars().count();
    let gap = width.saturating_sub(left_used + meta_w);
    let name_color = if e.is_choose || e.is_dir {
        t.accent
    } else {
        t.foreground
    };

    let mut spans = vec![
        Span::styled(cursor.to_string(), Style::default().fg(t.accent)),
        Span::styled(icon.to_string(), Style::default().fg(name_color)),
        Span::styled(name, Style::default().fg(name_color)),
        Span::raw(" ".repeat(gap)),
        Span::styled(meta, Style::default().fg(t.dim)),
    ];
    if selected {
        for s in &mut spans {
            s.style = s.style.bg(t.selected_bg);
        }
    }
    Line::from(spans)
}

/// Case-insensitive subsequence test: are all chars of `needle` present in
/// `haystack` in order? (`needle` is already lowercased.)
fn subsequence(haystack_lc: &str, needle_lc: &str) -> bool {
    let mut chars = haystack_lc.chars();
    needle_lc.chars().all(|c| chars.any(|h| h == c))
}

/// Title bar: " Pick a file/folder · <path> ", truncated from the left
/// (keeping the most specific tail) when it doesn't fit.
fn title_for(cwd: &Path, mode: PickerMode, total_width: u16) -> String {
    let path = cwd.to_string_lossy();
    let prefix = match mode {
        PickerMode::OpenFile => " Pick a file · ",
        PickerMode::Dir => " Pick a folder · ",
    };
    let avail = (total_width as usize).saturating_sub(prefix.chars().count() + 3);
    let shown: String = if path.chars().count() > avail && avail > 1 {
        let tail: String = path
            .chars()
            .rev()
            .take(avail - 1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    } else {
        path.to_string()
    };
    format!("{prefix}{shown} ")
}

fn resolve_start_dir(start: &Path) -> PathBuf {
    let cand = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    if cand.is_dir() {
        cand
    } else {
        home_dir().unwrap_or_else(|| PathBuf::from("/"))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

fn rel_time(mtime: Option<SystemTime>, now: SystemTime) -> String {
    let Some(m) = mtime else {
        return String::new();
    };
    let secs = now.duration_since(m).map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 {
        "now".into()
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else if secs < 2_592_000 {
        format!("{}d", secs / 86_400)
    } else if secs < 31_536_000 {
        format!("{}mo", secs / 2_592_000)
    } else {
        format!("{}y", secs / 31_536_000)
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut s = bytes as f64;
    let mut i = 0;
    while s >= 1024.0 && i < UNITS.len() - 1 {
        s /= 1024.0;
        i += 1;
    }
    format!("{s:.1} {}", UNITS[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::fs::{File, create_dir};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        create_dir(d.path().join("zeta_dir")).unwrap();
        create_dir(d.path().join("alpha_dir")).unwrap();
        File::create(d.path().join("readme.txt")).unwrap();
        File::create(d.path().join("Cargo.toml")).unwrap();
        File::create(d.path().join(".hidden")).unwrap();
        d
    }

    fn names(p: &FilePicker) -> Vec<String> {
        p.view.iter().map(|&i| p.all[i].name.clone()).collect()
    }

    #[test]
    fn lists_dirs_first_then_files_with_parent_on_top() {
        let d = fixture();
        let p = FilePicker::new(d.path());
        let n = names(&p);
        assert_eq!(n[0], "..");
        // dirs (case-insensitive) before files
        assert_eq!(&n[1..3], &["alpha_dir", "zeta_dir"]);
        assert!(n.contains(&"Cargo.toml".to_string()));
        // hidden shown by default
        assert!(n.contains(&".hidden".to_string()));
    }

    #[test]
    fn hidden_toggle_hides_dotfiles_but_keeps_parent() {
        let d = fixture();
        let mut p = FilePicker::new(d.path());
        p.handle_key(key(KeyCode::Char('.')));
        let n = names(&p);
        assert!(!n.contains(&".hidden".to_string()));
        assert_eq!(n[0], "..");
    }

    #[test]
    fn filter_narrows_to_subsequence_matches() {
        let d = fixture();
        let mut p = FilePicker::new(d.path());
        p.handle_key(key(KeyCode::Char('/')));
        for c in "cgo".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        // "cgo" is a subsequence of "cargo.toml"
        assert!(names(&p).contains(&"Cargo.toml".to_string()));
        assert!(!names(&p).contains(&"readme.txt".to_string()));
    }

    #[test]
    fn entering_a_dir_changes_cwd_and_picking_a_file_returns_it() {
        let d = fixture();
        let mut p = FilePicker::new(d.path());
        // Select the first real dir (index 1 = alpha_dir) and enter it.
        p.handle_key(key(KeyCode::Down));
        let out = p.handle_key(key(KeyCode::Enter));
        assert!(matches!(out, Outcome::Pending));
        assert_eq!(p.cwd(), d.path().join("alpha_dir"));
        // Back to parent.
        p.handle_key(key(KeyCode::Backspace));
        assert_eq!(p.cwd(), d.path());
        // Jump to Cargo.toml via filter and pick it.
        p.handle_key(key(KeyCode::Char('/')));
        for c in "cargo".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        let out = p.handle_key(key(KeyCode::Enter));
        match out {
            Outcome::Selected(path) => assert_eq!(path, d.path().join("Cargo.toml")),
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn esc_cancels() {
        let d = fixture();
        let mut p = FilePicker::new(d.path());
        assert!(matches!(
            p.handle_key(key(KeyCode::Esc)),
            Outcome::Cancelled
        ));
    }

    #[test]
    fn dir_mode_lists_only_dirs_with_a_choose_row() {
        let d = fixture();
        let p = FilePicker::new_dir(d.path());
        let n = names(&p);
        assert_eq!(n[0], "[ choose this folder ]");
        assert!(n.contains(&"..".to_string()));
        assert!(n.contains(&"alpha_dir".to_string()));
        // Files are not listed when picking a directory.
        assert!(!n.contains(&"Cargo.toml".to_string()));
        assert!(!n.contains(&"readme.txt".to_string()));
    }

    #[test]
    fn dir_mode_choose_returns_the_current_dir() {
        let d = fixture();
        let mut p = FilePicker::new_dir(d.path());
        // The choose row is first and selected by default.
        match p.handle_key(key(KeyCode::Enter)) {
            Outcome::Selected(path) => assert_eq!(path, d.path()),
            _ => panic!("expected Selected(cwd)"),
        }
    }
}
