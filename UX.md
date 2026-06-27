# UX.md — secretbase UI/UX specification

The canonical design system for secretbase's terminal UI. **Read this
before any UI change or new feature, and keep it up to date before every
push** (see `CLAUDE.md`). The whole point is that every screen looks and
behaves the same — when in doubt, reuse an existing component; never invent
a one-off.

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
- **header** — the search box (`widgets::draw_search_box`) on the inbox.
  Screens without a filter (Teams) drop this slot.
- **body** — the list/table (see below) or a detail layout. The inbox splits
  it 22 % / 78 % into the filter sidebar and the conversation list.
- **cmdlog** — `widgets::draw_cmd_log`: the rolling `keybase …` command log
  (6 rows, `✓ cmd  →  detail  (3s)`, newest at the bottom; `cmd_log_scroll`
  walks back). Worker ops carry their duration (request → response),
  formatted by `domain::format_duration` as a single smallest-unit value
  (`ms`/`s`/`m`/`h`/`d`/`y`).
- **status** — `widgets::draw_status_strip`: feedback (spinner / ✓ / ✗) when
  an action is in flight, else the per-focus footer hint on the left with
  **`F1 help` anchored right**.

The **conversation** screen uses a custom layout (identity · header ·
messages · compose · status) because the message stream is a *viewer*, not
a grid. The **login** screen is the signed-out exception: it omits the
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
  reactions trigger a quiet re-read so they reproject correctly.
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

## Panels & focus

The inbox's focusable panels are the `screens::Focus` variants: `Search`,
`Filters`, `List`, `CmdLog` (the identity bar and status strip are chrome,
not focus targets). Conventions:

- `/` jumps to Search; `Tab`/`Shift+Tab` cycle focus via
  `input::common::cycle_focus` over `FOCUS_ORDER`.
- A panel is "focused" → accent + bold border/title
  (`view::mod::titled_block(title, focused, app)`); otherwise the `inactive`
  tint.

## Lists & tables — the single pattern

**Numbered section borders.** Each list section carries a `─[N]-` tag woven into
its top border. The inbox numbers
its panels `─[/]-Search`, `─[1]-Filters`, `─[2]-Inbox`, `─[3]-Command log`; Teams
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
- Map mouse clicks with `widgets::table_row_at(rect, y, scroll, len)` (via
  `input::common::list_click`).
- Truncate an overflowing primary column with `widgets::middle_ellipsis`
  (head-biased, UTF-8 safe).

Screens using `list_table`: inbox (filters + conversation list), teams. The
**conversation** message viewer and the **global-search** results are the
exceptions (custom `Paragraph` / `List`).

## List state convention (`App`)

The inbox keeps: `filtered_cache: Vec<usize>` (indices into `conversations`
surviving the active filter + search), `search: LineEditor`, `list_selected`
(indexes the **filtered** cache), `list_scroll`. Rebuild via
`rebuild_filter()`; ranking is `domain::search::fuzzy_score_lowered` over the
pre-lowercased `LoweredConversation` projection (channel 100, topic 60,
creator 20), then sorted most-recent-first by `active_at_ms`.
`rebuild_lowered` refreshes the projection + per-filter counts once per load.
`clamp_list_selected` runs in `draw` so the selection and the `· N of M`
title never disagree. Selection always indexes the filtered cache.

## Chrome & widgets (`view::widgets` + `view::mod`)

- `view::mod::titled_block(title, focused, app)` — the bordered block:
  focused = accent + bold, else `inactive`. Used for every panel.
- `view::mod::split_main(area, identity_rows)` — the standard vertical stack.
- `widgets::list_table` / `list_title` / `col_width` / `middle_ellipsis` /
  `table_row_at` — the list renderer + sizing helpers.
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
  `input::common::route_line_editor`.

## Help popup

`view::help::draw` is **context-aware and scrollable**: it shows the
shortcuts for the screen it was opened from (`App::help_from`, stamped on
F1) plus a shared Global section. The renderer owns the viewport — it clamps
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
