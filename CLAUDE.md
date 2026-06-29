# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Keybase CLI — read `CLI.md` first (hard rule)

**Before any request to add or edit functionality, read
[`CLI.md`](CLI.md)** — the mirror of the Keybase CLI docs. secretbase is
a thin wrapper over the `keybase` binary, so every feature maps to a
`keybase` command / JSON-API method; check the exact command, subcommand
and flags there before designing or wiring anything.

**Cite the command up front.** Before implementing a feature, state which
`CLI.md` command / JSON-API method it maps to (e.g. *"new-channel → covered
by `keybase chat api {"method":"newconv"}` per CLI.md"*). If the feature
needs a command/flag not in `CLI.md`, say so, verify it, and add it to
`CLI.md` in the same change.

**Authoritative source = the keybase `client` GitHub repo, NOT the
website.** `book.keybase.io/docs/cli` has *no* chat content, and the chat
doc pages (`/docs/chat/api`, …) return HTTP 403 to the fetcher — do not
rely on them. Verify against the source in `keybase/client` via `gh api`:

- **chat-api methods** are documented verbatim in
  `go/client/chat_api_doc.go` (every `{"method": …}` with options).
- each **CLI subcommand** is `go/client/cmd_chat_<name>.go` (read its
  `Usage`, `Flags`, `ParseArgv`, and any interactive `Prompt`).
- the `setstatus`/`hide` status enum is `chat1.ConversationStatus`.

Fetch with: `gh api repos/keybase/client/contents/go/client/<file> -H
"Accept: application/vnd.github.raw"`. (Or `keybase help <cmd>` on a real
install.)

## UX/UI — read `UX.md` first (hard rule)

**Before any UI edit or new feature, read [`UX.md`](UX.md)** — it is the
canonical design system (screen layout, the shared `view::widgets`
components, chrome, popups, keybinding conventions, theme). Build new UI
by reusing the documented components, never a one-off. **Keep `UX.md`
updated before every push** when a change touches UX/UI (new screen, new
pattern, changed convention): update the spec in the same change and apply
it everywhere.

The design system: the `split_main` vertical stack, the
top identity bar, the single `widgets::list_table` renderer, the navigable
`widgets::draw_confirm_popup`, the `LineEditor` + `editor_spans` text-input
model, the shared `input::common` mechanics, the rolling command log, and a
restrained night-sky splash. When in doubt, reuse the documented component.

## Working agreements (READ FIRST)

1. **Fix every occurrence, not just the one reported.** When the user
   reports a problem (a bug, a wording issue, a layout glitch, a missing
   keybinding, …), treat the reported spot as one *instance* of a class.
   Before finishing, grep the whole app for the same pattern and fix it
   everywhere it appears.
   - Practical step: after a targeted fix, `grep` for the literal/string/
     pattern across `src/` and confirm there are no siblings left.

2. **Every UX change must stay coherent with the rest of the UI.** This
   app has established, repeated patterns; a change to one screen should
   match all the others, and ideally reuse the same component.
   - Signed-in screens → `view::mod::split_main` stack (identity · header ·
     body · cmdlog · status). Panels → `view::mod::titled_block(title,
     focused, app)`. Popups → `widgets::center_rect` / `rounded_block`.
   - Multi-column lists → `widgets::list_table` (header + content-sized
     columns via `col_width` + `▶` + persisted scroll + `· X of Y` title via
     `list_title`). Never a stretching `Min` on a non-final column.
   - Text inputs → `domain::LineEditor` rendered with `widgets::editor_spans`,
     keys routed via `input::common::route_line_editor` / `search_key`. The
     inbox search box uses `widgets::draw_search_box`.
   - Bottom strip → `widgets::draw_status_strip` (feedback when busy, else the
     footer hint + `F1 help` right). Command log → `widgets::draw_cmd_log`.
   - Confirmations → `widgets::draw_confirm_popup` + `input::common::confirm_key`
     (navigable y/n, default highlight = cancel for destructive ops).
   - Help popup → `tui::view::help::draw` lists the shortcuts per screen.
     Any new screen/keybinding must be added to the matching section.
   - List filtering follows the `App::filtered_cache: Vec<usize>` +
     `search: LineEditor` + `rebuild_filter()` convention, ranked with
     `domain::search::fuzzy_score_lowered` over the pre-lowercased
     `LoweredConversation` projection. The tree cursor (`tree_selected`)
     indexes the visible `tree_rows()`, which are built from the **filtered**
     cache, never the raw `conversations` vec.
   - Global keys are consistent across screens (`/` focus search, `Esc`
     back, `F1` help, `Tab` cycle focus). **Only `Ctrl+C` quits**
     (everything else is free for navigation / type-to-search). Per-list
     actions use the **`Alt+<letter>`** convention (e.g. `Alt+N` new,
     `Alt+C` copy, `Alt+M` mark read, `Alt+T` teams) — see `UX.md`.

3. **Verify before declaring done.** Run `cargo build`, `cargo clippy
   --all-targets -- -D warnings` (must be warning-free) and `cargo test`
   after every change. Add/adjust unit tests for new pure logic on
   `App`/`domain`.

## What this is

`secretbase` — a terminal UI (Ratatui) over the **Keybase CLI**. Flow:
boot (`keybase status`) → inbox (sidebar filters + search + conversation
list) → conversation detail (read history + compose / edit / delete /
react / pin) → teams. It shells out to the `keybase` binary and parses its
JSON (`keybase chat api` / `keybase team api`); there is **no** Keybase
SDK dependency. Keybase owns all cryptography — this app only wraps the
CLI.

## Before every commit (no exceptions)

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings   # must be warning-free
cargo test
```

Run this even on a one-line or comment-only change. `cargo fmt` is the
formatter of record (default rustfmt; there is no `rustfmt.toml`, so don't
hand-format against the default style). Clippy is a hard gate — warnings
are failures here, not suggestions. Keep the `README.md` keybinding tables
and the `view/help.rs` popup in sync when shortcuts change.

## Stack

- **Rust, edition 2024**, toolchain pinned in `rust-toolchain.toml`
  (`1.95.0`, with `clippy` + `rustfmt` + `llvm-tools-preview`). Don't bump
  the channel as a side effect.
- `#![forbid(unsafe_code)]` at the crate root (`lib.rs` and `main.rs`) —
  no `unsafe`, ever.
- **Ratatui 0.30** + **Crossterm 0.29** for the TUI and terminal events.
- **Serde / serde_json** to parse `keybase … api` JSON output.
- **color-eyre** for error reports, **figlet-rs** for the splash wordmark
  (bundled `src/tui/assets/slant.flf`, no system `figlet` needed),
  **zeroize** to wipe chat/credential-bearing buffers, **chrono**
  (`clock`, no default features) for local-timezone chat timestamps,
  **emojis** for the full standard Unicode emoji set (reaction picker),
  **syntect** (`regex-fancy`, no oniguruma C FFI — stays pure-Rust and
  `forbid(unsafe_code)`-clean) for fenced-code syntax highlighting
  (`tui::syntax`).
- `tempfile` is a dev-dependency only (test fixtures).
- Release profile is size-optimized (`opt-level = "s"`, `lto`, `strip`).

## Execution model — worker thread + mpsc (do NOT touch unprompted)

The keybase port lives on a **single worker thread** that owns it and
serves requests serially over `mpsc`; the render thread never blocks on a
`keybase` call (`tui/worker.rs`). The flow:

1. A `request_*` builder (in `tui/flows/`) validates input, stashes a
   `worker::InFlight` ticket on `App::in_flight`, sets a `Running` toast,
   and sends a `WorkerRequest` on the worker channel.
2. The worker runs the blocking `keybase` call off-thread and sends a
   `WorkerResponse` back. Each call is wrapped in `run_caught`
   (`catch_unwind`) so a panic in one can't kill the worker — it surfaces
   as `KeybaseError::Internal`.
3. The run loop (`tui/mod.rs`) drains the response channel every frame and
   routes each response through `flows::apply_response`, which consumes the
   `in_flight` ticket and dispatches to the owning `handle_*`. The spinner
   animates throughout; `Ctrl+C` stays instant.

Only one user request is in flight at a time (`App::in_flight:
Option<_>`). Two layers enforce this: `input::common::busy_blocks` gates
keys while busy (so the user can't queue a second), and every `request_*`
claims the slot via **`App::begin(slot)`** instead of assigning `in_flight`
directly — `begin` refuses (and logs) if one is already in flight, so a
*programmatic* race (e.g. the idle auto-refresh vs. opening a conversation)
can't overwrite the slot and desync the `in_flight` ↔ response ordering
(which surfaced as a "dispatch mismatch"). Each call still has a
per-operation timeout in `adapters/keybase_cli/process.rs`.

Clipboard + settings stay **synchronous** on the render thread (they're
fast).

## Real-time push (`keybase chat api-listen`)

Alongside the request/response worker, a **third lane** delivers push
updates: a long-lived `keybase chat api-listen --convs --hide-exploding`
process (`adapters/keybase_cli/listen.rs`) whose reader thread parses each
JSON line into a `domain::ChatEvent` and sends it on a channel. `main.rs`
owns the listener guard (its child is killed on exit) and hands only the
`Receiver<ChatEvent>` to the TUI, which drains it every frame in the run
loop and applies events via `flows::apply_chat_event` — no `InFlight`
ticket, so a push can land at any time without touching the user's slot.
Events update state **incrementally** (append a message to the open
conversation, bump a conversation in the inbox) instead of re-fetching;
edits/deletes/reactions trigger a re-read to reproject. Because real-time
arrives via push, the periodic inbox `list` is only a **safety-net resync**
(`inbox_refresh_secs`, default 180 s). The listener is push-only (no stdin);
see `CLI.md` for the event shapes and the CLI's limits (no typing/read-state
over `api-listen`).

**Do NOT pull in `tokio`/`async-std`.** Extend the `std::thread` + `mpsc`
worker pattern: add a `WorkerRequest`/`WorkerResponse`/`InFlight` variant
and a `request_*`/`handle_*` pair.

## No Keybase SDK

All Keybase access is the `keybase` CLI binary spawned as a subprocess
(`adapters/keybase_cli/`). Adding functionality means a new CLI
invocation, not an SDK crate: build the JSON request in `codec.rs`, run
with a timeout in `process.rs`, parse in `mod.rs`/`json.rs`.

For latency, all `chat`/`team` API calls funnel through the two
chokepoints `run_api`/`run_api_raw`, which run them over a **persistent
`keybase <family> api` stream** (`adapters/keybase_cli/session.rs`,
`ApiSession`) instead of spawning a fresh process per call — amortising
the binary's fork+exec and `keybased` connection. The worker is the
single serial caller, so request↔response ordering is implicit. Any
protocol failure (dead process, broken pipe, a first-call probe timeout)
**transparently falls back to a one-shot spawn** and disables the stream
after repeated failures, so it is never less correct than the old path.
`status`/`logout` are not API-mode and stay one-shot. Parse
strict-but-tolerant — skip a malformed conversation/team row (collecting a
diagnostic), don't drop the whole list. Every invocation is appended to
the in-app command log.

## Architecture (hexagonal / ports & adapters)

```
main ──► tui ──► flows ──► ports ◄── adapters
                    ▲
                    └── domain (pure types, no I/O)
```

- `src/domain/` — pure types and rules, no I/O (e.g. `Conversation`,
  `Message`/`MessageContent`, `TeamMembership`, `StatusFilter`,
  `fuzzy_score_lowered`/`LoweredConversation`, validators).
- `src/ports/` — trait abstractions: `KeybasePort`, `ClipboardPort`,
  `SettingsPort`, `KeybaseError`.
- `src/adapters/` — the only layer allowed to do I/O: `keybase_cli/`
  (subprocess + `codec` + `process` + `json`), `clipboard_system.rs`
  (`wl-copy`/`xclip`/`xsel`/`pbcopy`), `settings_toml.rs` (hand-rolled
  TOML that preserves unknown keys, atomic writes, owner-only perms).
- `src/tui/` — the driving adapter:
  - `app.rs` — global mutable `App` state container (incl. worker channels
    + `in_flight`).
  - `worker.rs` — the worker thread + `WorkerRequest`/`WorkerResponse`/
    `InFlight` enums.
  - `action.rs` — `ActionState` (Idle/Running/Done/Error) + `CmdEntry`.
  - `screens.rs` — `Screen`, `Focus` enums.
  - `flows/` — per-feature `request_*`/`handle_*` pairs (`auth`, `chat`,
    `teams`, `copy`). `flows::apply_response` routes each `WorkerResponse`
    to its `handle_*`.
  - `input/` — per-screen keyboard handlers (wired in `input/mod.rs`) +
    `mouse.rs`; shared mechanics in `input/common.rs` (`clamp_move`,
    `cycle_focus`, `busy_blocks`, `route_line_editor`, `search_key`,
    `confirm_key`). New handlers delegate here.
  - `view/` — per-screen Ratatui renderers (router in `view/mod.rs::draw`,
    which owns `split_main` + `titled_block`) + the shared `list_table` and
    chrome in `view/widgets.rs`; `theme.rs`, `logo.rs`, `starfield.rs`
    (restrained, splash/login only).
  - `domain::LineEditor` backs every text input across the app.
  - Helpers: `debug_log.rs` (`SECRETBASE_DEBUG=1` → `~/.secretbase.log`,
    mode 0600), `mouse_areas.rs` (hit-test rects).

When adding a new screen: add the `Screen` variant, an `input/<screen>.rs`
handler (wired in `input/mod.rs`), a `view/<screen>.rs` renderer (wired in
the `view/mod.rs::draw` router — popups draw their origin screen
underneath first), reuse `widgets::*` for chrome, and add its section to
`view/help.rs`. **Reuse the shared helpers above rather than
re-implementing per screen** — that's what keeps the app coherent.

## Security & memory hygiene

- **Chat payloads live in `zeroize`d buffers.** Domain types
  (`Conversation`, `Message`, …) derive `Zeroize`/`ZeroizeOnDrop`, and the
  adapter zeroizes subprocess JSON buffers before drop. This is *hygiene*
  (no stale plaintext on the heap), not a cryptographic defense.
- **Text inputs are zeroized.** `domain::LineEditor` derives `ZeroizeOnDrop`.
  Every text input can hold sensitive chat content (a message draft,
  a participant username, a global-search query), so the buffer is wiped on
  drop. This restores and extends the hygiene the compose/new-conversation
  drafts had as `Zeroizing<String>` before they moved onto the shared editor.
  Keep this derive when touching `LineEditor`.
- **Clipboard auto-clear** (`clipboard_clear_secs`, default 30 s) wipes a
  copied value — only if the clipboard still holds secretbase's write.
- **Attachment downloads are path-traversal hardened**
  (`safe_attachment_basename`); settings are written atomically
  (temp + fsync + rename) with `0o700` dirs / `0o600` files.
- Don't add a surface that writes chat content to disk (beyond the
  user-chosen attachment download path) without an explicit ask.

