# UX.md — secretbase UI/UX specification

The canonical design system for secretbase's terminal UI. **Read this
before any UI change or new feature, and keep it up to date before every
push** (see `CLAUDE.md`). The whole point is that every screen looks and
behaves the same — when in doubt, reuse an existing component; never invent
a one-off.

**Responsiveness is a hard rule.** Every element must adapt to the panel
width: show in full when it fits, otherwise **wrap** onto continuation lines
(action bars, message bodies, reaction rows) or **trim to the width** with a
trailing `…` (reply quotes, snippets — one-line previews where only the start
matters) — never a fixed character cap that the terminal then clips. Size
against the real content width, not a magic number.

The design system: the unified two-pane Home layout, the
single `widgets::list_table` list renderer, `draw_search_box`, `draw_cmd_log`,
`draw_status_strip`, the navigable `draw_confirm_popup`, and the
`LineEditor` + `editor_spans` text-input model. The shared input mechanics
live in `tui::input::common`. The decoration budget is small and lives
**only** on the splash / login screens — working screens carry no
background art.

Everything documented here is backed by real code in `src/tui/view/` and
`src/tui/input/`. When you change a component, update this file in the same
change.

## Screen layout — the unified Home

The router `view::mod::draw` picks a base screen per `Screen`, then overlays
any popup on top (popups draw the base screen underneath first). The
terminal floor is **70×18** (`view::mod::MIN_W`/`MIN_H`); below it every
screen is replaced by the centered "terminal too small" notice
(`view::mod::draw_too_small`): three vertically-centered lines — an
`error`-colored bold `Terminal too small` title, a `dim` `Resize to at least
{MIN_W}×{MIN_H} (currently {w}×{h})` line, and a `dim` `Ctrl+C to quit` hint.

The inbox is a **unified two-pane "Home"** (Discord-style): the conversation
tree on the left, the open chat on the right — no separate full-screen
conversation on normal terminals. Each signed-in screen owns its layout,
sharing the responsive command log (`widgets::cmdlog_height`) and the
status strip at the bottom.
- **two full-height columns** — the tree pane on the left (its `─[Alt+F]-Search`
  filter box, 3 rows, atop the tree) and the **chat on the right, spanning the
  full height**: no permanent header row is reserved above it. The conversation
  **name** lives on the Messages panel title (`─[Alt+M]-Messages — <name>`,
  `conversation::chat_title`). In-conversation search is a **`Ctrl+F` modal**
  (`Screen::ConvSearch`). The chat draws an **adaptive header** only when it has
  something to say (`conversation::has_adaptive_header` / `draw_adaptive_header`):
  a **pin** (`📌 sender · "content" · Alt+U unpin`) or, failing that, the channel
  **topic** (latest `headline`) — one borderless line; when there's **neither**
  it collapses to **0 rows** and the message history takes the space, so no
  chrome is reserved for nothing.
