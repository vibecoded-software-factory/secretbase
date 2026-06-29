# UX.md — secretbase UI/UX specification

The canonical design system for secretbase's terminal UI. **Read this
before any UI change or new feature, and keep it up to date before every
push** (see `CLAUDE.md`). The whole point is that every screen looks and
behaves the same — when in doubt, reuse an existing component; never invent
a one-off.

**Responsiveness is a hard rule.** Every element must adapt to the panel
width: show in full when it fits, otherwise **wrap** onto continuation lines
(action bars, message bodies) or **trim to the width** with a trailing `…`
(reply quotes, reaction rows, snippets) — never a fixed character cap that the
terminal then clips. Size against the real content width, not a magic number.

The design system: the `split_main` vertical stack, the top identity bar, the
single `widgets::list_table` list renderer, `draw_search_box`, `draw_cmd_log`,
`draw_status_strip`, the navigable `draw_confirm_popup`, and the
`LineEditor` + `editor_spans` text-input model. The shared input mechanics
live in `tui::input::common`. The decoration budget is small and lives
**only** on the splash / login screens — working screens carry no
background art.

Everything documented here is backed by real code in `src/tui/view/` and
`src/tui/input/`. When you change a component, update this file in the same
change.

## Screen layout — the `split_main` stack

The router `view::mod::draw` picks a base screen per `Screen`, then overlays
any popup on top (popups draw the base screen underneath first). The
terminal floor is **70×18** (`view::mod::MIN_W`/`MIN_H`); below it every
screen is replaced by the centered "terminal too small" notice
(`view::mod::draw_too_small`): three vertically-centered lines — an
`error`-colored bold `Terminal too small` title, a `dim` `Resize to at least
{MIN_W}×{MIN_H} (currently {w}×{h})` line, and a `dim` `Ctrl+C to quit` hint.

Every signed-in screen is built from the same vertical stack via
`view::mod::split_main(area, identity_content_rows)` →
`[identity, header, body, cmdlog, status]`:

- **identity** — `widgets::draw_identity_bar`: `user <name> · device <dev> ·
  N unread`. Size the slot with `widgets::identity_content_rows(app, width)`
  (+2 for borders); it is currently always 1 row.
The inbox is a **unified two-pane "Home"** (Discord-style): the conversation
tree on the left, the open chat on the right — no separate full-screen
conversation on normal terminals.
- **header** — one shared row: the tree filter (`─[Alt+F]-Search`, above the
  tree) + the in-chat search (`conversation::draw_chat_header`, above the
  chat). The conversation **name lives on the Messages panel title**
  (`Messages — <name>` via `conversation::chat_title`, plus `📌 #id` when
  pinned) — no separate "Conversation" box.
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
    (`chat::reveal_in_tree` — expand its group + move the cursor). The first
    (slow) load shows a `widgets::draw_skeleton` whose `Loading chats…` legend
    **replaces** the `Chats   #` column header until the list arrives.
  - the right pane is `conversation::draw_chat` (messages + compose; the header
    name/search are on the shared top row) when a conversation is open, else a
    placeholder. `Focus::Chat` routes keys to `input::conversation::handle`
    (compose / select / in-conv search); `Esc` on an empty compose closes the
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
    (`{conv}-{msg}.ext`, background via the worker), a `⏳ loading…` placeholder
    holds the row meanwhile, and the run loop paints the cached graphic over the
    reserved rows — repainting only when the visible set/positions change, with
    a per-`(path,size)` chafa-output cache so scrolling is cheap. **`symbols`
    output is parsed into Ratatui spans and rendered *in-buffer*** (the chafa
    ANSI → `image::symbols_to_lines`, spliced over the reserved rows) so scroll,
    overlay occlusion and clearing all work via the frame diff — no
    direct-to-stdout ghosting; only the true-graphics protocols paint over the
    rows from the run loop. **Animated GIFs play inline**: ImageMagick
    (`convert -coalesce`) splits the frames to a disk cache once
    (`image::extract_gif_frames`, `App::gif_anims`), each frame is chafa-cached,
    and the run loop advances them by wall-clock (`anim_ms`, `GifFrames::frame_at`),
    polling at ~12 fps while a GIF is on screen (`gif_animating`). In select
    mode `c` **copies the image** to the clipboard (`wl-copy`/`xclip`/`pbcopy`,
    `image::copy_to_clipboard`) when an image is selected, and `s` downloads it.