## Branching & PRs

- The default / integration branch is **`dev`** — never commit directly
  to it.
- **One feature = one branch = one PR.** Branch off `dev` as
  `feat/<short-slug>` (or `fix/` · `chore/` · `docs/`), implement +
  verify, push, open a PR against `dev`, then **merge it** (squash,
  `--delete-branch`) and `git checkout dev && git pull`. Keep PRs small
  and focused on a single feature.
- Remote is SSH (`git@github.com:vibecoded-software-factory/secretbase`);
  push with `gh`/`git` over SSH.

## Commits

- **Conventional Commits** prefix: `feat:` · `fix:` · `refactor:` ·
  `docs:` · `test:` · `chore:` · `ci:` · `style:` · `perf:` · `build:`.
- Subject ≤ 72 chars. Body explains the **why**, not the what. One logical
  change per commit.
- Only commit or push when the user asks.
- **Do NOT append `Co-Authored-By: Claude …` or any AI / generated-by
  trailer to commits, and no "🤖 Generated with Claude Code" footer in PR
  bodies.** The Claude Code default adds these; this rule overrides that
  default. Don't add them unless explicitly asked.

## Things to NOT touch unprompted

- The worker-thread + `mpsc` execution model — keep `keybase` calls off
  the render thread; extend the `WorkerRequest`/`WorkerResponse`/`InFlight`
  + `request_*`/`handle_*` pattern, don't reach for `tokio`/`async-std`.
- The ports/adapters boundary — I/O (subprocess, filesystem, clipboard)
  belongs only in `adapters/`; keep `domain/` pure.
- Existing keybindings and the shared chrome widgets — changing one
  screen's UX means changing all of them for coherence (see Working
  agreements), not a one-off divergence.
- The memory-hygiene discipline (zeroize, tolerant parsing, panic
  isolation, atomic settings writes, owner-only perms).

## Commands

```sh
cargo run                         # run the TUI (needs `keybase` on PATH)
cargo build                       # debug build
cargo build --release             # optimized binary at target/release/secretbase
cargo clippy --all-targets -- -D warnings   # lint (keep warning-free)
cargo test                        # unit tests
```