- **body** — `─[Alt+C]-Chats` tree (`Length(28)`) on the left, the chat
  (`Min(24)`) on the right:
  - `─[Alt+C]-Chats` (`Focus::Tree`) — the **conversation tree** (`App::tree_rows`
    → `TreeRow::{Group,Conv}`): a "Direct messages" group then one
    **collapsible** group per team, each (unless folded) followed by its
    conversations (channel name only, no `team#` prefix). **Unread** groups
    and conversations are **bold** (plus a `●` / unread count) so they stand
    out. The tree **starts fully collapsed** — `App::expanded` holds the
    *expanded* groups (empty = all folded, so it persists across refreshes); a
    non-empty search force-expands all. The border count is the **visible rows**
    (headers + the conversations of expanded groups) of the total. `↑/↓` move
    (`chat::tree_move`); `Enter`/`→`/`l` expand a group or **open a
    conversation in the right pane** (`chat::tree_activate` →
    `enter_conversation`, which stays on `Screen::Inbox` and moves focus to
    `Chat`), and `←`/`h` collapse a group or **close the open chat**
    (`chat::tree_back`). Opening from the quick
    switcher / global search **reveals** the conversation in the tree
    (`chat::reveal_in_tree` — expand its group + move the cursor). There is **no
    loading skeleton** — the inbox is only ever entered with its conversations
    already loaded (the splash doubles as the loading screen; see *Boot &
    loading* below), so the tree never renders empty/half-loaded. **Empty states**
    (`inbox::draw_tree_notice`) fill the panel with a friendly bold headline +
    dim hints instead of a blank list: a failed load shows `⚠ Couldn't load
    chats` + the error + `r / F5 to retry`; a genuinely empty inbox shows `No
    conversations yet` + `n to start one`; and a filter/search that matches
    nothing shows `No chats match "…"` (+ `Esc clears the filter`), `No unread
    chats`, or `No chats to show` per the active filter.
  - the right pane is `conversation::draw_chat` (messages + compose; the header
    name/search are on the shared top row) when a conversation is open, else a
    placeholder. `Focus::Chat` routes keys to `input::conversation::handle`
    (compose / select / in-conv search); `Esc` follows the vim chain — cancel
    edit → cancel reply → **Select mode** (the draft stays put; Esc never
    destroys typed text) → close the
    conversation (focus back to `Tree`). Message bodies render **inline
    markdown** (Keybase's set: `*bold*`, `_italic_`, `~strike~`, `` `code` ``;
    `domain::parse_inline` → styled runs, markers hidden, `code` verbatim) plus
    block elements (` ``` ` fenced code and `>` blockquotes with a dim `▏` bar).
    A fenced block that names a language (e.g. ` ```rust `) is **syntax-
    highlighted** via `syntect` (`tui::syntax`, dark/light theme picked from the
    active theme's brightness, foreground-only so the terminal background shows
    through, memoized per block so scrolling is cheap); an unknown / absent
    language falls back to the flat code colour. Bodies are
    **wrapped** to the panel width preserving styles (`conversation::wrap_runs`)
    instead of being cut off — `@mentions` are highlighted in the same pass.
    There is **no standalone full-screen conversation screen** — the unified
    Home is the only chat surface. **Image attachments render inline**: the
    filename + size/type sit above a thumbnail painted with the terminal's
    image protocol (kitty/sixel/iterm) or `chafa` symbols as a fallback
    (`tui::image`, `image_protocol` setting, `auto` by default; `off` disables).
    The file is downloaded once to `$XDG_CACHE_HOME/secretbase/images`
    (`{conv}-{msg}.ext`, background via the worker). While an image is
    downloading **or** (for a GIF) decoding, a **skeleton** fills the reserved
    rows (`conversation::image_skeleton_lines`, `conversation::ImgState`:
    `Loading` / `Decoding` / `Ready`) — a dim label + muted block bars, never an
    empty void; the run loop polls fast while images load so it gives way
    promptly. Then the run loop paints the cached graphic over the reserved rows
    — repainting only when the visible set/positions change, with a
    per-`(path,size)` chafa-output cache so scrolling is cheap. **`symbols`
    output is parsed into Ratatui spans and rendered *in-buffer*** (the chafa
    ANSI → `image::symbols_to_lines`, spliced over the reserved rows) so scroll,
    overlay occlusion and clearing all work via the frame diff — no
    direct-to-stdout ghosting; only the true-graphics protocols paint over the
    rows from the run loop. **Animated GIFs play inline**, decoded **off the
    render thread**: a `WorkerRequest::DecodeGif` runs ImageMagick
    (`convert -coalesce -resize`, frames capped to `GIF_FRAME_MAX_PX` so a large
    GIF doesn't burn decode/chafa time) on the background lane
    (`image::extract_gif_frames`, `App::gif_anims`/`gif_pending`/`gif_to_decode`);
    the UI shows the skeleton meanwhile and never freezes. Each frame is
    chafa-cached, and the run loop advances them by wall-clock (`anim_ms`,
    `GifFrames::frame_at`), polling at ~12 fps while a GIF is on screen
    (`gif_animating`). In select
    mode `c` **copies the image** to the clipboard (`wl-copy`/`xclip`/`pbcopy`,
    `image::copy_to_clipboard`) when an image is selected, and `s` downloads it.
- **cmdlog** — `widgets::draw_cmd_log`: the rolling `keybase …` command log
  (6 rows, `✓ cmd  →  detail  (3s)`, newest at the bottom). When focused
  (`Focus::CmdLog`) it is a **visual multi-select**: a `▶` cursor walks the
  whole history (the window follows it; `App::cmdlog_cursor`), **`v` anchors a
  visual range** that plain `j/k` extend (`App::cmdlog_anchor`; `Alt+Shift+K/J`
  / `Alt+Shift+↑/↓` remain as chord aliases), `Space` toggles a line
  (`●`, `App::cmdlog_marks`), and `y`/`Enter` copy the **full** line(s) while
  `c` copies the **detail only** (`chat::do_copy_cmd_log(full)`) — marked
  lines, or the cursor line; the selection is kept so both copies work. `Esc`
  clears the selection then leaves. The title shows `cursor/total · N sel`.
  The chat's **Select mode** (`Alt+V`, or `Esc` from the compose) uses the
  **same multi-select**: `Space` marks messages, **`v` sets a visual anchor**
  — every motion (`j/k`, paging, `g/G`, `{`/`}` speaker runs) then extends the
  shaded range, `v` again clears it (`Alt+Shift+K/J` / `Alt+Shift+↑/↓` remain
  as chord aliases) — `y` copies author + time + body
  and `c` copies bodies only (`chat::do_copy_messages(full)`), separated by a
  blank line / newline; with a selection active the action bar collapses to
  the reduced copy / react set. **`Shift+X` delete and `+` react operate on the
  whole marked set** (or the cursor if nothing's marked). Because the worker is serial,
  a multi-message action runs as a **sequential batch** (`App::pending_batch` /
  `chat::PendingBatch`) — one request at a time, advanced by each response —
  *not* N concurrent requests the busy-guard would drop; a `Deleting… 3/10`
  toast shows progress. Delete only targets your **own** messages. When the batch
  finishes it clears the shading, **stays in Select mode**, and reloads — the read
  handler re-anchors the cursor (deleted rows are gone). Single-message verbs
  (`e` edit, `p` pin, `r` reply, `s` download) act on the cursor and clear the
  marks.
  Worker ops carry their duration (request → response), formatted by
  `domain::format_duration` as a single smallest-unit value
  (`ms`/`s`/`m`/`h`/`d`/`y`).
- **status** — `widgets::draw_status_strip`: an **nvim-style `-- MODE --`
  badge** on the far left (always visible, coloured per mode — `NORMAL` accent,
  `COMPOSE` success, `SELECT` warm, `SEARCH` cyan — from `App::ui_mode()` /
  `app::UiMode`), then feedback (spinner / ✓ / ✗) when an action is in flight,
  else the per-focus footer hint, with **`F1 help` anchored right**. The badge
  tells the user what a keystroke will do (type vs act vs navigate).
- **List navigation is centralized.** Every list handler routes universal
  movement through `input::common::list_nav` (`↑↓`/`j k`, `PgUp/PgDn`, `g/G`,
  `Home/End`) — or `list_nav_arrows` (only `↑↓`/`PgUp/PgDn`) for a list behind a
  text input, where the letter aliases would be typed. This guarantees identical
  coverage so screens can't drift. The **mouse wheel** scrolls whatever list is
  active on every screen (`input::mouse` dispatches by `Screen`).

The **conversation** pane uses a custom layout (header ·
messages · compose · status) because the message stream is a *viewer*, not
a grid. The screen is mouse-interactive: click a message to select it
(enters select mode on that row) and scroll the wheel to page history
(`mouse_areas` carries the messages viewport rect + a per-message row map).
The compose box is **multi-line**: `Alt+Enter` inserts a newline (`Enter`
sends), the box grows with the line count (capped, then it scrolls to keep
the cursor visible), and `widgets::editor_lines` renders the multi-row
cursor. Single-line inputs (search, react, new-conversation) keep
`widgets::editor_spans`. The chat column is **full-height** with an **adaptive
header** (`has_adaptive_header` / `draw_adaptive_header`) — a single borderless
line for a **pin** (`📌 sender · "content" · Alt+U unpin`) or the channel
**topic**, or **nothing at all** (0 rows) when there's neither, so history isn't
squeezed by empty chrome. **In-conversation search is a modal**
(`Screen::ConvSearch`, `view::conv_search`): `Ctrl+F` opens a centered
`searchregexp` box + results (sibling of the `Ctrl+G` global-search modal,
sharing the `MODAL_*` geometry) — on-demand, not a permanent panel. Typing +
`Enter` runs `keybase chat api searchregexp` over the open
conversation (full history); each hit is **two rows** — the sender + snippet,
then a dim **day/time** line (`InboxHit::sent_at` from the hit's `ctime`,
`message_time`) for context. `↑/↓` pick, `Enter` jumps to + highlights the
message (via `pending_search_jump`, paginating older if needed), `Esc` closes.
The query + hits are **retained** until the conversation switches/closes:
reopening restores them, editing the query invalidates them, and in Select
mode `n`/`N` cycle the hits (wrapping) without reopening the modal — vim's
post-`/` motion. The **login** screen
(`view::login`) is the signed-out exception: it
shows a **login form** over the figlet/starfield backdrop — a
rounded `Login` block (cleared so the starfield doesn't bleed through) with
three fields (**Username** / **Device name** / **Paper key**) and two action
buttons (**Log in** / **Log in in terminal**). Fields render via
`editor_spans`; the paper key uses `editor_spans_masked` (`●`) unless F2
reveals it. Focus (`App::login_focus`, [`LoginField`]) is shown by an accent
label / highlighted button; `Tab`/`↑↓` cycle, `Enter` submits, `F2` reveals,
`F5` retries status, `Esc` clears the focused field, `Ctrl+C` quits (the
only quit, as everywhere). As a **text-entry** screen it owns
bare letters as typed text (the gradient rule), so its actions live on
non-text keys. Layout: a whole-screen `Layout::vertical([Fill(2), Length(20),
Fill(1), Length(1)])` puts the wordmark up top (2/3 of the stars above the
form, 1/3 below) with the form block in a fixed 20-row chunk and the hint bar
at the bottom; `fill_stars` paints every non-form chunk (incl. the gutters
either side of the centred `width−8`, clamped `[44,72]`, form) so the backdrop
is continuous. Inside, a `Layout::vertical` gives each field a label row + a
3-row bordered input, then the buttons, a short centred hint, and a
top-bordered **feedback strip** carrying the last error/result. While a login
or status check is `Running`, the screen shows the shared `splash` (spinner)
instead of the form.

Login maps to the two real `keybase login` paths (see `CLI.md`): **Log in**
runs the non-interactive **paper-key** login on the worker
(`KeybasePort::login_paperkey` → `keybase login --devicename <d> <user>` with
the paper key on stdin, held in a `Zeroizing` buffer), for a device not yet
provisioned. **Log in in terminal** handles the already-provisioned
(passphrase) case, which keybase collects via pinentry/terminal and **can't**
be scripted: the run loop **cedes the terminal** (`run_native_login` —
leave alt-screen + raw mode, run interactive `keybase login`, restore) then
re-checks status. This terminal hand-off is the one place a subprocess runs on
the render thread rather than the worker, precisely because it needs the real
TTY.

## Boot & loading

The **splash doubles as the loading screen** — the app never enters the inbox
half-loaded. The boot sequence (`flows::auth`): `request_status` shows the
splash with **"Checking session…"**; on success the splash *stays up* and the
legend flips to **"Loading chats…"** (`chat::request_boot_load_inbox`) while the
inbox `list` runs; only when it lands does `chat::handle_load_inbox_response`
transition **Splash → Inbox** with the conversations already in hand (a failed
boot load still enters the inbox so its error/retry is reachable, never stranded
on the splash). Logged out → Login. The same path serves the Login screen's `R`
retry. There is **no loading skeleton** anywhere — the legend on the splash is
the single loading affordance.

## Real-time updates

The UI is push-driven, not poll-driven. A `keybase chat api-listen` stream
feeds `flows::apply_chat_event` (drained each frame), so the screens update
on their own:

- **Inbox** — an incoming message bumps its conversation (recency + the
  unread dot) and re-sorts in place; no spinner, no full reload. The
  periodic `list` is just a safety-net resync (`inbox_refresh_secs`).
- **Open conversation** — incoming messages append live; edits / deletes /
  reactions trigger a quiet re-read so they reproject correctly. Reactions
  render **collapsed** beneath their target message (via the message's
  `reactions` field), as `{glyph} {count}` chips that **wrap Discord-style** —
  fill a row left-to-right, then continue on a new row below, never truncated
  (`conversation::reaction_lines`); the standalone reaction events are dropped from the
  stream so they don't show as stray `reacted :emoji: on msg #N` lines.
  **Message grouping** (`conversation::render_messages`): consecutive messages
  from the same sender within `GROUP_WINDOW_SECS` (5 min) collapse into one run
  — only the first carries the `→/icon sender HH:MM` header (`domain::clock_time`),
  the follow-ups render body-only (Discord/Slack-style); a **reply**, an **edit**,
  or the **pinned** message always keeps its own header. **Day dividers**
  (`── Today ── / Yesterday / Mon 12 Feb`, via `domain::day_divider_label` +
  `same_local_day`) mark each local-day change and carry the date, so the header
  only needs the clock. A **`new messages` divider** (`conv_unread`-coloured)
  sits above the first message newer than what you'd already seen — anchored by
  `App::unread_boundary`, seeded from the session-local `App::conv_last_seen`
  (recorded when you leave / switch a conversation; no baseline on a first-ever
  open, so no divider then). All three are drawn with `conversation::divider_line`
  and pushed **outside** any `spans_map` / image reservation, so they're
  non-selectable and never perturb click-to-select or the scroll math.
  **Edits fold in place** (`domain::fold_edits`): the standalone `edit`
  envelope is dropped, the target's body is replaced with the latest edit, and
  it carries a dim `(edited)` marker in its header (Discord-style) instead of a
  separate "edited" line. **`@mentions`** are highlighted in accent — but only
  the *resolved* ones (`Message::mentions`, from the read's
  `userMentions`/`teamMentions`) plus `@here`/`@channel`/`@everyone`
  (highlighted by `domain::parse_inline`), matching the GUI. **Typing `@…`** in the
  compose box opens an **autocomplete popup** (`draw_mention_popup`, floated
  above the compose) of conversation members + people who've spoken
  (`App::conv_members`); `↑/↓` pick and `Tab` inserts `@username `
  (`domain::active_mention` detects the token, `chat::accept_mention` inserts).
  **Adding** a reaction (`+` in select mode) opens a searchable
  **reaction picker** (`Screen::React`): a `/`-style search over the emoji
  catalogue — the **full standard Unicode set** (`domain::emoji::standard`,
  from the `emojis` crate; searchable by shortcode/name so "thumb" finds 👍)
  merged with the team's custom emojis from `emojilist` (fetched once,
  warmed at boot). Most-used first (per-session frecency). An arrow-navigable
  list of `glyph :alias:`, `Enter` to react, and a custom-`:shortcode:`
  fallback when nothing matches. Stock emojis send their **raw glyph** (works
  for the whole set without a shortcode lookup); custom ones send `:alias:`.
  (Keybase's `emojilist` returns only the custom emojis, hence the bundled
  standard set.) Stored reactions render
  as their **glyph** in the chat (the `:shortcode:` is resolved via the
  catalogue), collapsed under the message.
- **Select mode** (`Alt+V`) shows a contextual **action bar** under the
  highlighted message listing what you can do with it and the key for each
  (`+ react`, `r reply`, `e edit`/`Shift+X delete` on your own messages,
  `p pin`, `s download` on attachments, `o open link`/`l copy link` when the
  message contains a URL) — visual feedback that accompanies the direct keybindings, it doesn't
  replace them. `o` opens the first `http(s)` link
  (`domain::extract_urls` → `ports::OpenerPort`, the `xdg-open`/`open` adapter).
  The bar **packs onto one line when it fits and wraps onto continuation lines
  otherwise** (`select_actions_lines`) — never truncated. The same
  fit-or-wrap rule governs **attachments**: the file type sits next to the size
  (`size · type`) when there's room, else it drops to a dim line below
  (`render_attachment`).
- **Sending** — the message echoes instantly the moment `Enter` is pressed,
  as an optimistic **outbox** bubble below the history (`App::outbox`, kept
  separate from `messages` so a re-read can't drop it). It carries a state:
  `○ sending…` while in flight; on success it flips to a delivered `→` and
  the reconciling re-read prunes it; on failure it stays as a red
  `✗ failed · Alt+R to resend`, preserving the body so **`Alt+R`** retries
  it. The compose is cleared at send time (the draft is safe in the outbox).
- **Jump-to-latest cue** — when the reader is **scrolled up** in history and
  messages arrive below the fold, a floating accent pill at the foot of the
  viewport (`conversation::render_messages`) reads **`▼ N new · End`** (the count
  of arrivals since you scrolled up, from `App::new_since_scroll`, bumped in
  `chat::handle_incoming_message`), or `▼ latest · End` when merely scrolled up
  with nothing new. It clears the instant the reader is back at the bottom
  (`effective_back == 0`). Purely a paint over the last viewport row — it doesn't
  touch the scroll / `spans_map` math. With an **empty compose**, `End` jumps
  to the latest message and `Home` to the oldest loaded (with draft text they
  stay compose-cursor keys), so the cue's `End` is honest in the default mode.

Loading states stay honest: the message viewer shows "Loading messages…"
during the first fetch (not the empty-conversation prompt), and the unread
badge clears as soon as a conversation is read (not on the next resync).

## Quick switcher (`Ctrl+K`)

Discord-style jump-to-conversation modal (`Screen::QuickSwitcher`), opened
with `Ctrl+K` from the inbox or an open conversation (returns to wherever it
was opened — `App::switcher_from`). With an **empty** query it shows
Discord-style sections — **Drafts**, **Unread**, **Recent** — each by
recency, no conversation repeated (`App::switcher_rows` →
`SwitcherRow::{Header,Conv}`); **typing** collapses to a flat fuzzy list
(`fuzzy_score_lowered`). Unread rows carry a `●`. `↑/↓` select (over the
selectable convs, `switcher_selectable`), `Enter` jumps (opens by conv id,
bypassing the inbox filter), `Esc` cancels. The footer degrades to
`↑↓ select · Enter` when the modal is too narrow for the full hint.

**Drafts** (`App::drafts`, in memory only — not persisted): the compose text
you leave unsent is stashed per conversation when you switch/close, restored
on reopen, and dropped on send. Editing an existing message isn't stashed.
It's navigation, distinct from `/` (inbox filter) and `Ctrl+G` (message
search).

## Command palette (`Ctrl+P`)

Sibling of the quick switcher, for **actions** instead of conversations
(`Screen::CommandPalette`, opened with `Ctrl+P` from the inbox or Teams,
returns to `App::palette_from`). A fuzzy query box over a **context-aware**
list of commands (`flows::palette::palette_commands` — only the actions valid
where you are: the per-conversation status verbs need a selected/open chat, the
`Conversation` group only shows with a chat open, the channel browser only on a
team, **Members** only on a team channel, and the **Message** group (edit /
reply / react / pin / download / copy / delete the cursor message) only in
Select mode — with "Select messages" itself hidden once you're already
selecting). With an **empty** query the rows are grouped under category headers
(`Chat` · `Conversation` · `Message` · `Navigate` · `App`, via `palette_rows` →
`PaletteRow::{Header,Cmd}`); **typing** collapses to a flat filtered list
(substring over label + keywords). Each row shows its **keybinding**
right-aligned, so the palette doubles as an executable cheat-sheet — the `F1`
help is read-only, this *acts*. `↑/↓` select (over `filtered_commands`), `Enter`
runs (`run_palette_action` restores the base screen then calls the very same
`flows::*` the keybinding would — it never diverges), `Esc`/`Ctrl+P` cancel.
Shares the standard modal geometry (`center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT)`,
`view::palette`). `Ctrl+K` (chats) and `Ctrl+P` (actions) stay deliberately
separate — two clear, single-purpose palettes.

**Keep the command set in sync (hard rule).** The palette is now a **fourth**
place that enumerates actions, alongside the footer hints, the `F1` help popup
and the `README.md` keybinding tables. **Any new action or changed keybinding
must be added/updated in `flows::palette::palette_commands` in the same change**
— with the right category and its keybinding in the `keys` field — so the
palette never lists a stale or missing command. Update all four together
(footer · help popup · README tables · command palette).

## File picker (`tui::file_picker`)

A self-contained, headless-safe file chooser (no GUI/portal dependency),
rendered as a modal overlay (`App::file_picker`, drained before screen
routing in `input::handle_events`, drawn last in `view::draw`). Single-pane,
desktop-chooser style: a path bar, the current directory (dirs first;
dotfiles and any extension shown), and a hint bar. `↑↓`/`kj` move,
`Enter`/`→` open-or-pick, `⌫`/`←` parent, `~` home, `g`/`G` ends, `/` fuzzy
filter, `.` toggles hidden, `Esc` cancels. Two modes:

- **File** (`PickerMode::OpenFile`) — pick an existing file. `Alt+A` in a
  conversation opens it; the pick feeds `keybase chat api attach` (upload).
- **Directory** (`PickerMode::Dir`) — only directories are listed, plus a
  `📂 [ choose this folder ]` row that picks the current dir. Opened from
  select-mode `s` on an attachment to choose a **download** destination; it
  starts at the OS Downloads folder (`$XDG_DOWNLOAD_DIR` → `~/Downloads` →
  `/`) and the file is saved as `<dir>/<sanitised-basename>`.

`App::picker_action` (`Upload` / `Download{message_id, filename}`) records
why the picker was opened, so the `Outcome::Selected(path)` handler knows
which action to fire. The module depends only on `ratatui` + the shared
`Theme` + `LineEditor`, so it is reused verbatim across the TUIs.

## Panels & focus

The inbox's focusable panels are the `screens::Focus` variants: `Search`
(chat filter), `Tree` (conversation tree), `Chat` (the open chat — skipped in
the Tab cycle when no conversation is open), `CmdLog` (the status strip is
chrome, not a focus target).
Conventions:

- **Every focusable panel's border tag is its literal go-to combo** (no bare
  `/`): `[Alt+F]` chat Filter, `[Alt+C]` Chats, `[Alt+M]` Messages, `[Alt+L]`
  command Log. (`Ctrl+F` in-chat search is a modal now, not a focusable panel,
  so it carries no border tag.) The jumps are
  **global** — they fire from any focus, even mid-compose (weechat-style
  modifier chords), so the tag always tells the truth. The `Alt` prefix is
  reserved for these panel jumps; list *actions* are bare letters (the gradient
  convention above) routed by the focused panel's handler.
  `Tab`/`Shift+Tab` also cycle focus via `input::common::cycle_focus`.
- **Positional pane navigation** — a `Ctrl+W` leader (vim window-nav) arms
  `App::pending_pane_nav`; each following `h/j/k/l` or arrow moves to the
  spatial neighbour (`input::inbox::pane_target`: the filter on the top-left,
  Chats/Chat in the body, command log across the bottom). The leader
  **stays armed across consecutive directions**, so two keys make a diagonal
  (e.g. `k` then `h` = up-left). `Esc`/`Enter` exit; any other key exits and is
  re-processed. The status strip shows the armed hint.
- **Border tiers signal reachability** (three states, not two):
  - **focused** → accent + bold (`view::mod::titled_block(title, true, app)`).
  - **available, unfocused** → the `inactive` tint (bright gray) — you can Tab /
    go-to it.
  - **unavailable** → `view::mod::disabled_block` (the darker `muted` tint) for a
    panel you **can't** focus right now, so it doesn't look like a Tab target
    that's just ignoring you. Currently the **Chat** pane and its **in-chat
    search** when no conversation is open (both skipped by Tab, their go-to keys
    gated); `draw_search_box` takes a `disabled` flag for the search box.

## Lists & tables — the single pattern

**Numbered section borders.** Each list section carries a `─[N]-` tag woven into
its top border. The inbox numbers
its panels `─[Alt+F]-Search`, `─[Alt+C]-Chats`, `─[Alt+M]-Messages`, `─[Alt+L]-Command log`; Teams
shows plain `Teams` / `Command log` titles (nothing there is Tab-focusable,
and a tag must never advertise a key that doesn't work). `draw_search_box`
adds the `─[/]-` tag
itself; `draw_cmd_log` takes the panel number; list titles are prefixed at the
call site.

**All multi-column lists render through `widgets::list_table(...)`.** It owns:
header row (dim + bold), bordered titled block, `▶ ` selection symbol,
`selected_bg` + bold row highlight, `column_spacing(2)`, and **persisted
scroll** (writes back the offset). Build `headers: &[&str]`,
`widths: &[Constraint]` and `rows: Vec<Row>`, then call it. Title is built
with `widgets::list_title` → **`"<Thing> · {filtered} of {total}"`**.

Rules:
- **Size content columns to the *visible* (filtered) rows.** **Never** put a
  stretching `Constraint::Min(..)` on a non-final content column — it shoves trailing
  columns to the far right (the recurring "gap" bug). Only the last column
  may be `Min`.
- Row-level color overrides are per `Cell` (unread `conv_unread`, members-type
  tag `conv_dm` / `conv_team`, team role color).
- Map mouse clicks with `widgets::table_row_at(rect, y, scroll, len)` (called
  from `input::mouse`).
- Truncate an overflowing primary column with `widgets::middle_ellipsis`
  (head-biased, UTF-8 safe). For a one-line preview where only the start
  matters (reply quotes, search-result snippets) use
  `widgets::trim_end_ellipsis` (trailing `…`) instead.

Screens using `list_table`: inbox (filters + conversation list), teams. The
**conversation** message viewer and the **global-search** results are the
exceptions (custom `Paragraph` / `List`).

## List state convention (`App`)

The inbox keeps: `filtered_cache: Vec<usize>` (indices into `conversations`
surviving the active filter + search), `search: LineEditor`, `tree_selected`
(the cursor into the visible `tree_rows()`), `list_scroll` (the tree's scroll
offset). Rebuild via `rebuild_filter()`; ranking is
`domain::search::fuzzy_score_lowered` over the pre-lowercased
`LoweredConversation` projection (channel 100, topic 60, creator 20), then
sorted most-recent-first by `active_at_ms`. `rebuild_lowered` refreshes the
projection + per-filter counts once per load. `rebuild_filter` re-seats
`tree_selected` on the first conversation row; `handle_load_inbox_response`
then restores it onto the same conversation **by id** so a background resync
doesn't yank the cursor. `tree_selected` only ever indexes `tree_rows()` via
`.get()`, so it can never point out of bounds.

## Chrome & widgets (`view::widgets` + `view::mod`)

**Rounded borders everywhere (hard rule).** Every section panel uses
`BorderType::Rounded` — set once in `titled_block`, `list_table`, the Settings
`focus_block`, and the file picker, plus the existing `rounded_block` pickers
(global search, react, quick switcher). A new bordered panel **must** be
rounded too (reuse `titled_block` / `list_table` and it is, for free). The only
non-rounded borders are the deliberate `BorderType::Double` modal frames — the
destructive `draw_confirm_popup`, the Settings outer frame, and the help popup —
where Double signals "modal overlay"; don't round those without a deliberate
decision.

- `view::mod::titled_block(title, focused, app)` — the bordered block (rounded):
  focused = accent + bold, else `inactive`. Used for every panel.
- `widgets::list_table` / `list_title` / `middle_ellipsis` /
  `trim_end_ellipsis` / `table_row_at` — the list renderer + sizing helpers.
- `widgets::draw_search_box(frame, app, area, title, placeholder, editor,
  focused)` — the `[/] Search` box; placeholder when empty/unfocused, block
  cursor when focused (via `editor_spans`).
- `widgets::editor_spans(editor, focused, theme)` — renders a `LineEditor`
  with a reverse-video block cursor; the one text-input renderer (search,
  compose, popup inputs).
- `widgets::draw_cmd_log` / `widgets::draw_status_strip` — the command-log
  panel and the bottom feedback/hint strip.
- `widgets::draw_confirm_popup(frame, area, theme, title, body, confirmed)` —
  the shared navigable y/n overlay.
- `widgets::center_rect` / `rounded_block` / `help_line` — popup chrome.
  `widgets::editor_spans` / `editor_spans_masked` render a `LineEditor` (the
  latter as `●` for secret fields like the Login paper key).
- `widgets::MODAL_WIDTH_PCT` / `widgets::MODAL_HEIGHT` — **the standard
  centered-modal geometry** (currently 80% wide × 22 rows). Every list / picker
  overlay imitates it via `center_rect(MODAL_WIDTH_PCT, MODAL_HEIGHT, …)` so
  they all line up: **global search** (Ctrl+G), the **in-conversation search**
  (Ctrl+F), the **quick switcher** (Ctrl+K), the **command palette** (Ctrl+P),
  the **reaction picker**, the **file picker**, and the **Settings** overlay
  (which keeps its own content-sized *width* but adopts this *height* +
  centered position). A new full-screen-ish modal should use these constants,
  not a one-off `center_rect(w, h, …)`.
- `widgets::button(label, active, theme)` — **the** action-button span, rendered
  `[ label ]` (accent on `selected_bg` + bold when focused/highlighted, else
  `dim`). The Login form's two buttons and `draw_confirm_popup`'s confirm/cancel
  both use it, so every button reads the same — a new button reuses this, never a
  one-off styled span.
- `widgets::unread_style(theme)` + `widgets::unread_dot` / `favorite_star` — the
  single "unread / attention" emphasis (`conv_unread` + bold) and its `●` / `★`
  marker spans. Every unread affordance (the tree's dot + count, the ★
  favourite) draws through these so the emphasis is identical everywhere.
- `widgets::key_style(theme)` — the **keybind-letter** style (`accent` + bold).
  Every place that shows a shortcut glyph — the `F1` help popup (`help_line`),
  the select-mode action bar (`select_actions_lines`), the `Ctrl+P` command
  palette — styles the key through this, so keys read the same everywhere (the
  "keybind letters = accent" gradient rule). Labels keep their per-context style.

## Overlays & confirmations

Popups are centered and **always drawn over their base screen** by
`view::mod::draw` (inbox under logout/new-conv/global-search; conversation
under react/delete/download).

- **Confirmations** (`ConfirmLogout`, `ConfirmDeleteMessage`,
  `ConfirmConvAction`) render through `widgets::draw_confirm_popup`
  (centered, double border) — its `[ confirm ]` / `[ cancel ]` buttons are
  `widgets::button` spans, matching the Login form's buttons: `←/→` (or
  `Tab`/`h`/`l`) move between **confirm/cancel**, `Enter` activates the
  highlighted one, `y`/`n`/`Esc` are shortcuts. **Default highlight = cancel** for the destructive action
  (`logout_yes` / `delete_msg_yes` / `conv_action_yes` default `false`).
  Classified by `input::common::confirm_key`/`ConfirmInput`.
  `ConfirmConvAction` is the generic home for per-conversation status
  actions (`App::ConvAction`: `Ignore` / `Block` / `Report`) — each variant
  supplies its own title/note and maps to a `setstatus` value
  (`ignored`/`blocked`/`reported`), so adding one is a single enum arm.
  **Favourite (`s`) and mute (`u`) are local-only toggles** — neither
  uses Keybase's conversation `status` (the chat `list` JSON has no `status`
  field, verified on `ConvSummary`, so it can't be read back and a synced state
  would drift). Both are synchronous, no worker call, fully owned by us
  (`chat::toggle_favorite_conversation`/`toggle_muted_conversation` →
  `App::toggle_favorite`/`toggle_muted`, persisted to the `favorites`/`muted`
  config keys). Favourite renders a golden **★**; **mute suppresses the unread
  indicators** secretbase controls — `App::conv_is_unread` (`unread && !muted`)
  gates the `●` dot, the bold, the unread count and the switcher's Unread
  section, and a muted conv renders **dim**. (The TUI has no
  notifications, so a local mute can't silence your phone — see README →
  *Not supported*.) **Ignore / block / report** stay server-side (`setstatus`)
  because their effect *is* observable (the conv leaves the inbox).
