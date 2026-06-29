//! Inline terminal image rendering for chat attachments.
//!
//! Shells out to `chafa`, which can emit the **kitty** graphics protocol
//! (`-f kitty`), **sixel** (`-f sixel`), **iTerm** inline images (`-f iterm`)
//! or plain Unicode **symbols** (`-f symbols`). We detect the terminal from
//! the environment and force the matching format (chafa's own probing does not
//! apply when its stdout is captured by us).
//!
//! Ported from jewel's single-image preview, adapted for the chat's different
//! use case: **many** images visible at once, scrolling. The improvement over
//! jewel is a per-`(path, w, h)` **render cache** ([`RenderCache`]) so that
//! repainting on every scroll is just writing cached bytes — chafa runs once
//! per image size, not once per frame.
//!
//! Lives in the TUI layer, not `adapters/`: it is terminal rendering (like the
//! clipboard), not a Keybase call, and it writes image bytes straight to
//! stdout positioned over reserved regions — something only the render loop
//! can coordinate.

use std::collections::HashMap;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Image protocol used to render inline thumbnails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImgProto {
    Kitty,
    Sixel,
    Iterm,
    Symbols,
}

impl ImgProto {
    fn chafa_format(self) -> &'static str {
        match self {
            ImgProto::Kitty => "kitty",
            ImgProto::Sixel => "sixel",
            ImgProto::Iterm => "iterm",
            ImgProto::Symbols => "symbols",
        }
    }

    /// True for the native graphics protocols (a single escape blob written
    /// once); `Symbols` is text rows that must be positioned per line.
    fn is_graphics(self) -> bool {
        !matches!(self, ImgProto::Symbols)
    }
}

/// Resolves the effective protocol from the `image_protocol` config value
/// (`auto`/`kitty`/`sixel`/`iterm`/`symbols`/`off`), falling back to
/// environment detection for `auto` / unknown values. `off` disables images.
pub fn resolve(setting: &str) -> Option<ImgProto> {
    match setting.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "disabled" => None,
        "kitty" => Some(ImgProto::Kitty),
        "sixel" => Some(ImgProto::Sixel),
        "iterm" | "iterm2" => Some(ImgProto::Iterm),
        "symbols" | "ansi" | "chafa" => Some(ImgProto::Symbols),
        _ => Some(detect()),
    }
}

/// Best-effort terminal capability detection from the environment.
pub fn detect() -> ImgProto {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let term = env("TERM").to_ascii_lowercase();
    let term_program = env("TERM_PROGRAM");

    let kitty = !env("KITTY_WINDOW_ID").is_empty()
        || !env("KITTY_PID").is_empty()
        || term.contains("kitty")
        || !env("GHOSTTY_RESOURCES_DIR").is_empty()
        || term_program.eq_ignore_ascii_case("ghostty")
        || term_program.eq_ignore_ascii_case("WezTerm")
        || !env("KONSOLE_VERSION").is_empty();
    if kitty {
        return ImgProto::Kitty;
    }
    if term_program.to_ascii_lowercase().contains("iterm") {
        return ImgProto::Iterm;
    }
    let sixel = ["foot", "contour", "mlterm", "yaft", "sixel"]
        .iter()
        .any(|t| term.contains(t));
    if sixel {
        return ImgProto::Sixel;
    }
    ImgProto::Symbols
}

/// Caches chafa output bytes per `(path, cols, rows)` so re-scroll repaints
/// don't re-run chafa. Keyed by the rendered size since that's what changes.
#[derive(Default)]
pub struct RenderCache {
    map: HashMap<(String, u16, u16), Vec<u8>>,
}

impl RenderCache {
    /// Returns the chafa output for `path` at `cols`×`rows`, running chafa on a
    /// miss and caching the result.
    fn get(&mut self, proto: ImgProto, path: &str, cols: u16, rows: u16) -> io::Result<&[u8]> {
        let key = (path.to_string(), cols, rows);
        if !self.map.contains_key(&key) {
            let bytes = run_chafa(proto, path, cols, rows)?;
            self.map.insert(key.clone(), bytes);
        }
        Ok(self.map.get(&key).map(Vec::as_slice).unwrap_or_default())
    }

