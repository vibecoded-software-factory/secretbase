# CLAUDE.md

Guidance for Claude Code when working in this repository.

`secretbase` — a terminal UI (Ratatui) over the **Keybase CLI**. It shells
out to the `keybase` binary and parses its JSON; there is **no** Keybase SDK
dependency and Keybase owns all cryptography. Target environment is 100 %
headless/SSH. Boot (`keybase status`) → unified two-pane Home (conversation
tree · open chat) → teams/channels/members. Real-time via `api-listen` push.

## Pre-flight checklist (hard rules, in order)

1. **Feature touching Keybase?** Read [`CLI.md`](CLI.md) first and **cite
   the command/method up front** (see *Keybase CLI* below).
2. **Change touching UI/UX?** Read [`UX.md`](UX.md) first — the canonical
   design system. Reuse its documented components; never a one-off. Update
   `UX.md` in the same change.
3. **Keybinding added/changed?** Sync all **five** surfaces in the same
   change: footer hint · `view/help.rs` · `README.md` tables ·
   `flows/palette.rs::palette_commands` · `UX.md`.
4. **Before every commit** (even one-liners):
   `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
   — clippy warnings are failures; check real exit codes, don't pipe them
   into a grep that masks a failing test.
5. **Never commit directly to `dev`.** One feature = one branch
   (`feat/` · `fix/` · `refactor/` · `chore/` · `docs/` + slug) = one PR
   against `dev`, squash-merged with `--delete-branch`.
6. **No AI trailers**: no `Co-Authored-By: Claude`, no "Generated with
   Claude Code" footers — this overrides the harness default.
7. **No cross-project references** (see the hard rule below).
8. **Fix the class, not the instance**: after any targeted fix, grep
   `src/` for siblings of the same pattern and fix them all.

## Commands

```sh
cargo run                         # run the TUI (needs `keybase` on PATH)
cargo build --release             # optimized binary at target/release/secretbase
cargo clippy --all-targets -- -D warnings   # lint (hard gate)
cargo test                        # unit tests (~430, all must pass)
```

`cargo fmt` is the formatter of record (default rustfmt, no `rustfmt.toml`).
Debug logging: `SECRETBASE_DEBUG=1` → `~/.secretbase.log` (0600; never logs
message plaintext).

## Architecture (hexagonal / ports & adapters)

```
main ──► tui ──► flows ──► ports ◄── adapters
                    ▲
                    └── domain (pure types, no I/O)