- **cmdlog** — `widgets::draw_cmd_log`: the rolling `keybase …` command log
  (6 rows, `✓ cmd  →  detail  (3s)`, newest at the bottom). When focused
  (`Focus::CmdLog`) it is a **visual multi-select**: a `▶` cursor walks the
  whole history (the window follows it; `App::cmdlog_cursor`), `Alt+Shift+K/J` (or `Alt+Shift+↑/↓`) shades a contiguous range (`App::cmdlog_anchor`), `Space` toggles a line
  (`●`, `App::cmdlog_marks`), and `y`/`Enter` copy the **full** line(s) while
  `c` copies the **detail only** (`chat::do_copy_cmd_log(full)`) — marked
  lines, or the cursor line; the selection is kept so both copies work. `Esc`
  clears the selection then leaves. The title shows `cursor/total · N sel`.
  The chat's **Select mode** (`Alt+V`) uses the **same multi-select**: `Space`
  marks messages, `Alt+Shift+K/J` (or `Alt+Shift+↑/↓`) shades a range, `y` copies author + time + body
  and `c` copies bodies only (`chat::do_copy_messages(full)`), separated by a
  blank line / newline; with a selection active the action bar collapses to
  the reduced copy / react set.
  Worker ops carry their duration (request → response), formatted by
  `domain::format_duration` as a single smallest-unit value
  (`ms`/`s`/`m`/`h`/`d`/`y`).
- **status** — `widgets::draw_status_strip`: feedback (spinner / ✓ / ✗) when
  an action is in flight, else the per-focus footer hint on the left with
  **`F1 help` anchored right**.

The **conversation** screen uses a custom layout (identity · header ·
messages · compose · status) because the message stream is a *viewer*, not
a grid. The screen is mouse-interactive: click a message to select it
(enters select mode on that row) and scroll the wheel to page history
(`mouse_areas` carries the messages viewport rect + a per-message row map).
The compose box is **multi-line**: `Alt+Enter` inserts a newline (`Enter`
sends), the box grows with the line count (capped, then it scrolls to keep
the cursor visible), and `widgets::editor_lines` renders the multi-row
cursor. Single-line inputs (search, react, new-conversation) keep
`widgets::editor_spans`. The conversation **header row is split horizontally**:
the `Conversation` name (left, 35%) + a `draw_search_box` **in-conversation
search** (right, 65% — wider, the priority). `Ctrl+F` focuses it — and it is
also a **Tab stop** (`Focus::ChatSearch`, between `Chat` and `CmdLog`, skipped
when no conversation is open); Tabbing in activates the search and Tabbing out
closes it (`conv_search_active` stays in sync via `input::inbox::set_focus`).
Typing + `Enter` runs `keybase chat api searchregexp`
over the open conversation (full history), and the match list renders in the
message viewport (`render_conv_search_results`) — each hit is **two rows**:
the sender + snippet, then a dim **day/time** line (`InboxHit::sent_at` from
the hit's `ctime`, formatted by `message_time`) for context. `↑/↓` pick,
`Enter` jumps to + highlights the message (via `pending_search_jump`,
paginating older if needed), `Esc` closes. The **login** screen is the signed-out exception: it omits the
identity bar and shows the figlet/starfield backdrop with a "run `keybase
login`, then R" hint.

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
  `reactions` field); the standalone reaction events are dropped from the
  stream so they don't show as stray `reacted :emoji: on msg #N` lines.
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
  (`+ react`, `r reply`, `e edit`/`d delete` on your own messages,
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
the Tab cycle when no conversation is open), `CmdLog` (the identity bar and
status strip are chrome, not focus targets).
Conventions:

- **Every focusable panel's border tag is its literal go-to combo** (no bare
  `/`): `[Alt+F]` chat Filter, `[Alt+C]` Chats, `[Alt+M]` Messages, `[Ctrl+F]`
  in-chat search (classic find), `[Alt+L]` command Log. The jumps are
  **global** — they fire from any focus, even mid-compose (weechat-style
  modifier chords), so the tag always tells the truth. Re-homed to free the
  section letters: copy label `Alt+Y` (yank), mark read `Alt+E` (seen).
  `Tab`/`Shift+Tab` also cycle focus via `input::common::cycle_focus`.