- **Channel browser** (`ChannelBrowser`, `c` on a team row) — a standard
  centered modal (`view::channels`, `MODAL_*` geometry) listing **every**
  channel of the team via `keybase chat api listconvsonname` (same `ConvSummary`
  shape as `list`, so the tolerant parser is reused; `member_status == Active`
  marks the ones you're in, sorted joined-first). `↑/↓` pick, `Enter` opens a
  joined channel or **joins** one you aren't in (`join`), `Shift+L` **leaves** a
  joined one (`leave`), `n` enters an inline **create** mode (a channel-name
  input → `newconv` on a team channel, reusing the `NewConversation` request
  routed by an `InFlight::CreateChannel` slot), `r` an inline **rename** mode
  (pre-filled → `rename-channel`), `Shift+X` an inline **delete** confirm
  (destructive + irreversible — the same navigable confirm mechanics as the
  overlays: `widgets::button` pair, default = cancel, `←/→`/Tab/Enter/`y`/`n`,
  error-red → `delete-channel`), `t` toggles the
  channel as a team **default** (new members auto-join; a `★ default` badge,
  `#general` always), `F5` refreshes, `Esc` closes. The inline modes share one
  bottom row + the `channel_new_name` editor; `rename-channel`/`delete-channel`/
  `default-channels` are **CLI subcommands** (one-shot spawn, no API method), the
  rest are chat-api methods. `default-channels` is fetched (get) chained after
  the list load to badge; `t` recomputes and **replaces** the whole set (the CLI
  can't clear it to empty, so removing the last one is refused). Every mutation
  resyncs the inbox (silent) so the tree tracks it and reloads the browser. Flows
  in `chat::*channel*`; input in `input::popups::channel_browser`.
- **Members view** (`Members`, `m` in the channel browser or `Alt+P` on an open
  team channel) — a standard centered modal (`view::members`) listing a
  conversation's members via `keybase chat api listmembers` (the six
  `ChatMembersDetails` role buckets flattened to `domain::ChatMember`, sorted
  higher-privilege-role first then by name, with the role label dimmed). `a`
  enters an inline **add** mode (comma/space-separated usernames, validated →
  `addtochannel`), `Shift+X` an inline **remove** confirm (navigable, default =
  cancel → `removefromchannel`),
  `F5` reloads, `Esc` returns to wherever it was opened from (`members_return`).
  Add/remove reload the list on success. DMs/non-team convs are refused (fixed
  membership). Flows in `chat::*member*`; input in `input::popups::members`.
- **Input popups** (`NewConversation`, `UnhideConversation`, `React`,
  `DownloadAttachment`, `SearchGlobal`): a centered box with an `editor_spans`
  field and
  self-contained `Enter: … | Esc: cancel` instructions. Keys route through
  `input::common::route_line_editor`. `SearchGlobal`'s `Enter` opens the
  hit's conversation **and jumps to the matched message**: it stamps
  `App::pending_search_jump`, then the read handlers select+highlight that
  message once loaded, paginating older pages until it's found (or history
  is exhausted — then a "not found" toast).

