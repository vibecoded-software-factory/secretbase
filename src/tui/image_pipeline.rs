//! Inline image & animated-GIF rendering pipeline state.
//!
//! # Why this is its own type
//!
//! Inline images are a self-contained concern with **two independent
//! lifecycles** and a fistful of per-frame paint flags. Living as a dozen
//! loose `image_*` / `gif_*` fields on the [`App`](crate::tui::app::App) god
//! object, they were indistinguishable from the inbox, compose, or search
//! state around them — the reader had to know *by convention* which fields
//! moved together. Grouping them here gives the cluster **one reason to
//! change** (Single Responsibility): a tweak to how images are downloaded,
//! decoded, cached, or painted touches this file and its call sites, nothing
//! else. The field names also stop stuttering — `app.images.ready` reads
//! better than `app.image_ready`.
//!
//! # The two lifecycles
//!
//! A cache path (`{conv}-{msg}.ext` for attachments, a URL hash for web
//! media) flows through:
//!
//! 1. **Download** — `to_fetch` → `pending` → `ready` | `failed`. The view
//!    discovers a visible image and pushes its `(message_id, path)` onto
//!    `to_fetch`; the run loop drains it, marks the path `pending`, and the
//!    background worker's reply lands it in `ready` or `failed`.
//! 2. **GIF decode** (only for a `ready` `.gif`) — `to_decode` → `decoding`
//!    → `frames`. Splitting a GIF into frames shells out to ImageMagick and
//!    must never run on the render thread, so a `ready` GIF stays a skeleton
//!    (`decoding`) until the worker fills `frames`.
//!
//! # What deliberately stays *outside* this type
//!
//! The orchestration that *drives* these lifecycles —
//! [`ensure_visible_images`](crate::tui::flows::chat::ensure_visible_images)
//! and [`ensure_pending_gif_decodes`](crate::tui::flows::chat::ensure_pending_gif_decodes)
//! — needs the worker channels and user settings, which live on `App`. Per
//! the project's hard rule, the ports/worker boundary is not something a
//! pure state cluster should reach across. So this type owns the **data and
//! its named state transitions**; the flow layer owns the *decisions* about
//! when to fetch and which lane to use, calling the transition methods here.
//! That split is the whole point: `ImagePipeline` is testable and
//! side-effect-free; the coupling to the worker stays in one honest place.
//!
//! Fields are `pub` on purpose — this is a cohesive record inside one crate,
//! and wrapping `HashSet::contains` in a getter would be Java ceremony, not
//! Rust. The methods exist only where an operation is a *named concept*
//! reused across call sites (or where it centralises the "flag a repaint"
//! invariant), not to hide field access.

use std::collections::{HashMap, HashSet};

use ratatui::layout::Rect;

use crate::ports::UserSettings;
use crate::tui::image::{GifFrames, ImgProto, RenderCache};

/// All state for painting inline images and animated GIFs into the chat.
/// See the [module docs](self) for the two lifecycles and the design split.
pub struct ImagePipeline {
    // ── Resolved output configuration ─────────────────────────────────────
    /// Resolved image protocol, or `None` when images are disabled / no
    /// terminal support. Set once at boot from settings, updated live by the
    /// Settings picker.
    pub proto: Option<ImgProto>,
    /// Cache of chafa render output, keyed by `(path, cols, rows)`.
    pub render_cache: RenderCache,

    // ── Download lifecycle: to_fetch → pending → ready | failed ───────────
    /// Visible images still needing a download, `(message_id, cache path)` —
    /// filled by the view each frame, drained by the run loop, which enqueues
    /// the background fetches.
    pub to_fetch: Vec<(u64, String)>,
    /// Downloads currently in flight (cache paths).
    pub pending: HashSet<String>,
    /// Cache paths that finished downloading and are ready to paint.
    pub ready: HashSet<String>,
    /// Downloads that failed — show the text fallback, don't retry.
    pub failed: HashSet<String>,

    // ── GIF decode lifecycle: to_decode → decoding → frames ───────────────
    /// Visible GIFs that are downloaded but not yet decoded — drained by the
    /// run loop, which enqueues the background decode (mirrors [`Self::to_fetch`]).
    pub to_decode: Vec<String>,
    /// GIF cache paths whose off-thread decode is in flight (de-dupes the
    /// request and drives the "decoding" skeleton).
    pub decoding: HashSet<String>,
    /// Animated-GIF frames by cache path: `Some` once extracted (animated),
    /// `None` when checked and found to be a still image (don't re-extract).
    /// Populated **off-thread** by the worker so a large GIF never blocks the
    /// render thread while it's decoded.
    pub frames: HashMap<String, Option<GifFrames>>,