- **Positional pane navigation** — a `Ctrl+W` leader (vim window-nav) arms
  `App::pending_pane_nav`; each following `h/j/k/l` or arrow moves to the
  spatial neighbour (`input::inbox::pane_target`: filter/in-chat-search on the
  top row, Chats/Chat in the body, command log across the bottom). The leader
  **stays armed across consecutive directions**, so two keys make a diagonal
  (e.g. `k` then `h` = up-left). `Esc`/`Enter` exit; any other key exits and is
  re-processed. The status strip shows the armed hint.
- A panel is "focused" → accent + bold border/title
  (`view::mod::titled_block(title, focused, app)`); otherwise the `inactive`
  tint.

## Lists & tables — the single pattern

**Numbered section borders.** Each list section carries a `─[N]-` tag woven into
its top border. The inbox numbers
its panels `─[Alt+F]-Search`, `─[Alt+C]-Chats`, `─[Alt+M]-Chat`, `─[Alt+L]-Command log`; Teams
uses `─[1]-Teams`, `─[2]-Command log`. `draw_search_box` adds the `─[/]-` tag
itself; `draw_cmd_log` takes the panel number; list titles are prefixed at the
call site.

**All multi-column lists render through `widgets::list_table(...)`.** It owns:
header row (dim + bold), bordered titled block, `▶ ` selection symbol,
`selected_bg` + bold row highlight, `column_spacing(2)`, and **persisted
scroll** (writes back the offset). Build `headers: &[&str]`,
`widths: &[Constraint]` and `rows: Vec<Row>`, then call it. Title is built
with `widgets::list_title` → **`"<Thing> · {filtered} of {total}"`**.

Rules:
- **Size content columns to the *visible* (filtered) rows** with
  `widgets::col_width(indices, lo, hi, |i| …len)`. **Never** put a stretching
  `Constraint::Min(..)` on a non-final content column — it shoves trailing
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

- `view::mod::titled_block(title, focused, app)` — the bordered block:
  focused = accent + bold, else `inactive`. Used for every panel.
- `view::mod::split_main(area, identity_rows)` — the standard vertical stack.
- `widgets::list_table` / `list_title` / `col_width` / `middle_ellipsis` /
  `trim_end_ellipsis` / `table_row_at` — the list renderer + sizing helpers.
- `widgets::draw_identity_bar` / `identity_content_rows` — the top identity
  bar.
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
- `widgets::checkbox_spans` / `chip_span` — shared toggle / tab spans.

## Overlays & confirmations

Popups are centered and **always drawn over their base screen** by
`view::mod::draw` (inbox under logout/new-conv/global-search; conversation
under react/delete/download).

- **Confirmations** (`ConfirmLogout`, `ConfirmDeleteMessage`,
  `ConfirmConvAction`) render through `widgets::draw_confirm_popup`
  (centered, double border): `←/→` (or `Tab`/`h`/`l`) move between
  **confirm/cancel**, `Enter` activates the highlighted one, `y`/`n`/`Esc`
  are shortcuts. **Default highlight = cancel** for the destructive action
  (`logout_yes` / `delete_msg_yes` / `conv_action_yes` default `false`).
  Classified by `input::common::confirm_key`/`ConfirmInput`.
  `ConfirmConvAction` is the generic home for per-conversation status
  actions (`App::ConvAction`: ignore/…) — each variant supplies its own
  title/note and maps to a `setstatus` value, so adding one is a single
  enum arm.