## Help popup

`view::help::draw` is **context-aware and scrollable**: it shows the
shortcuts for the screen it was opened from (`App::help_from`, stamped on
F1) plus a shared Global section. On the unified Home it follows the focused
pane — `Focus::Chat` shows the conversation shortcuts, otherwise the inbox
ones. The renderer owns the viewport — it clamps
`App::help_scroll` against the real overflow, so the input handler bumps the
offset freely (`j/k`/arrows scroll, `PgUp/PgDn` page, `g/G` top/bottom,
`q`/`Esc`/`F1` close); `▲`/`▼` border marks flag hidden content. Add a
section for every new screen and keep it in sync with `README.md`.

## Inline editing & the text-input model

Every text input is a `domain::LineEditor` (UTF-8-safe byte cursor on a char
boundary; `insert`/`backspace`/`delete`/`left`/`right`/`home`/`end`/`set`/
`clear`, plus readline/vim-insert **word ops** — `Ctrl+W` delete word,
`Ctrl+U` kill to line start (line-aware in the multi-line compose),
`Ctrl+←/→` word jumps, `Ctrl+A`/`Ctrl+E` start/end — wired once in
`route_line_editor` so every input gets them). `Ctrl+W` therefore only arms
the pane-nav leader in non-typing surfaces (tree, cmdlog, Select mode). Here it derives `ZeroizeOnDrop` (a secretbase-specific divergence)
because any input can hold sensitive chat content — see `CLAUDE.md`. Handlers feed keys through `input::common::route_line_editor`
(returns `true` when the text changed → rebuild a filter) or, for the inbox
filter box, `input::common::search_key`/`SearchAction`. Rendering is always
`widgets::editor_spans`. Empty boxes show a dim placeholder
(`theme.placeholder`).