    /// Like [`Self::get`] but returns an owned copy — for the `symbols`
    /// in-buffer path, which parses the bytes into Ratatui lines.
    pub fn bytes(
        &mut self,
        proto: ImgProto,
        path: &str,
        cols: u16,
        rows: u16,
    ) -> io::Result<Vec<u8>> {
        self.get(proto, path, cols, rows).map(<[u8]>::to_vec)
    }

    /// Drops every cached render (e.g. on protocol change). The downloaded
    /// files are owned elsewhere; this only frees the rendered bytes.
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

/// Parses `chafa -f symbols` ANSI output into Ratatui lines so the image can
/// render **inside** the frame buffer (scroll, occlusion, and clearing then
/// come for free — no direct-to-stdout ghosting). Handles 24-bit `38;2`/`48;2`
/// SGR colours plus reset; each output row is left-padded by `indent` spaces.
pub fn symbols_to_lines(bytes: &[u8], indent: usize, rows: u16) -> Vec<Line<'static>> {
    let text = String::from_utf8_lossy(bytes);
    let pad = " ".repeat(indent);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for raw in text.split('\n').take(rows as usize) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if indent > 0 {
            spans.push(Span::raw(pad.clone()));
        }
        let mut style = Style::default();
        let mut buf = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                if !buf.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buf), style));
                }
                if chars.peek() == Some(&'[') {
                    chars.next();
                }
                let mut seq = String::new();
                for nc in chars.by_ref() {
                    if nc == 'm' {
                        break;
                    }
                    seq.push(nc);
                }
                style = apply_sgr(style, &seq);
            } else {
                buf.push(c);
            }
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Applies a `;`-separated SGR parameter list to `style` (the subset chafa
/// emits: reset, 24-bit fg/bg, default fg/bg).
fn apply_sgr(mut style: Style, seq: &str) -> Style {
    let codes: Vec<&str> = seq.split(';').collect();
    let num = |s: &str| s.parse::<u8>().ok();
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            "" | "0" => style = Style::default(),
            "39" => style = style.fg(Color::Reset),
            "49" => style = style.bg(Color::Reset),
            "38" | "48" if codes.get(i + 1) == Some(&"2") => {
                let rgb = (
                    codes.get(i + 2).and_then(|s| num(s)),
                    codes.get(i + 3).and_then(|s| num(s)),
                    codes.get(i + 4).and_then(|s| num(s)),
                );
                if let (Some(r), Some(g), Some(b)) = rgb {
                    let color = Color::Rgb(r, g, b);
                    style = if codes[i] == "38" {
                        style.fg(color)
                    } else {
                        style.bg(color)
                    };
                }
                i += 4;
            }
            _ => {}
        }
        i += 1;
    }
    style
}