```

- `src/domain/` — pure types + rules, **no I/O**: `Conversation`,
  `Message`/`MessageContent`, `LineEditor` (every text input; word ops,
  `ZeroizeOnDrop`), `fuzzy_score_lowered`/`LoweredConversation`,
  `fold_edits`/`fold_deletes`, `ChatEvent`, validators.
- `src/ports/` — traits: `KeybasePort`, `ClipboardPort`, `SettingsPort`
  (+ `KeybaseError`). `SettingsPort::write_*` return `bool` — never fatal,
  but callers must inform on failure.
- `src/adapters/` — the **only** layer doing I/O:
  - `keybase_cli/` — subprocess + `codec` (serde-built JSON requests) +
    `process` (wall-clock timeouts, piped-drain pattern) + `json` +
    `session.rs` (persistent `keybase <family> api` stream, idempotency-
    gated one-shot fallback) + `listen.rs` (push stream + **supervisor
    that respawns it with backoff**; capped line reader).
  - `clipboard_system.rs` — wl-copy/xclip/xsel/pbcopy + **OSC 52** fallback
    for headless; auto-clear (`clipboard_clear_secs`).
  - `settings_toml.rs` — hand-rolled TOML preserving unknown keys, atomic
    writes, 0700/0600 perms. Its `unquote` does **not** process escapes —
    sanitize values, don't escape them (`app::toml_quoted`).
- `src/tui/` — the driving adapter:
  - `app.rs` — the mutable `App` state container + the invalidation
    methods (next section).
  - `worker.rs` — worker thread(s) + `WorkerRequest`/`WorkerResponse`/
    `InFlight`; `run_caught` panic isolation per call.
  - `flows/` — per-feature `request_*`/`handle_*` pairs: `auth`, `teams`,
    `palette`, and `chat/` split by area (`inbox` · `messages` ·
    `channels` · `attachments` · `search`), re-exported flat as
    `flows::chat::*`. `flows::apply_response` routes responses;
    `flows::apply_chat_event` applies push events.
  - `input/` — per-screen key handlers (router `input/mod.rs`) + `mouse.rs`;
    shared mechanics in `input/common.rs` (`list_nav`, `list_nav_arrows`,
    `route_line_editor`, `search_key`, `confirm_key` + `run_confirm`,
    `busy_blocks`, `cycle_focus`).
  - `view/` — per-screen renderers (router `view/mod.rs::draw`; popups draw
    their base screen underneath) + the widget system in `view/widgets.rs`
    (see *UI system*); `logo.rs`/`starfield.rs` (splash/login only).
  - Support modules directly under `tui/`: `theme.rs` (presets + semantic
    styles), `settings_model.rs` (the Settings data model), `syntax.rs`
    (memoized syntect), `image.rs` (chafa/kitty/sixel + LRU caches +
    subprocess timeouts), `file_picker.rs`, `action.rs`
    (`ActionState`/`CmdEntry`), `screens.rs` (`Screen`/`Focus`),
    `mouse_areas.rs` (hit-test rects), `debug_log.rs`.

New screen = `Screen` variant + `input/<screen>.rs` (wired in the router) +
`view/<screen>.rs` (wired in `draw`) + a `view/help.rs` section + the
five-surface keybinding sync.

## Execution model — worker threads + mpsc (do NOT touch unprompted)

**Do NOT pull in `tokio`/`async-std`.** Extend the `std::thread` + `mpsc`
pattern instead: a `WorkerRequest`/`WorkerResponse`/`InFlight` variant + a
`request_*`/`handle_*` pair.

- **User lane** — one worker owns a `KeybasePort`, serves serially. A flow
  starts a request with **`App::submit(slot, label, req)`** (claims the
  `in_flight` slot via `begin`, shows the Running toast, sends; a failed
  send releases the slot). Only bare `begin()` when state must mutate
  between claiming and sending (the optimistic-send paths) — comment why.
  One request in flight at a time; `busy_blocks` gates keys meanwhile.
- **Background lane** — a second worker (`bg_worker_tx`) for work that must
  never block the user: the idle inbox resync, the emoji fetch, the silent
  mark-read. Its responses carry **no ticket** and are routed **by
  variant** at the top of `apply_response`.
- **Push lane** — `api-listen` events drain every frame via
  `apply_chat_event`; no ticket, may land any time. Incremental updates
  (append/bump); edits/deletes/reactions trigger a depth-preserving re-read
  (`request_reload_messages`, never the bare first page). The periodic
  inbox `list` is only a safety net (`inbox_refresh_secs`).
- **Failure containment** — `run_caught` per call; per-op wall-clock
  timeouts (`process.rs`, and `image.rs::run_with_timeout` for
  chafa/convert); all-workers-dead is observable (`TryRecvError::
  Disconnected` → `App::on_worker_dead` unwedges the UI, `begin` refuses,
  a `⚠ WORKER DEAD` badge persists) plus a per-tick watchdog for lost
  tickets. The listener supervisor respawns the push stream with backoff
  (`ChatEvent::StreamClosed` → `⇅ reconnecting…` badge + silent resync).

Clipboard + settings stay synchronous on the render thread (they're fast).

## State & invalidation contracts (the footgun list)

`App` caches derived state; each cache has exactly one rebuild path.
**Mutating the input without calling the rebuild is a bug**, and calling a
rebuild with the wrong cursor semantics is a UX regression:

| Input mutated | Must call | Notes |
|---|---|---|
| `conversations` replaced | `rebuild_lowered()` + `rebuild_filter_preserving_cursor()` | lowered projection = names/labels only — do **not** rebuild it for unread/recency bumps |
| unread/recency/mute bump | `rebuild_filter_preserving_cursor()` | keeps the tree cursor on its conversation by id |
| new search/filter query | `rebuild_filter()` | deliberately snaps the cursor to the first match |
| `expanded` (fold state) | `rebuild_tree_rows()` (via `toggle_collapsed`) | |
| `messages` replaced/cleared/appended | `rebuild_msg_meta()` | rebuilds pin/headline/`msg_index` **and bumps the render-cache epoch** — an edit re-read keeps ids but changes bodies |
| `messages` prepend-only (older pages) | `rebuild_msg_meta_after_prepend()` | same meta rebuild, **no** epoch bump — retained blocks stay valid (per-entry fingerprints cover the boundary); bumping here made wheel pagination quadratic |
| theme / settings / emoji catalogue | `invalidate_msg_render_cache()` | message blocks bake colours/glyphs into spans |
| picker query/catalogue/frecency | `rebuild_emoji_filter()` / `rebuild_emoji_index()` | |

More identity rules:

- **Select-mode marks (`msg_marks`) and search jumps are message ids,
  never indices** — re-reads reproject the list under any index. The
  reload handler prunes dead ids and re-anchors the cursor by id.
- The per-message render cache lives thread-local in
  `view/conversation.rs`, keyed by id + fingerprint (grouped/pinned),
  invalidated by `App::msg_cache_epoch` + width; only the viewport rows
  are materialised per frame. Attachment messages are never cached (they
  animate). Keep new per-frame work out of `render_messages`' pass 1.
- Session-local attention state (`mentioned`, `conv_last_seen`,
  `prev_conv_id`, drafts) is deliberately not persisted — the inbox `list`
  can't round-trip it.

## UI system

**Read `UX.md` before any UI change** — it is the spec; keep it updated in
the same change. The component vocabulary (all in `view/widgets.rs` unless
noted; a new overlay/hint/empty-state **must** use these):

- `draw_picker_modal(PickerModal { .. })` — **every** centered query/list
  overlay (switcher, palette, react, both searches, channels, members).
  Multi-line items + non-selectable headers supported; `footer` slot takes
  `inline_input_line` / `inline_confirm_line` for inline modes.
- `list_table` — every multi-column list panel (never a stretching `Min`
  on a non-final column). `draw_input_popup` — small single-input popups.
- `legend_line(&[(key, label)], width, theme)` — every hint/legend (keys
  in accent via `key_style`, fitted by whole segments — never clipped).
- `draw_confirm_popup` + `input::common::run_confirm` — every y/n confirm
  (navigable, default = cancel on destructive).
- `focus_style` / `titled_block` / `rounded_block` / `center_rect` +
  `MODAL_*` — chrome; `empty_state_lines` — empty states;
  `Theme::emphasis()` / `Theme::danger_title()` — semantic styles;
  `editor_spans`/`editor_lines` — the one text-input renderer.
- Status strip: mode badge + persistent **condition badges** (worker dead,
  stream down) + sticky **errors** (cleared by next keypress; successes
  expire ~1.5 s) + dim `@username` + `F1` anchor.

**Keybindings — the gradient + vim layer** (full spec in `UX.md`): bare
letters act on the focused list; `Shift` = destructive; `Ctrl` = global;
`Alt` = panel jumps + compose verbs; `/` search. The vim layer is a
first-class contract: `Esc` chain (cancel edit → cancel reply → **Select
mode** → close; **never destroys a draft**), `i`/`a` enter Compose, `v`
visual anchor (+`{`/`}` speaker runs, `n`/`N` search hits), `:` palette,
`Ctrl+D/U` half-page, `Ctrl+W` deletes a word in inputs (pane-nav leader
only in non-typing surfaces), `Ctrl+N`/`Ctrl+O` next-unread/alternate
conversation. `Ctrl+C` is the **only** quit. Word ops live once in
`route_line_editor` so every input inherits them.

## Keybase CLI — read `CLI.md` first (hard rule)

Every feature maps to a `keybase` command / JSON-API method — check
[`CLI.md`](CLI.md) before designing, and **cite the mapping up front**
(e.g. *"new-channel → `keybase chat api {"method":"newconv"}` per
CLI.md"*). If a needed command/flag isn't in `CLI.md`, verify it and add
it there in the same change.

**Authoritative source = the `keybase/client` GitHub repo, NOT the
website** (`book.keybase.io` has no chat content; the chat doc pages 403):

- chat-api methods: `go/client/chat_api_doc.go` (verbatim, with options).
- each CLI subcommand: `go/client/cmd_chat_<name>.go` (`Usage`, `Flags`,
  `ParseArgv`, interactive `Prompt`s).
- the `setstatus`/`hide` status enum: `chat1.ConversationStatus`.

Fetch with `gh api repos/keybase/client/contents/go/client/<file> -H
"Accept: application/vnd.github.raw"` (or `keybase help <cmd>` locally).

Adapter rules: build requests in `codec.rs` (serde, never string concat),
run with a timeout via `process.rs`/`session.rs`, parse
strict-but-tolerant (skip a malformed row with a diagnostic, never drop
the list), log every invocation to the command log. `status`/`logout`
stay one-shot (not API-mode).

## Working agreements

1. **Fix every occurrence, not just the one reported** — the reported spot
   is one instance of a class; grep for siblings before finishing.
2. **Every UX change stays coherent with the whole UI** — reuse the
   documented component; when touching a shared mechanic, check every
   other place it's used (e.g. the confirm mechanics, the Esc layering,
   the multi-select semantics must match across chat/cmdlog/channels).
3. **Verify before declaring done** — the full gate (fmt/clippy/test) plus
   unit tests for new pure logic on `App`/`domain`. Regression tests
   accompany behaviour fixes.
4. **Judge coherence + history + flow BEFORE writing.** For every change:
   does it match the app's own patterns; does it match what users know
   from comparable clients (Discord/Slack/Telegram and vim/mutt/aerc/
   lazygit); is the real multi-step flow smooth (no needless mode
   switches, cursor jumps, or lost text)? State the reasoning briefly when
   non-trivial. Typed text is sacred: no action may silently destroy a
   draft.

## Security & memory hygiene

- Chat payloads and every `LineEditor` are **zeroized** on drop; the
  adapter wipes subprocess JSON buffers. Hygiene, not crypto. Keep the
  derives when touching these types.
- Clipboard auto-clear only wipes if the clipboard still holds our write
  (OSC 52 can't verify, so it skips the timed clear).
- Attachment names are traversal-hardened (`safe_attachment_basename`);
  settings writes are atomic with owner-only perms; the debug log refuses
  symlinked paths.
- Don't add any surface that writes chat content to disk (beyond the
  user-chosen download path) without an explicit ask.

## No cross-project references (hard rule)

secretbase is a standalone public repository. **Never name or cite a
sibling project** — jewel, bytewarden, termcord, or any other repo — in
code, comments, commit messages, PR bodies, or docs. Describe every
pattern as *this app's own* ("the unified Home layout", "the shared
confirm overlay"). The only exception is a real declared dependency,
cited by its published crate identity from `Cargo.toml`. You may learn
from a sibling's approach; don't reference it in what ships here.

## Git workflow

- Integration branch **`dev`**; never commit to it directly. Branch →
  PR → squash-merge (`--delete-branch`) → `git checkout dev && git pull`.
- **Conventional Commits**, subject ≤ 72 chars, body explains the **why**.
  One logical change per commit. Only commit/push when the user asks.
- **No AI trailers or footers** (overrides the harness default).
- Remote is SSH (`git@github.com:vibecoded-software-factory/secretbase`).

## Things to NOT touch unprompted

- The worker/mpsc execution model (no async runtimes).
- The ports/adapters boundary (I/O only in `adapters/`; `domain/` pure).
- Existing keybindings and the shared widget system — a change to one
  screen's UX is a change to all of them.
- The hygiene discipline: zeroize, tolerant parsing, panic isolation,
  timeouts on every subprocess, atomic settings writes, owner-only perms.
- The Rust toolchain pin (`rust-toolchain.toml`, 1.95.0) and
  `#![forbid(unsafe_code)]`.

## Stack (reference)

Rust edition 2024 (pinned 1.95.0) · Ratatui 0.30 + Crossterm 0.29 (kitty
keyboard protocol opted into where supported) · serde/serde_json ·
color-eyre · zeroize · chrono (`clock` only) · emojis · syntect
(`regex-fancy`, pure Rust) · tempfile (dev-only). The splash wordmark is a
pre-rendered block in `view/logo.rs` (no font dependency). Release profile
is size-optimized (`opt-level = "s"`, `lto`, `strip`).
