//! Emoji catalogue — the reaction/insert picker's data, lookup index and
//! per-session frecency ranking.
//!
//! # Why this is its own type
//!
//! Like [`ImagePipeline`](crate::tui::image_pipeline), this is a cohesive
//! cluster that had been scattered as loose `emoji*` fields on the
//! [`App`](crate::tui::app::App) god object. It bundles three things that
//! only ever change together:
//!
//! * the **catalogue** (`all`) — the bundled standard Unicode set seeded at
//!   boot, later merged with the team's custom emojis from `emojilist`;
//! * the **lookup index** (`index`) — `alias`/`display` → position, so a
//!   stored reaction resolves to its glyph in O(1) instead of a linear scan
//!   of ~1.9k entries per chip per frame;
//! * the **picker projection** (`filtered`) + **frecency** (`uses`) — the
//!   query-filtered, most-used-first row order the reaction picker renders.
//!
//! Grouping them gives the concern one home and one reason to change, and
//! lets the render/pick hot paths read a small, obviously-consistent record.
//!
//! # What deliberately stays *outside* this type
//!
//! * The **fetch** ([`request_emojis`](crate::tui::flows::chat::request_emojis))
//!   runs on the background worker lane and needs the channels on `App`, so
//!   it stays in the flow layer — same split as the image pipeline. It calls
//!   [`EmojiCatalog::set`] with the merged list; the catalogue holds no I/O.
//! * The **picker query** lives on `App` (`react`, the shared `LineEditor`),
//!   not here — the catalogue is query-agnostic. [`EmojiCatalog::rebuild_filter`]
//!   takes the query as a parameter, and `App::rebuild_emoji_filter` is the
//!   thin bridge that passes `app.react` in. So the coupling to the picker
//!   input lives in exactly one place, and the catalogue stays reusable.
//!
//! Fields are `pub` (idiomatic within the crate); `filtered` is private
//! because its only valid producer is [`EmojiCatalog::rebuild_filter`] —
//! exposing it mutably would invite a write that skips the frecency sort.

use std::collections::HashMap;

use crate::domain::{Emoji, emoji};

/// The emoji picker's catalogue, lookup index and frecency ranking. See the
/// [module docs](self) for the design split.
pub struct EmojiCatalog {
    /// The full catalogue: the bundled standard set, plus the team's custom
    /// emojis once [`Self::set`] merges them in.
    pub all: Vec<Emoji>,
    /// Whether the `emojilist` fetch has completed (so it runs at most once).
    pub loaded: bool,
    /// Whether an `emojilist` fetch is in flight (de-dupes the request).
    pub loading: bool,
    /// Lookup index over [`Self::all`]: both the `alias` and the `display`
    /// glyph map to the entry's position (earliest catalogue entry wins).
    /// Rebuilt on every catalogue change — reaction chips resolve through
    /// this instead of scanning the catalogue per chip per frame.
    pub index: HashMap<String, usize>,
    /// Cached reaction-picker rows: indices into [`Self::all`] surviving the
    /// current query, frecency-sorted. Rebuilt by [`Self::rebuild_filter`] on
    /// keystroke / catalogue change — never per frame. Private: its only
    /// valid producer is `rebuild_filter` (which applies the frecency sort).
    filtered: Vec<usize>,
    /// Per-alias reaction usage this session — floats the most-used emojis to
    /// the top of the picker (Discord-style frecency).
    pub uses: HashMap<String, u32>,
}

impl Default for EmojiCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl EmojiCatalog {
    /// Builds the catalogue seeded with the bundled standard set, so the
    /// picker has content and reactions resolve to glyphs immediately; the
    /// `emojilist` fetch later merges the team's custom emojis on top.
    pub fn new() -> Self {
        let mut catalog = Self {
            all: emoji::standard(),
            loaded: false,
            loading: false,
            index: HashMap::new(),
            filtered: Vec::new(),
            uses: HashMap::new(),
        };
        catalog.rebuild_index();
        catalog.rebuild_filter("");
        catalog
    }

    /// Replaces the catalogue (the caller merges standard + team custom
    /// emojis) and rebuilds the lookup index. Does **not** refilter — the
    /// caller re-runs [`Self::rebuild_filter`] with the live picker query,
    /// since the query lives on `App`.
    pub fn set(&mut self, all: Vec<Emoji>) {
        self.all = all;
        self.rebuild_index();
    }

    /// Rebuilds [`Self::index`] from [`Self::all`]. Called after every
    /// assignment to the catalogue (via [`Self::new`] / [`Self::set`]).
    pub fn rebuild_index(&mut self) {
        self.index = HashMap::with_capacity(self.all.len() * 2);
        for (i, e) in self.all.iter().enumerate() {
            self.index.entry(e.alias.clone()).or_insert(i);
            self.index.entry(e.display.clone()).or_insert(i);
        }
    }

    /// Resolves a stored reaction key — a `:shortcode:` or a raw glyph — to
    /// its catalogue entry: matches by alias (colons trimmed) or by display
    /// glyph, earliest catalogue entry winning, in O(1) via [`Self::index`].
    pub fn for_reaction(&self, key: &str) -> Option<&Emoji> {
        let alias = key.trim_matches(':');
        let a = self.index.get(alias).copied();
        let d = self.index.get(key).copied();
        let idx = match (a, d) {
            (Some(x), Some(y)) => x.min(y),
            (Some(x), None) | (None, Some(x)) => x,
            (None, None) => return None,
        };
        self.all.get(idx)
    }

    /// Indices into [`Self::all`] matching the reaction-picker query — the
    /// cached result of [`Self::rebuild_filter`]. The picker view reads this
    /// per frame; the filter/sort runs only on a keystroke / catalogue change.
    pub fn filtered(&self) -> &[usize] {
        &self.filtered
    }

    /// Recomputes [`Self::filtered`] from the picker `query` (case-insensitive
    /// substring on the keywords; all when empty), frecency-sorted. Call
    /// whenever the query, the catalogue, or the usage ranking changes.
    pub fn rebuild_filter(&mut self, query: &str) {
        let q = query.trim().to_lowercase();
        let mut idx: Vec<usize> = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.keywords.contains(&q))
            .map(|(i, _)| i)
            .collect();
        // Most-used first; `sort_by` is stable, so ties keep catalogue order.
        idx.sort_by(|&a, &b| {
            let ua = self.uses.get(&self.all[a].alias).copied().unwrap_or(0);
            let ub = self.uses.get(&self.all[b].alias).copied().unwrap_or(0);
            ub.cmp(&ua)
        });
        self.filtered = idx;
    }

    /// Records one use of `alias` (the chosen reaction), floating it up the
    /// picker next time it's rebuilt.
    pub fn bump_use(&mut self, alias: &str) {
        *self.uses.entry(alias.to_string()).or_insert(0) += 1;
    }
}
