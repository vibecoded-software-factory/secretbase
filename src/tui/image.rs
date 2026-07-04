//! Inline terminal image rendering for chat attachments.
//!
//! Shells out to `chafa`, which can emit the **kitty** graphics protocol
//! (`-f kitty`), **sixel** (`-f sixel`), **iTerm** inline images (`-f iterm`)
//! or plain Unicode **symbols** (`-f symbols`). We detect the terminal from
//! the environment and force the matching format (chafa's own probing does not
//! apply when its stdout is captured by us).
//!
//! Built for the chat's use case: **many** images visible at once, scrolling.
//! A per-`(path, w, h)` **render cache** ([`RenderCache`]) means repainting on
//! every scroll is just writing cached bytes — chafa runs once per image size,
//! not once per frame.
//!
//! Lives in the TUI layer, not `adapters/`: it is terminal rendering (like the
//! clipboard), not a Keybase call, and it writes image bytes straight to
//! stdout positioned over reserved regions — something only the render loop
//! can coordinate.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::process::{Command, Output, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

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

/// The default chafa symbol set: sextants (2×3 sub-cell) with block/space
/// fallback — widely supported in modern monospace / Nerd fonts.
pub const DEFAULT_SYMBOLS: &str = "sextant+block+space";

/// Upper bound on entries kept in each render cache before the
/// least-recently-used one is evicted. Bounds memory over a long session and,
/// crucially, over a **many-frame animated GIF**: without it the per-frame
/// renders would accumulate without limit.
const RENDER_CACHE_CAP: usize = 256;

/// A tiny capacity-bounded LRU map. On overflow it evicts the entry with the
/// oldest access stamp (an internal monotonic clock bumped on every get/insert),
/// so neither the chafa-byte cache nor the parsed-line cache can grow without
/// bound — the fix for a long GIF spilling frame renders forever.
struct Lru<K: std::hash::Hash + Eq + Clone, V> {
    map: HashMap<K, (V, u64)>,
    clock: u64,
    cap: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V> Lru<K, V> {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            clock: 0,
            cap: cap.max(1),
        }
    }

    fn get(&mut self, k: &K) -> Option<&V> {
        let t = self.clock;
        self.clock = self.clock.wrapping_add(1);
        let e = self.map.get_mut(k)?;
        e.1 = t;
        Some(&e.0)
    }

    fn insert(&mut self, k: K, v: V) {
        let t = self.clock;
        self.clock = self.clock.wrapping_add(1);
        self.map.insert(k, (v, t));
        if self.map.len() > self.cap
            && let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (_, stamp))| *stamp)
                .map(|(key, _)| key.clone())
        {
            self.map.remove(&oldest);
        }
    }

    fn clear(&mut self) {
        self.map.clear();
    }
}

/// Caches rendered images so repaints (scroll, GIF animation) don't re-run
/// chafa. Two layers, both LRU-bounded ([`RENDER_CACHE_CAP`]):
///
/// * `bytes` — raw chafa output per `(path, cols, rows)` for the native
///   graphics protocols (kitty/sixel/iterm), written straight to the terminal.
/// * `lines` — **parsed Ratatui lines** per `(indent\0path, cols, rows)` for the
///   `symbols` path. This is the key win: a GIF frame is chafa-rendered **and**
///   ANSI-parsed exactly once, then steady-state animation is a cache lookup —
///   no `chafa` subprocess and no re-parse on the render thread every tick.
pub struct RenderCache {
    // `Rc` values so a cache hit is a pointer bump, not a deep clone of the
    // whole rendered frame (hundreds of spans / KBs of chafa bytes) — hits
    // happen per visible image per frame while a GIF animates.
    bytes: Lru<(String, u16, u16), Rc<[u8]>>,
    lines: Lru<(String, u16, u16), Rc<[Line<'static>]>>,
    /// chafa `--symbols` spec for the symbol path (font-dependent).
    symbols: String,
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new(DEFAULT_SYMBOLS.to_string())
    }
}