- **Input popups** (`NewConversation`, `React`, `DownloadAttachment`,
  `SearchGlobal`): a centered box with an `editor_spans` field and
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
`clear`). Here it derives `ZeroizeOnDrop` (a secretbase-specific divergence)
because any input can hold sensitive chat content — see `CLAUDE.md`. Handlers feed keys through `input::common::route_line_editor`
(returns `true` when the text changed → rebuild a filter) or, for the inbox
filter box, `input::common::search_key`/`SearchAction`. Rendering is always
`widgets::editor_spans`. Empty boxes show a dim placeholder
(`theme.placeholder`).

## Keybindings (global conventions)

- `/` focus search · `Esc`/`h` back · `F1` help · `F9` Settings ·
  `Tab`/`Shift+Tab` cycle
  focus · **only `Ctrl+C` quits** (everything else is free for navigation /
  type-to-search).
- `j/k` + `↑/↓` navigate · `PgUp/PgDn` page · `g/G` top/bottom ·
  `Enter`/`l` open.
- **Actions use the `Alt+<letter>` convention**: `Alt+N` new conversation,
  `Alt+C` copy, `Alt+M` mark read, `Alt+U`/`Alt+O` mute/unmute, `Alt+I`
  ignore, `Alt+T` teams, `Ctrl+G` global search, `Shift+L` logout. In the
  conversation,
  `Alt+V` select mode, `Alt+E`/`Alt+D` edit/delete own, `Alt+J`/`Alt+P`
  react/pin. The footer shows only a few; the full per-screen list lives in
  the help popup and the `README.md` tables — **keep both in sync**.

## Theme (`tui::theme`)

Resolved from the optional `[theme]` section of `config.toml`; every key
optional, partial configs valid. Fields: `accent`, `inactive`, `selected_bg`,
`success`, `error`, `dim`, `foreground` (defaults to `Color::Reset` to
inherit the terminal), `placeholder`, `muted`, the restrained splash
starfield colors `star_dim`/`star_mid`/`star_bright`, and the conversation
marker colors `conv_dm` / `conv_team` / `conv_unread`. **Don't hardcode
colors — use these.**

**Presets.** Themes are built from a `Palette` (13 named roles) via
`Theme::from_palette`, which maps the core roles and derives the starfield +
conversation-marker colors. Four presets ship (`Preset::ALL`:
`catppuccin-mocha`, `dracula`, `nord` (default — `Preset::DEFAULT`),
`catppuccin-latte`); `name = "<preset>"` in `[theme]` picks the base and per-key
hex entries override it. The Settings picker applies live. Adding a preset = one
`Palette` arm in `Preset::palette`.

## Settings overlay (`F9`)

`F9` opens a centered **Settings** overlay (`Screen::Settings`, drawn over
`settings_from` like Help) — `view::settings::draw_popup`, input in
`input::settings`. Layout: a left **section sidebar** + the active section's
**panel**, `Tab`/arrows move between and within them. It's **sectioned so the
preferences surface can grow** (Clipboard, Notifications…) without changing the
chrome; today the only section is **Theme**, a preset picker that **previews
live** as you move (`App::settings_preview_theme`) — `Enter` applies + persists
`name = "<preset>"` to `config.toml` (`SettingsPort::write_theme_name`), `Esc`/`F9`
cancels and restores the pre-open theme. Add a new section by extending
`SettingsSection`.

## Golden rules

1. **Reuse, don't reinvent** — a new list = `list_table`; a new input =
   `LineEditor` + `editor_spans` (routed via `input::common`); a new panel =
   `titled_block`; a new confirm = `draw_confirm_popup`; a new signed-in
   screen = `split_main`.
2. **Fix the class, not the instance** — when you change one screen, change
   every screen with the same pattern (and update this file).
3. **Every change stays coherent** with the rest of the UI. If you diverge,
   update the spec here first and apply it everywhere.
4. **No decorative noise on working screens** — the figlet + starfield belong
   to splash/login only. Screen identity comes from the bordered block titles.