    // ── Per-frame paint state ─────────────────────────────────────────────
    /// Screen rects + cache paths of the **ready** images visible this frame
    /// (rebuilt every render), painted by the run loop after the text draw.
    pub areas: Vec<(Rect, String)>,
    /// Set when the visible image set / positions changed and the graphics
    /// need repainting (scroll, new download, resize…).
    pub dirty: bool,
    /// Set by the view when an animated GIF is on screen, so the run loop
    /// polls at the animation cadence instead of idling.
    pub animating: bool,
    /// Wall-clock milliseconds since the run loop started — drives GIF frame
    /// selection. Stamped each iteration by the loop.
    pub anim_ms: u64,

    // ── Web-media source map (giphy) ──────────────────────────────────────
    /// `cache path → source URL` for **web media** previews (giphy GIFs
    /// linked in messages). Filled by the view when it reserves image rows;
    /// [`ensure_visible_images`](crate::tui::flows::chat::ensure_visible_images)
    /// routes these to the web fetcher instead of a keybase attachment
    /// download. Session-local.
    pub web_urls: HashMap<String, String>,
}

impl ImagePipeline {
    /// Builds an empty pipeline for the given resolved protocol and chafa
    /// symbol set. Prefer [`Self::from_settings`] at the composition site.
    pub fn new(proto: Option<ImgProto>, symbols: String) -> Self {
        Self {
            proto,
            render_cache: RenderCache::new(symbols),
            to_fetch: Vec::new(),
            pending: HashSet::new(),
            ready: HashSet::new(),
            failed: HashSet::new(),
            to_decode: Vec::new(),
            decoding: HashSet::new(),
            frames: HashMap::new(),
            areas: Vec::new(),
            dirty: false,
            animating: false,
            anim_ms: 0,
            web_urls: HashMap::new(),
        }
    }

    /// Resolves a fresh pipeline from the user's settings — the single place
    /// that maps `image_protocol` / `image_symbols` to pipeline state.
    pub fn from_settings(settings: &UserSettings) -> Self {
        Self::new(
            crate::tui::image::resolve(&settings.image_protocol),
            settings.image_symbols.clone(),
        )
    }

    /// Applies a changed `image_protocol` setting, live. Flags a repaint so
    /// the next frame reflects it.
    pub fn set_protocol(&mut self, proto: Option<ImgProto>) {
        self.proto = proto;
        self.dirty = true;
    }

    /// Applies a changed `image_symbols` setting: rebuilds the render cache
    /// (its keys bake in the symbol set) and flags a repaint.
    pub fn set_symbols(&mut self, symbols: String) {
        self.render_cache = RenderCache::new(symbols);
        self.dirty = true;
    }

    /// Whether a cache path still needs a download — not already ready, in
    /// flight, or known-failed. The fetch queue and the view's reservation
    /// loop share this gate.
    pub fn needs_download(&self, path: &str) -> bool {
        !self.ready.contains(path) && !self.pending.contains(path) && !self.failed.contains(path)
    }

    /// Adopts a cache file that already exists on disk (fetched in an earlier
    /// session) as ready, without a download, and flags a repaint.
    pub fn adopt_cached_file(&mut self, path: String) {
        self.ready.insert(path);
        self.dirty = true;
    }

    /// Records a finished background download: the path becomes `ready` on
    /// success or `failed` otherwise, and a repaint is flagged. Centralises
    /// the download-complete transition (and its `dirty` invariant) so every
    /// caller can't forget a step.
    pub fn on_download_finished(&mut self, path: String, ok: bool) {
        self.pending.remove(&path);
        if ok {
            self.ready.insert(path);
        } else {
            self.failed.insert(path);
        }
        self.dirty = true;
    }

    /// Whether a `ready` GIF still needs decoding — not already in flight and
    /// not yet in `frames`.
    pub fn needs_decode(&self, path: &str) -> bool {
        !self.decoding.contains(path) && !self.frames.contains_key(path)
    }

    /// Records a finished off-thread GIF decode: `Some(frames)` for an
    /// animated GIF, `None` for a still / single-frame one. Clears the
    /// `decoding` flag and flags a repaint so the skeleton gives way.
    pub fn on_decode_finished(&mut self, path: String, frames: Option<GifFrames>) {
        self.decoding.remove(&path);
        self.frames.insert(path, frames);
        self.dirty = true;
    }
}