impl RenderCache {
    /// Builds a cache that renders symbol output with the given chafa
    /// `--symbols` spec (empty → chafa's default set).
    pub fn new(symbols: String) -> Self {
        Self {
            bytes: Lru::new(RENDER_CACHE_CAP),
            lines: Lru::new(RENDER_CACHE_CAP),
            symbols,
        }
    }

    /// Chafa output for `path` at `cols`×`rows`, running chafa on a miss and
    /// caching it. Used by the native-graphics paint path.
    fn cached_bytes(
        &mut self,
        proto: ImgProto,
        path: &str,
        cols: u16,
        rows: u16,
    ) -> io::Result<Rc<[u8]>> {
        let key = (path.to_string(), cols, rows);
        if let Some(b) = self.bytes.get(&key) {
            return Ok(Rc::clone(b));
        }
        let bytes: Rc<[u8]> = run_chafa(proto, path, cols, rows, &self.symbols)?.into();
        self.bytes.insert(key, Rc::clone(&bytes));
        Ok(bytes)
    }

    /// Parsed symbol lines for `path` at `cols`×`rows`, left-padded by `indent`
    /// and trailing-blank-trimmed — the in-buffer `symbols` render. Runs chafa +
    /// ANSI-parse **once per (path,size)** and caches the result, so an animated
    /// GIF's per-tick redraw is a pure cache hit (no subprocess, no re-parse).
    /// Returns an empty vec when chafa is unavailable / fails (the caller then
    /// keeps the skeleton).
    pub fn symbol_lines(
        &mut self,
        proto: ImgProto,
        path: &str,
        cols: u16,
        rows: u16,
        indent: usize,
    ) -> Rc<[Line<'static>]> {
        let key = (format!("{indent}\u{0}{path}"), cols, rows);
        if let Some(l) = self.lines.get(&key) {
            return Rc::clone(l);
        }
        let Ok(bytes) = self.cached_bytes(proto, path, cols, rows) else {
            return Rc::new([]);
        };
        let mut lines = symbols_to_lines(&bytes, indent, rows);
        // Trim the trailing blank row chafa's output leaves so the reservation
        // matches the real thumbnail height (no gap below).
        while lines
            .last()
            .is_some_and(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        {
            lines.pop();
        }
        let lines: Rc<[Line<'static>]> = lines.into();
        self.lines.insert(key, Rc::clone(&lines));
        lines
    }

    /// Drops every cached render (e.g. on protocol / symbol-set change). The
    /// downloaded files are owned elsewhere; this only frees the rendered data.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.lines.clear();
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

/// The decoded frames of an animated GIF: per-frame PNG paths on disk (from
/// ImageMagick `-coalesce`) and their durations in milliseconds.
#[derive(Debug, Clone)]
pub struct GifFrames {
    pub frames: Vec<String>,
    pub delays_ms: Vec<u32>,
    pub total_ms: u32,
}

impl GifFrames {
    /// The frame path showing at `ms` (looping over `total_ms`).
    pub fn frame_at(&self, ms: u64) -> &str {
        if self.frames.is_empty() {
            return "";
        }
        let mut t = (ms % self.total_ms.max(1) as u64) as u32;
        for (i, d) in self.delays_ms.iter().enumerate() {
            if t < *d {
                return &self.frames[i];
            }
            t = t.saturating_sub(*d);
        }
        self.frames.last().map(String::as_str).unwrap_or("")
    }
}

/// Max pixel dimension of an extracted GIF frame. The frames are only ever
/// downsampled by chafa to a tiny terminal thumbnail, so writing full-resolution
/// PNGs (a 9.9 MB GIF can be 1000s of px) just burns decode + disk + chafa time.
/// Capping with ImageMagick's `>` (shrink-only) keeps quality at thumbnail size
/// while slashing extraction/render cost. Frames already ≤ this are untouched.
const GIF_FRAME_MAX_PX: u32 = 480;

/// Max age for entries in the on-disk image cache. Downloaded previews and
/// extracted GIF frames accumulate across sessions with no other eviction
/// (a single large GIF can leave hundreds of frame PNGs), so anything
/// untouched for this long is swept at startup.
const DISK_CACHE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Sweeps the on-disk image cache — downloaded previews plus the extracted
/// GIF frame dirs — deleting entries older than [`DISK_CACHE_MAX_AGE`].
/// Runs once at startup on a background thread; an entry swept too eagerly
/// just re-downloads / re-extracts on demand, so the policy errs cheap.
pub fn sweep_disk_cache() {
    let dir = crate::tui::flows::chat::image_cache_dir();
    let now = std::time::SystemTime::now();
    let expired = |p: &std::path::Path| -> bool {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > DISK_CACHE_MAX_AGE)
    };
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.filter_map(Result::ok) {
            let p = entry.path();
            // `frames/` (the only subdir) is handled below.
            if !p.is_dir() && expired(&p) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    // A frame dir is judged by the freshest PNG inside (the dir's own mtime
    // only reflects extraction); an expired animation is removed whole.
    if let Ok(rd) = std::fs::read_dir(dir.join("frames")) {
        for entry in rd.filter_map(Result::ok) {
            let d = entry.path();
            if !d.is_dir() {
                continue;
            }
            let any_fresh = std::fs::read_dir(&d)
                .map(|it| it.filter_map(Result::ok).any(|f| !expired(&f.path())))
                .unwrap_or(false);
            if !any_fresh {
                let _ = std::fs::remove_dir_all(&d);
            }
        }
    }
}

/// Per-GIF frame directory under the image cache (`…/images/frames/<stem>`).
/// Lives here (not the view) so the worker can compute it when decoding
/// off-thread.
pub fn gif_frame_dir(gif_path: &str) -> std::path::PathBuf {
    let stem = std::path::Path::new(gif_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("gif");
    crate::tui::flows::chat::image_cache_dir()
        .join("frames")
        .join(stem)
}

fn sorted_frame_pngs(dir: &std::path::Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

/// Extracts an animated GIF's frames into `dir` via ImageMagick `-coalesce`
/// (reusing already-extracted frames), returning `None` for a still image, a
/// single-frame GIF, or when ImageMagick isn't available.
/// Wall-clock budget for `chafa` (runs on the **render thread** via the
/// paint path — a wedged run would freeze the whole UI for its duration).
const CHAFA_TIMEOUT_SECS: u64 = 5;
/// Wall-clock budget for ImageMagick `convert` (runs on the **background
/// worker lane** — a wedged run would silently kill the idle auto-refresh
/// and emoji fetch for the rest of the session).
const CONVERT_TIMEOUT_SECS: u64 = 30;

/// Runs `cmd` to completion with a wall-clock deadline, draining stdout /
/// stderr on threads so a full pipe can't deadlock (same pattern as the
/// keybase process runner). On timeout the child is killed and a
/// `TimedOut` error returned — no image tool is allowed to hang the render
/// thread or a worker lane.
fn run_with_timeout(cmd: &mut Command, secs: u64) -> io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(secs);
    let stdout_t = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = s.read_to_end(&mut b);
            b
        })
    });
    let stderr_t = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = s.read_to_end(&mut b);
            b
        })
    });
    let mut poll = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Output {
                status,
                stdout: stdout_t.and_then(|t| t.join().ok()).unwrap_or_default(),
                stderr: stderr_t.and_then(|t| t.join().ok()).unwrap_or_default(),
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "image tool timed out",
            ));
        }
        std::thread::sleep(poll);
        // Adaptive backoff: tight at first so quick runs return fast, then
        // relaxed so a long decode doesn't busy-spin.
        poll = (poll * 2).min(Duration::from_millis(20));
    }
}

