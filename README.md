# secretbase

A terminal user interface (TUI) for the [Keybase CLI](https://book.keybase.io/docs/cli),
written in Rust on top of [Ratatui](https://ratatui.rs/).

The project follows a hexagonal architecture: pure domain types in the
core, ports as trait abstractions, adapters as the only layer allowed to
spawn subprocesses or touch the filesystem, and a driving TUI adapter on
top.

```text
 main ──► tui ──► flows ──► ports ◄── adapters
                       ▲
                       └── domain (used by every layer above)
```

## Status

**v0.1.0**

A working terminal client for everyday Keybase chat. Highlights:

* `keybase status --json` on boot to detect the session; logout with
  confirmation.
* **Login screen** (signed out): a form to **log in from the app** — a
  non-interactive **paper-key** login (`keybase login --devicename <d> <user>`,
  paper key on stdin) for a new device, and a **"Log in in terminal"** button
  that cedes the terminal to interactive `keybase login` for the passphrase
  path when the device is already provisioned. (Keybase has no scriptable
  username+password login; see `CLI.md`.)
* Discord-style unified two-pane Home: a collapsible **conversation tree**
  (Direct messages + a group per team; unread in bold) on the left, the open
  **chat** on the right; the header search fuzzy-filters the tree.
* **Real-time** updates over `keybase chat api-listen` — incoming messages
  append live and bump the inbox; the periodic `list` is only a safety-net
  resync.
* **Compose & message actions**: send (multi-line), edit / delete / react /
  pin / reply, optimistic send with resend-on-failure, `@`-mention
  autocomplete, inline markdown (`*bold*`, `_italic_`, `~strike~`,
  `` `code` ``, quotes) and **syntax-highlighted** fenced code blocks
  (` ```rust ` …, via `syntect`). The stream **groups consecutive messages by
  author** (per-user colored names), with **day separators** and a **new
  messages** marker.
* **Attachments**: upload via the built-in file picker, download to a chosen
  folder, and inline image thumbnails / animated-GIF playback via `chafa`.
* **Search**: local fuzzy filter, server-side inbox search (`Ctrl+G`),
  in-conversation regexp search (`Ctrl+F`), a quick switcher (`Ctrl+K`), and a
  **command palette** (`Ctrl+P`) — a fuzzy list of every action (with its
  keybinding), run by name.
* **Teams**: list your memberships with role and member count (`t`); a
  **channel browser** (`c` on a team) lists every channel to join / open /
  leave, plus create / rename / delete and toggle a channel as a team **default**
  (`listconvsonname` · `join` · `leave` · `newconv` · `rename-channel` ·
  `delete-channel` · `default-channels`). A **members** view (`m` in the browser,
  or `Alt+P` on an open team channel) lists members by role and adds / removes
  them (`listmembers` · `addtochannel` · `removefromchannel`).
* New conversation (`n`), mark read (`e`), ignore / block / report (`Shift+I` /
  `Shift+B` / `Shift+R`), unhide (restore blocked/reported by name), copy label,
  plus **local-only** ★ favorite (`s`) and mute (`u`).
* Full **Settings** screen (`F10`): your identity, a live theme picker, and
  every preference below — edited in place and saved immediately.

## Requirements

* Rust 1.95 or newer (`rust-toolchain.toml` pins the channel).
* The `keybase` binary on `$PATH`, with a logged-in session.
* A clipboard tool on Linux/X11 (`xclip` or `xsel`) / Wayland (`wl-copy`).
  macOS uses `pbcopy`/`pbpaste`. None of these is required — when there's no
  display server (a **headless / SSH** box, the common case), copy falls back to
  the **OSC 52** terminal escape, which puts the text on your *local* terminal's
  clipboard with no tool at all (the terminal must support OSC 52; tmux needs
  `set-clipboard on`). Note: the OSC 52 path can't time-clear the clipboard
  (`clipboard_clear_secs` is a no-op there — no reliable read-back).

## Build & run

```sh
cargo build --release
./target/release/secretbase
```

Or from the project root:

```sh
cargo run --release
```

## Keybindings

Keys follow a **gradient** convention: a **lowercase letter** acts on the
focused list (frequent, safe), **`Shift+letter`** is the loud/destructive
tier, **`Ctrl`** is global (search, switcher, quit), **`Alt`** jumps to a
panel, and **`/`** focuses search. A text field (compose, the filter box)
owns bare letters as typed text — its actions move to `Alt`/`Ctrl`. **Only
`Ctrl+C` quits.** The help popup (`F1`) is the in-app source of truth and
stays in sync with these tables.

### Inbox

| Key | Action |
|---|---|
| `↑` / `↓` · `k` / `j`   | Navigate |
| `PgUp` / `PgDn`         | Page |
| `g` / `G`               | Top / bottom |
| `Enter` / `→` / `l`     | Open conversation / expand group |
| `←` / `h`               | Close chat / collapse group |
| `/`                     | Focus the chat **F**ilter (or `Alt+F`) |
| `Tab` / `Shift+Tab`     | Cycle focus (search · tree · chat · log) |
| `Ctrl+W` then `h`/`j`/`k`/`l` (or arrows) | Move between panels **positionally** (vim window-nav); stays armed so two keys do a diagonal (`Esc` exits) |
| `Ctrl+W` then `z`       | Zoom the chat column (tmux-style toggle; tree + command log hidden) |
| `Alt+C` / `Alt+M`       | Go to `[Alt+C]` Chats / `[Alt+M]` Messages (works mid-compose) |
| `Alt+F` / `Alt+L`       | Go to `[Alt+F]` Filter / `[Alt+L]` Command log |
| `Ctrl+F`                | In-chat search — modal (open conversation) |
| `Ctrl+G`                | Global search (inbox & Teams) |
| `Ctrl+K`                | Quick switcher — jump to a conversation (inbox & Teams) |
| `Ctrl+P` / `:`          | Command palette — run any action (`:` from any non-typing surface, vim-style) |
| `Ctrl+N`                | Next unread — mentions first, then DMs, then channels (wraps) |
| `Ctrl+O`                | Previous conversation (vim's alternate-buffer toggle) |
| `Ctrl+D` / `Ctrl+U`     | Half-page down / up in every list and viewer |
| `Ctrl+J` / `Ctrl+K`     | Selection down / up inside every picker popup |
| `Alt+1`..`Alt+9`        | Pick-and-activate the Nth row in any query picker |
| **lowercase = frequent, safe** | |
| `n`                     | New conversation |
| `r` / `F5`              | Refresh inbox |
| `y`                     | Yank (copy) label to clipboard |
| `e`                     | Mark as sEEn (read) |
| `u`                     | Toggle mute (local only — hides the unread badge; not synced) |
| `s`                     | Toggle ★ favorite (local only — not synced to Keybase) |
| `t`                     | Teams |
| `c`                     | Channel browser (on a team) — join/open/leave, new/rename/delete, `t` default |
| **`Shift` = loud / destructive** | |
| `Shift+I`               | Ignore conversation (hidden until next message) |
| `Shift+B`               | Block conversation (hide for good) |
| `Shift+R`               | Report conversation (flag to Keybase + hide) |
| `Shift+H`               | Unhide — restore a blocked/reported chat by name |
| `Shift+L`               | Logout (confirmation) |

In the **Chats** tree, `↑` / `↓` move; `Enter` / `→` / `l` open the selected
conversation (or expand a group) and `←` / `h` close the open chat (or
collapse a group); the header search fuzzy-filters it.

### Conversation

| Key | Action |
|---|---|
| typing                  | Extend draft |
| `Enter`                 | Send / save edit |
| `Alt+Enter` / `Shift+Enter` | New line (multi-line message; Shift needs a kitty-protocol terminal) |
| `↑` (empty compose)     | Edit your last message (at the bottom of history) |
| `:emo…` in compose      | Emoji autocomplete (`Tab`/`Enter` insert, `Esc` dismiss) |
| `Ctrl+E`                | Edit the draft in `$VISUAL`/`$EDITOR` |
| `@…` then `Tab`         | Mention autocomplete (`↑`/`↓` pick) |
| `↑` / `↓` · `PgUp` / `PgDn` | Scroll history |
| `←` / `→` · `Home` / `End`  | Compose cursor (empty compose: `Home`/`End` jump to oldest / latest message) |
| `Ctrl+W` / `Ctrl+U`     | Delete word / to line start (works in every text input) |
| `Ctrl+←` / `Ctrl+→` · `Ctrl+A` / `Ctrl+E` | Word jump · line start / end (every input) |
| `Ctrl+F`                | Search this conversation (jump to a match) |
| mouse                   | Click a message to select, scroll to page history |
| `Alt+V`                 | Select mode — act on a message (edit/delete/react/pin/reply) |
| `Alt+I`                 | Insert an emoji into the draft |
| `Alt+G`                 | GIF search — pick a giphy GIF and send it |
| `Alt+N`                 | Jump to the `new messages` divider |
| `Alt+U`                 | Unpin channel |
| `Alt+H`                 | Hide the pin banner (local only) |
| `Alt+R`                 | Resend failed message |
| `Alt+A`                 | Attach a file (opens the file picker) |
| `Alt+E`                 | Edit your most recent own message (Slack's up-to-edit) |
| `Alt+P`                 | Members (team channels only) — add / remove |
| `F5` / `Ctrl+R`         | Refresh |
| `Ctrl+Y`                | Copy label |
| `Esc`                   | vim chain: cancel edit → cancel reply → **Select mode** → close (drafts are never destroyed) |

In **Select** mode: `↑` / `↓` (·`PgUp`/`PgDn`) move the cursor · `g`/`G`
(`Home`/`End`) jump to the first / latest message · `v` sets a **visual
anchor** — every motion (`j`/`k`, paging, `g`/`G`, `{`/`}`) then extends the
range, `v` again clears it (`Shift+↑/↓` still shades too) · `{` / `}` jump by
**speaker run** · `Space` mark / unmark. **`x` delete and `+` react act on
the whole marked set** (or the cursor if nothing's marked) — a multi-message
action runs one at a time and you stay in Select mode with the shading cleared.
`y` copy selection (author + time + body) · `c` copy bodies only · `o` / `u`
open / copy the first link · `[` / `]` previous / next @mention of you ·
`w` who reacted · `Enter` (or clicking the selected row again) on
a **reply** jumps to the quoted message — older pages load automatically if
it's outside the loaded window; on anything else it returns to compose. **Fast react:** `Esc` (or `Alt+V`) `+` `Enter` —
three keys — reacts to the latest message with your most-used emoji (the
picker is frecency-sorted and `Enter` on an empty query takes the top row). · `e` / `p` edit / pin the cursor message · `r` / `s`
reply / download · `/` search this conversation · `n` / `N` jump to the next /
previous search hit (hits are retained after `Ctrl+F`, vim-style) · `i` / `a` /
`Enter` return to Compose · `Esc` clears the selection, then closes the
conversation.

The **command log** (Tab to focus it) has the same visual multi-select:
`↑/↓` move (·`PgUp`/`PgDn` page · `g`/`G` oldest / newest) · `v` visual anchor
(j/k extend) · `Shift+↑/↓` range ·
`Space` mark · `y` copy full line(s) · `c` copy detail only · `Esc` clear /
leave.

### Teams (`t`) & modals

| Key | Action |
|---|---|
| `↑` / `↓` · `k`/`j` · `PgUp`/`PgDn` · `g`/`G` | Navigate any list |
| `Enter` / `→` / `l` (Teams) | Browse the selected team's channels |
| `r` / `F5`              | Refresh · `Esc` back to inbox |

**Channel browser** (`c` on a team): `Enter` open/join · `n` new · `r` rename ·
`/` filter · `t` toggle team-default · `m` members · `Shift+L` leave · `x` delete ·
`F5` refresh. **Members** (`m` in the browser, or `Alt+P` on an open team
channel): `/` filter · `a` add · `x` remove · `F5` refresh.

### Login (signed out)

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` · `↑`/`↓` | Move between fields / buttons |
| typing                  | Edit the focused field |
| `F2`                    | Reveal / hide the paper key |
| `Enter`                 | Log in (paper key on a field/`Log in`; native on `Log in in terminal`) |
| `F5`                    | Retry the status check |
| `Esc`                   | Clear the focused field |
| `Ctrl+C`                | Quit |

### Global

| Key | Action |
|---|---|
| `F1`                    | Toggle help |
| `F10`                   | Open Settings |
| `Ctrl+C`                | Quit |

## Not supported (CLI limitations)

secretbase is a wrapper over the `keybase` binary, so it can only do what the
CLI exposes. The principle for anything the CLI **can't drive transparently**:
either keep it fully local (so we own the state and the undo), or leave it out —
never a half-synced state that silently drifts.

* **Favorite and mute — handled fully locally.** Keybase's server-side
  conversation `status` (`favorite`, `muted`) **can't be read back**: the
  `keybase chat api {"method":"list"}` JSON (`ConvSummary`) omits it (the data
  exists at the service-RPC level — e.g. `IsMuted` — but the CLI doesn't project
  it). So secretbase **doesn't touch the server status at all** and keeps its
  own **local-only** state instead, persisted in `config.toml`
  (`favorites` / `muted`), never synced:
    * `s` toggles a local ★.
    * `u` toggles a local mute, which only suppresses secretbase's own
      unread indicators (the `●` dot, bold, the unread count + filter) — the TUI
      has no notifications to silence, so that *is* what "mute" means here.
      ⚠️ It does **not** silence Keybase notifications on your phone / desktop
      GUI (that's the server-side mute, which we don't use).
* **Typing indicators, read receipts, edit/delete deltas** over the push
  stream — the `keybase chat api-listen` callbacks return `nil` for these, so
  they're not available to any wrapper.

## Configuration

Everything below can be edited live from the in-app **Settings** screen
(`F10`) — changes save to `config.toml` immediately — or by hand in
`~/.config/secretbase/config.toml`:

```toml
clipboard_clear_secs = 30          # clipboard auto-clear (0 disables)
list_inbox_timeout_secs = 30       # wall-clock budget for `keybase chat api list`
download_timeout_secs = 300        # wall-clock budget for attachment downloads
auto_mark_read = true              # mark conversations as read when opened
inbox_refresh_secs = 180           # safety-net inbox resync cadence (0 disables;
                                    # real-time updates come from api-listen)
image_protocol = "auto"            # inline image attachments: auto | kitty | sixel
                                    # | iterm | symbols | off  (all via `chafa`)
image_symbols = "sextant+block+space"  # chafa --symbols set (symbol path only):
                                    # sextant (default), add `octant+` for denser
                                    # output on Unicode-16 fonts, or `half` anywhere
emoji_style = "glyph"              # reaction display: glyph (default) | shortcode
                                    # (`:alias:` text — legible when the terminal
                                    # renders emoji as tofu; a TUI can't set fonts)
icon_style = "unicode"             # UI icons: unicode (default, renders on any
                                    # font — right for a headless/SSH terminal)
                                    # | nerd (needs a patched Nerd Font)
web_previews = true                # fetch giphy GIFs linked in messages directly
                                    # from giphy for inline playback (needs `curl`;
                                    # reveals your IP to giphy, like any link
                                    # preview — false renders the plain URL)
giphy_api_key = ""                 # your own Giphy API key (free at
                                    # developers.giphy.com) enables the Alt+G GIF
                                    # search; Keybase's own giphy key is server-
                                    # vended and unreachable outside its GUI.
                                    # Editable in-app: Settings → Images

[theme]
name         = "nord"              # bundled preset (dark): nord (default),
                                    # catppuccin-mocha / -frappe / -macchiato,
                                    # dracula, tokyonight, tokyonight-storm,
                                    # gruvbox-dark, rose-pine, everforest,
                                    # kanagawa, one-dark, solarized-dark,
                                    # monokai-pro; (light): solarized-light,
                                    # rose-pine-dawn, catppuccin-latte
accent       = "#cba6f7"           # optional per-key overrides on top of the preset
inactive     = "#6c7086"
selected_bg  = "#313244"
# … see src/tui/theme.rs for the full key list
```

Inline image thumbnails require the **`chafa`** binary on `PATH` (it emits the
kitty/sixel/iterm graphics or ANSI symbols); without it, image attachments fall
back to their filename/size text. Set `image_protocol = "off"` to disable.
**Animated GIFs** play inline when **ImageMagick** (`convert`) is installed (it
splits the frames, which are cached on disk); otherwise a GIF shows its first
frame. **Giphy links** (the GUI's GIF picker posts a `media.giphy.com` URL)
also play inline: the `.gif` rendition is fetched directly from giphy with
`curl` — host-allowlisted and size-capped; disable with `web_previews = false`.

Pick a bundled palette with `name`, then override individual keys if you want.
Omitting `name` keeps the default (Nord with terminal-inherited text). You can
also switch presets live in-app from the **Settings** screen (`F10`).

## Architecture

```
src/
├── main.rs                 # composition root
├── lib.rs                  # crate doc + module declarations
├── domain/                 # pure types — no I/O
│   ├── conversation.rs     # Conversation, Channel, MembersType
│   ├── message.rs          # Message + MessageContent variants
│   ├── team.rs             # TeamMembership, TeamRole
│   ├── identity.rs         # IdentityInfo (keybase status)
│   ├── search.rs           # Fuzzy ranking
│   └── validation.rs       # username validators
├── ports/                  # trait abstractions
│   ├── keybase.rs          # KeybasePort (status, list/read/send/…)
│   ├── clipboard.rs        # ClipboardPort
│   ├── opener.rs           # OpenerPort (open a URL in the browser)
│   └── settings.rs         # SettingsPort + UserSettings
├── adapters/               # concrete impls — only layer doing I/O
│   ├── keybase_cli/        # shells out to `keybase` binary
│   │   ├── process.rs      # subprocess + timeout polling
│   │   ├── codec.rs        # JSON request builders
│   │   └── json.rs         # parsing helpers
│   ├── clipboard_system.rs # wl-copy / xclip / xsel / pbcopy
│   ├── opener_system.rs    # xdg-open / open / cmd start
│   └── settings_toml.rs    # ~/.config/secretbase/config.toml
└── tui/                    # Ratatui driving adapter
    ├── app.rs              # global state container
    ├── action.rs           # ActionState, PendingAction queue
    ├── screens.rs          # Screen, Focus enums
    ├── theme.rs            # color palette
    ├── mouse_areas.rs      # hit-test rectangles
    ├── debug_log.rs        # opt-in ~/.secretbase.log
    ├── flows/              # auth, teams, palette + chat/{inbox,messages,channels,attachments,search}
    ├── input/              # per-screen keyboard + mouse
    └── view/               # per-screen Ratatui renderers
```

## Testing

```sh
cargo test          # ~330 unit tests
cargo clippy --all-targets -- -D warnings
```

## License

MIT — see [`LICENSE`](LICENSE).