fn run_chafa(proto: ImgProto, path: &str, cols: u16, rows: u16) -> io::Result<Vec<u8>> {
    let size = format!("{cols}x{rows}");
    let mut args: Vec<&str> = vec![
        "-f",
        proto.chafa_format(),
        "--size",
        &size,
        // Quality: ordered dithering smooths gradients and the din99d colour
        // space quantises perceptually (ignored by truecolor kitty/sixel).
        "--dither=ordered",
        "--color-space=din99d",
    ];
    if proto == ImgProto::Symbols {
        // Sextants pack 2×3 sub-cell pixels (3× the detail of half-blocks);
        // block+space cover solid areas. Needs "Symbols for Legacy Computing"
        // glyphs in the font — falls to boxes otherwise, but most modern
        // monospace / Nerd fonts include them.
        args.push("--symbols=sextant+block+space");
    }
    args.extend(["--animate", "off", "--polite", "on", path]);
    let output = Command::new("chafa")
        .args(&args)
        .output()
        .map_err(|e| io::Error::other(format!("chafa not available: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("chafa failed: {}", err.trim())));
    }
    Ok(output.stdout)
}

/// Paints `path` into `area` with `proto`, via the render cache. Symbol output
/// is plain text rows: in raw mode `\n` only moves down (no carriage return),
/// so each row is repositioned with its own `MoveTo`. The native graphics
/// protocols are a single blob written at the top-left.
pub fn render_into(
    cache: &mut RenderCache,
    proto: ImgProto,
    area: Rect,
    path: &str,
) -> io::Result<()> {
    if area.width == 0 || area.height == 0 {
        return Ok(());
    }
    let bytes = cache.get(proto, path, area.width, area.height)?.to_vec();
    let mut out = io::stdout();
    if proto.is_graphics() {
        queue!(out, MoveTo(area.x, area.y))?;
        out.write_all(&bytes)?;
    } else {
        for (i, line) in bytes.split(|&b| b == b'\n').enumerate() {
            if i as u16 >= area.height {
                break;
            }
            queue!(out, MoveTo(area.x, area.y + i as u16))?;
            out.write_all(line)?;
        }
        out.write_all(b"\x1b[0m")?;
    }
    out.flush()?;
    Ok(())
}

/// Clears any kitty graphics from the screen (no-op on terminals that ignore
/// the APC sequence). Call before redrawing normally.
pub fn clear() -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(b"\x1b_Ga=d\x1b\\")?;
    out.flush()?;
    Ok(())
}

/// Copies the image file at `path` into the system clipboard as `mime`,
/// shelling out to the first available tool: `wl-copy` (Wayland), `xclip`
/// (X11), or `pbcopy` (macOS, type-agnostic).
pub fn copy_to_clipboard(path: &str, mime: &str) -> io::Result<()> {
    let data = std::fs::read(path)?;
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && pipe_to("wl-copy", &["--type", mime], &data).is_ok()
    {
        return Ok(());
    }
    if pipe_to(
        "xclip",
        &["-selection", "clipboard", "-t", mime, "-i"],
        &data,
    )
    .is_ok()
    {
        return Ok(());
    }
    if pipe_to("pbcopy", &[], &data).is_ok() {
        return Ok(());
    }
    Err(io::Error::other(
        "no image clipboard tool (install wl-clipboard or xclip)",
    ))
}

fn pipe_to(cmd: &str, args: &[&str], data: &[u8]) -> io::Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        stdin.write_all(data)?;
    } // drop closes stdin so the tool proceeds
    if child.wait()?.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{cmd} failed")))
    }
}

/// Whether a MIME type / filename looks like a raster image chafa can render.
pub fn is_image(mime: &str, filename: &str) -> bool {
    if mime.to_ascii_lowercase().starts_with("image/") {
        return true;
    }
    let lower = filename.to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".tiff", ".tif", ".ppm", ".pgm", ".xpm",
        ".tga", ".avif", ".heic", ".heif", ".jxl", ".svg", ".qoi",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_off_disables_and_explicit_wins() {
        assert_eq!(resolve("off"), None);
        assert_eq!(resolve("kitty"), Some(ImgProto::Kitty));
        assert_eq!(resolve("sixel"), Some(ImgProto::Sixel));
        assert_eq!(resolve("symbols"), Some(ImgProto::Symbols));
        // Unknown / auto → environment detection (always Some).
        assert!(resolve("auto").is_some());
    }

    #[test]
    fn symbols_to_lines_parses_fg_and_indent() {
        // "AB" in red, then reset, then "CD" default — single row (no newline).
        let s = b"\x1b[38;2;255;0;0mAB\x1b[0mCD";
        let lines = symbols_to_lines(s, 2, 4);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content.as_ref(), "  "); // indent
        let ab = spans.iter().find(|s| s.content.as_ref() == "AB").unwrap();
        assert_eq!(ab.style.fg, Some(Color::Rgb(255, 0, 0)));
        let cd = spans.iter().find(|s| s.content.as_ref() == "CD").unwrap();
        assert_eq!(cd.style.fg, None);
    }

    #[test]
    fn is_image_matches_mime_and_extension() {
        assert!(is_image("image/jpeg", "pimpi-01.jpeg"));
        assert!(is_image("", "photo.PNG"));
        assert!(is_image("application/octet-stream", "x.webp"));
        assert!(!is_image("text/plain", "notes.txt"));
        assert!(!is_image("", "archive.zip"));
    }
}