pub fn extract_gif_frames(gif: &str, dir: &std::path::Path) -> Option<GifFrames> {
    std::fs::create_dir_all(dir).ok()?;
    let mut frames = sorted_frame_pngs(dir);
    if frames.len() <= 1 {
        let pattern = dir.join("f-%04d.png");
        // `-coalesce` reconstructs each full frame, then `-resize …>` shrinks it
        // to the thumbnail cap (shrink-only) so we don't write/decode full-res.
        let ok = run_with_timeout(
            Command::new("convert")
                .arg(gif)
                .arg("-coalesce")
                .arg("-resize")
                .arg(format!("{GIF_FRAME_MAX_PX}x{GIF_FRAME_MAX_PX}>"))
                .arg(&pattern),
            CONVERT_TIMEOUT_SECS,
        )
        .map(|o| o.status.success())
        .unwrap_or(false);
        if !ok {
            return None;
        }
        frames = sorted_frame_pngs(dir);
    }
    if frames.len() <= 1 {
        return None; // still / single-frame — render as a static image
    }
    // Per-frame delay in centiseconds (`%T`), → milliseconds (min 20ms so a
    // 0-delay GIF doesn't spin at the redraw rate).
    let delays_cs: Vec<u32> = run_with_timeout(
        Command::new("convert")
            .arg(gif)
            .args(["-format", "%T,", "info:"]),
        CONVERT_TIMEOUT_SECS,
    )
    .ok()
    .map(|o| {
        String::from_utf8_lossy(&o.stdout)
            .split(',')
            .filter_map(|s| s.trim().parse::<u32>().ok())
            .collect()
    })
    .unwrap_or_default();
    // Saturating arithmetic: the per-frame delays come from the GIF itself
    // (an attacker-supplied attachment), so a crafted huge centisecond value
    // or frame count must clamp, not overflow.
    let delays_ms: Vec<u32> = (0..frames.len())
        .map(|i| {
            delays_cs
                .get(i)
                .copied()
                .unwrap_or(10)
                .max(2)
                .saturating_mul(10)
        })
        .collect();
    let total_ms = delays_ms
        .iter()
        .fold(0u32, |acc, &d| acc.saturating_add(d))
        .max(1);
    Some(GifFrames {
        frames,
        delays_ms,
        total_ms,
    })
}