## Keybindings — the gradient convention (hard rule)

Keys are assigned by a **gradient of tiers**, so the modifier tells you the
weight of the action before you press it. Every screen follows the same tiers:

- **bare lowercase letter = the frequent, safe action on the focused list**
  (`n` new, `r` refresh, `y` yank label, `e` mark read, `u` mute, `s` ★
  favorite, `t` teams, `c` channels). In a popup list the same tier holds its
  own vocabulary (`a` add, `r` rename, `m` members, …). This is the lazygit /
  aerc / mutt model: the list is "command mode", letters act on the cursor row.
- **`Shift+letter` = the loud / destructive / irreversible tier**: `Shift+I`
  ignore, `Shift+B` block, `Shift+R` report, `Shift+H` unhide, `Shift+L`
  logout (inbox); `Shift+X` delete/remove and `Shift+L` leave (select mode,
  channel browser, members). One rule everywhere: **if it removes, hides, or
  can't be undone in-app, it's `Shift`** (and still guarded by a confirm).
- **`Ctrl` = global** — works from any focus, never confused with typed text:
  `Ctrl+C` quit (the **only** quit), `Ctrl+F` in-chat find, `Ctrl+G` global
  search, `Ctrl+K` quick switcher, `Ctrl+P` command palette (both from the
  inbox *and* Teams), `Ctrl+W` positional pane nav (non-typing surfaces only —
  in an input it deletes a word), `Ctrl+D`/`Ctrl+U` half-page in every list
  and viewer, `Ctrl+J`/`Ctrl+K` selection moves inside every picker popup.
  **`:`** opens the command palette from any non-typing surface (the vim
  command line; `Ctrl+P` remains the works-everywhere form). `Ctrl+N` opens
  the **next unread** conversation (recency order, wrapping — the triage
  key); `Ctrl+O` toggles back to the **previous** conversation
  (`App::prev_conv_id`, vim's alternate buffer). The run loop
  opts into the **kitty keyboard protocol** where supported
  (`DISAMBIGUATE_ESCAPE_CODES` only), so Alt/Shift chords survive SSH;
  unsupported terminals keep classic delivery.
- **`Alt+letter` = jump to a panel** — each combo matches that panel's border
  tag: `Alt+F` Filter, `Alt+C` Chats, `Alt+M` Messages, `Alt+L` Log. These fire
  from any focus (even mid-compose), so a text field can't trap you. Compose
  also parks its *non-text* actions on `Alt` (`Alt+V` select, `Alt+A` attach,
  `Alt+E` edit-your-last-message, `Alt+U` unpin, `Alt+R` resend, `Alt+P`
  members) because bare letters there are typed text.
- **`/` = focus search** · `Esc`/`h` back · `F1` help · `F10` Settings ·
  `Tab`/`Shift+Tab` cycle focus · `j/k`+`↑/↓` navigate · `PgUp/PgDn` page ·
  `g/G` top/bottom · `Enter`/`l` open.

**Why the tiers, not `Alt` for everything.** A text field (compose, the filter
box) owns bare letters as typed text; a list doesn't type, so its letters are
free to act. Putting actions on bare letters in lists (and only shifting to
`Alt`/`Ctrl` where text input would collide) is what makes the app feel like a
pro TUI instead of a chord soup. The footer shows only a few keys; the full
per-screen list lives in the help popup, the `README.md` tables and the
**command palette** (`flows::palette::palette_commands`) — **keep all four in
sync** whenever an action or keybinding changes (footer · `F1` help · README
tables · `Ctrl+P` command palette).

## Theme (`tui::theme`)

Resolved from the optional `[theme]` section of `config.toml`; every key
optional, partial configs valid. Fields: `accent`, `inactive`, `selected_bg`,
`success`, `error`, `dim`, `foreground` (defaults to `Color::Reset` to
inherit the terminal), `placeholder`, `muted`, the restrained splash
starfield colors `star_dim`/`star_mid`/`star_bright`, and the conversation
marker colors `conv_dm` / `conv_team` / `conv_unread`. **Don't hardcode
colors — use these.**

**Per-user sender colors.** In the message stream each **peer's** name renders
in a stable, per-user hue so authors are easy to tell apart (Discord/IRC-style):
`Theme::user_color(name)` hashes the username (FNV-1a, pure/stable) into
`Theme::user_colors` — a spread of the preset's raw hues derived in
`from_palette`, **excluding `accent`** so a peer never looks like you. Your own
messages stay `accent`+bold and system lines stay `dim`+italic. `user_colors` is
always derived (not a `[theme]` override key).

**Legibility hierarchy (hard rule).** Text de-emphasis comes from *hierarchy*,
never from painting content almost the colour of the border. There are three
legibility tiers and a fourth recessive band — pick by what the text **is**,
not by reflex:

1. **`foreground`** — primary content (message bodies, conversation names, the
   command being run, input text). The thing the user is reading.
2. **`accent`** (often `+ BOLD`) — emphasis / interaction: the focused-panel
   border + title, the `▶` cursor, keybind letters, the active tab, the
   identity username.
3. **`dim`** — *readable* secondary text: counters (`· X of Y`), timestamps,
   command-log detail, footer hints, system-message bodies, the column-header
   row. `dim` is a **subtext that stays legible**, derived per-preset as a blend
   `overlay→text` (0.5) in `Theme::from_palette` — **not** the border tint. Do
   not map it back to `overlay`.
4. **`inactive`** — unfocused panel **borders**. A *bright*, near-text gray
   (`overlay→text` 0.6) — lazygit renders inactive borders in the terminal's
   default foreground, so they read clearly; what marks focus is the *active*
   border going `accent` + bold, never the inactive one fading out.
5. **Recessive band** — genuinely faint, chrome only: `placeholder`
   (empty-input "type here…", `overlay→text` 0.25), `muted` (` · ` separators,
   the popup `(←/→ · Enter · Esc)` legend, disabled chips). Never put content a
   user must read here.

**Navigable list items are content, not chrome.** Sidebar sections, setting
rows, switcher/picker rows render at **`foreground`** (the selected one `+ bold`,
and `accent + bold` when its pane is focused) — the `▶` marker + accent carry the
selection, so unselected rows must **not** be `dim`. Read-only values (the
Identity panel's username/device) are data you read → `foreground` too; the
"read-only" footer hint is what signals they can't be edited.

The failure mode to avoid: reaching for `dim` (or worse, `muted`) on text that
is actually *primary* or *secondary-but-needed*, so the screen reads as a wash
of low-contrast grey. When in doubt, one tier brighter. Borders + the selection
background already carry the focus signal — the text doesn't have to dim itself
to prove it's secondary.

**Presets.** Themes are built from a `Palette` (13 named roles) via
`Theme::from_palette`, which maps the core roles and derives the starfield +
conversation-marker colors. Four presets ship (`Preset::ALL`:
`catppuccin-mocha`, `dracula`, `nord` (default — `Preset::DEFAULT`),
`catppuccin-latte`); `name = "<preset>"` in `[theme]` picks the base and per-key
hex entries override it. The Settings picker applies live. Adding a preset = one
`Palette` arm in `Preset::palette`.

## Settings overlay (`F10`)

`F10` opens a centered **Settings** overlay (`Screen::Settings`, drawn over
`settings_from` like Help) — `view::settings::draw_popup`, input in
`input::settings`. Layout: a left **section sidebar** + the active section's
**panel**; `Tab` switches sidebar ↔ panel, `↑/↓` move within (section or row),
`←/→` change the focused setting. **Height + position follow the standard
modal geometry** (`MODAL_HEIGHT`, vertically centered — the same as global
search / the quick switcher), so overlays line up. The **width is
content-driven** (`settings::popup_dims` + `section_width`): sized once to the
widest section so it is **compact** and **never resizes as you navigate**
between sections, clamped to the terminal so it stays **responsive**. Inside a rows panel the
**label column is per-section** (sized to that section's longest label, not a
global fixed column — a tight label→value gap), the **value wraps** onto
continuation lines when it doesn't fit (`settings::wrap_chars`) **never a `…`
truncation** (matching the app's read-everything rule), and the focused row's
**hint is pinned to the bottom row** (`settings::panel_split`) — as is the Theme
panel's "Applies live…" note — so descriptions never float in the middle. The
sections (`SettingsSection`): **Identity**
(read-only — your username + device + device type, from `keybase status`),
**Theme** (the live preset picker), **Chat** (`auto_mark_read`,
`inbox_refresh_secs`), **Emoji** — two glyph-appearance levers (a TUI can't set
the terminal's font, so the only knob is *which glyph* to emit): `emoji_style`
(`glyph` vs `:shortcode:`, reaction display) and `icon_style` (`unicode` — the
headless-safe default that renders on any font — vs `nerd` for nerd-font glyphs,
via `tui::icons`), **Clipboard** (`clipboard_clear_secs`), **Network**
(`list_inbox_timeout_secs`, `download_timeout_secs`) and **Images**
(`image_protocol`, `image_symbols`).

Each non-Identity row is one of three controls keyed off `SettingId::kind`: a
**toggle**, a **number stepper** (clamped, `0` shown as `off` where it disables),
or a **choice** (cycles a fixed list). **Apply-immediately**: every change is
written to `settings_cache` *and* persisted to `config.toml` the instant you
adjust it (`App::settings_adjust` → `SettingsPort::write_setting`; the theme
picker uses `App::apply_theme_idx` → `write_theme_name`), so closing just
leaves — there is no separate confirm/cancel. **`Esc` steps back** (Panel →
Sidebar → close) so it doesn't dump you out of the overlay from inside a
section; **`F10` closes from anywhere**. A changed image protocol /
symbol set re-resolves `image_proto` / rebuilds the render cache live. Add a
new section by extending `SettingsSection::ALL` + `rows`; a new setting by
adding a `SettingId` arm — the box re-sizes itself to fit (and long values
wrap), so nothing overflows.

## Responsiveness — what adapts today (and how)

Responsiveness is a hard rule (see the top of this file). This is the **inventory
of mechanisms that already exist** — reuse them, and don't regress them. Every
one is backed by code; the file/function is named so you can find it.

**Vertical stack:** `header (3) · body (Min 5) · cmdlog · status (1)`. The
**body flexes**; the **command log is height-responsive** —
`widgets::cmdlog_height(area.height)` yields rows to the body as the terminal
gets short (6 when roomy → 3 at the floor) **monotonically** (a taller terminal
never shrinks the body). Neither the Home (`view::inbox`) nor **Teams**
(`view::teams`) reserves an identity row — the username/device live in
Settings → Identity, giving the lists the full height. `Enter`/`→`/`l` on a team opens the **channel browser** for it
(`chat::open_channel_browser_for_team`). Below **70×18** every screen is replaced by the centered
"terminal too small" notice (`view::mod::draw_too_small`, states required +
current size).

**Horizontal panes:** the tree column is **width-responsive** —
`widgets::tree_pane_width(area.width)` (~28% clamped to `[22, 40]`): it shrinks
toward the floor on a narrow terminal so the chat keeps room, and grows on a wide
one so long DM/team names aren't always truncated (no magic `28`). The Home's
header filter and body tree pass the same width so their columns line up; the
chat side is `Min(20/24)` and flexes, spanning the **full height** (no reserved
header row) — the chat's adaptive header line appears only when there's a pin or
topic, otherwise the message history takes the row.

**Text that fits-or-degrades (never a fixed char cap that the terminal clips):**
- **Footer hint** — `widgets::fit_segments` keeps only whole ` · ` segments that
  fit, appends ` …`, never cuts a keybinding in half; `F1 help · F10 settings`
  is anchored right and reserved first.
- **List columns** — size a content column to the **visible** rows, clamped;
  **only the final column may be `Min`** (a stretching `Min` on a middle
  column is the recurring "gap" bug). Overflow:
  `middle_ellipsis` (head-biased, keeps a `#id`/extension tail) for identifiers,
  `trim_end_ellipsis` (trailing `…`) for start-anchored previews (snippets,
  reply quotes).
- **Message bodies** — `conversation::wrap_runs` wraps to the panel width
  **preserving styles**, hard-splitting an over-long unbroken token; never
  clipped.
- **Reaction chips** — `conversation::reaction_lines` fill a row left-to-right
  then continue on a new row (Discord-style), never truncated.
- **Select-mode action bar** — `conversation::select_actions_lines` packs onto
  one line when it fits, else wraps onto continuation lines.
- **Attachment meta** — `size · type` inline when there's room, else `type`
  drops to a dim line below (`render_attachment`).

**Growing / scrolling regions:** the **compose** box grows with its line count
`(compose_lines + 2).clamp(3, 8)` then scrolls to keep the cursor visible
(`editor_lines`). The **message viewer**, **command log** and **help** each own
their viewport and clamp the scroll offset against the *real* overflow (so a
`usize::MAX`/`u16::MAX` "jump to end" sentinel is safe); help shows `▲`/`▼`
border marks when content is hidden. **Image** thumbnails reserve rows sized to
the available cols/rows, fill them with a skeleton while loading
(`image_skeleton_lines`), and cap GIF frames to `GIF_FRAME_MAX_PX`.

**Modals** share one geometry so they line up and stay on-screen:
`center_rect(MODAL_WIDTH_PCT = 80%, MODAL_HEIGHT = 22)`. The confirm popup sizes
`w = 66.min(area.width)`, `h = (body + 4).min(area.height)`; **Settings** keeps a
content-driven width sized **once** to the widest section (so it doesn't resize
as you navigate), clamps height to `MODAL_HEIGHT.min(area.height - 2)`, **wraps**
long values onto continuation lines (`settings::wrap_chars`, never a `…`), and
pins the focused row's hint to the bottom.

**Modals stay on-screen:** every list overlay windows its content by the *real*
inner height, not the nominal `MODAL_HEIGHT` — the **react picker** /
**quick switcher** scroll a viewport around the selection (`vh = rows[1].height`),
**global search** uses a `ListState` that scrolls the selection into view, and the
**confirm popup** clamps `h = (body + 4).min(area.height)`. So a short-but-valid
terminal shrinks the modal and the content follows; nothing clips off the bottom.

**Resize hygiene:** `view::draw` stamps `mouse_areas` with the frame size each
frame and the run loop `terminal.clear()`s on a size change; clicks whose
coordinates predate the latest resize are dropped.

**Remaining judgment calls (not bugs, but trade-offs to weigh):**

- The body keeps a hard `Min(5)`; on the rare 18-row terminal that's still tight,
  but the responsive command log already reclaims rows for it before that bites.
- `tree_pane_width` clamps at 40 cols — a *very* wide terminal gives the chat the
  rest, which is the right call, but extremely long team names can still need
  `middle_ellipsis` inside 40. That's intended (the chat is the priority), not a
  gap. Revisit only if users ask for a draggable/configurable split.

## Golden rules

1. **Reuse, don't reinvent** — a new list = `list_table`; a new input =
   `LineEditor` + `editor_spans` (routed via `input::common`); a new panel =
   `titled_block`; a new confirm = `draw_confirm_popup`; a new button =
   `widgets::button`; an unread/attention marker = `widgets::unread_style` /
   `unread_dot` / `favorite_star`; a new signed-in screen follows the Home
   layout conventions (shared cmdlog + status strip).
2. **Fix the class, not the instance** — when you change one screen, change
   every screen with the same pattern (and update this file).
3. **Every change stays coherent** with the rest of the UI. If you diverge,
   update the spec here first and apply it everywhere.
4. **No decorative noise on working screens** — the figlet + starfield belong
   to splash/login only. Screen identity comes from the bordered block titles.