fn run_chafa(
    proto: ImgProto,
    path: &str,
    cols: u16,
    rows: u16,
    symbols: &str,
) -> io::Result<Vec<u8>> {
    let size = format!("{cols}x{rows}");
    let mut args: Vec<&str> = vec![
        "-f",
        proto.chafa_format(),
        "--size",
        &size,
        // Quality: max effort on symbol/colour selection, ordered dithering to
        // smooth gradients, and the din99d colour space for perceptual
        // quantisation (the last two are ignored by truecolor kitty/sixel).
        "--work=9",
        "--dither=ordered",
        "--color-space=din99d",
    ];
    // The symbol set is configurable (`image_symbols`) because the best choice
    // is font-dependent — sextants are the safe default; octants are denser but
    // need a Unicode-16 font. Empty → chafa's own default.
    let symbols_arg = format!("--symbols={symbols}");
    if proto == ImgProto::Symbols && !symbols.is_empty() {
        args.push(&symbols_arg);
    }
    args.extend(["--animate", "off", "--polite", "on", path]);
    let output = run_with_timeout(Command::new("chafa").args(&args), CHAFA_TIMEOUT_SECS)
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
    let bytes = cache.cached_bytes(proto, path, area.width, area.height)?;
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
    fn gif_frame_at_loops_over_delays() {
        let g = GifFrames {
            frames: vec!["a".into(), "b".into(), "c".into()],
            delays_ms: vec![100, 100, 100],
            total_ms: 300,
        };
        assert_eq!(g.frame_at(0), "a");
        assert_eq!(g.frame_at(150), "b");
        assert_eq!(g.frame_at(250), "c");
        assert_eq!(g.frame_at(300), "a"); // wraps
        assert_eq!(g.frame_at(450), "b");
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut c: Lru<u32, u32> = Lru::new(2);
        c.insert(1, 10);
        c.insert(2, 20);
        // Touch 1 so 2 becomes the least-recently-used.
        assert_eq!(c.get(&1).copied(), Some(10));
        c.insert(3, 30); // over cap → evicts 2
        assert_eq!(c.map.len(), 2);
        assert_eq!(c.get(&2), None);
        assert_eq!(c.get(&1).copied(), Some(10));
        assert_eq!(c.get(&3).copied(), Some(30));
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
