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
  (` ```rust ` …, via `syntect`).
* **Attachments**: upload via the built-in file picker, download to a chosen
  folder, and inline image thumbnails / animated-GIF playback via `chafa`.
* **Search**: local fuzzy filter, server-side inbox search (`Ctrl+G`),
  in-conversation regexp search (`Ctrl+F`), and a quick switcher (`Ctrl+K`).
* **Teams**: list your memberships with role and member count (`Alt+T`); a
  **channel browser** (`Alt+K` on a team) lists every channel to join / open /
  leave, plus create / rename / delete (`listconvsonname` · `join` · `leave` ·
  `newconv` · `rename-channel` · `delete-channel`).
* New conversation (`Alt+N`), mark read, ignore / block / report, unhide
  (restore blocked/reported by name), copy label, plus **local-only** ★ favorite
  (`Alt+S`) and mute (`Alt+U`).
* Full **Settings** screen (`F10`): your identity, a live theme picker, and
  every preference below — edited in place and saved immediately.

## Requirements

* Rust 1.95 or newer (`rust-toolchain.toml` pins the channel).
* The `keybase` binary on `$PATH`, with a logged-in session.
* A clipboard tool on Linux/X11 (`xclip` or `xsel`) / Wayland (`wl-copy`).
  macOS uses `pbcopy`/`pbpaste`. None is required to run the TUI — only
  to copy.

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

Actions use the `Alt+<letter>` convention; **only `Ctrl+C` quits**
(every other key is free for navigation / type-to-search). The help
popup (`F1`) is the in-app source of truth and stays in sync with these
tables.

### Inbox

| Key | Action |
|---|---|
| `↑` / `↓` · `k` / `j`   | Navigate |
| `PgUp` / `PgDn`         | Page |
| `g` / `G`               | Top / bottom |
| `Tab` / `Shift+Tab`     | Cycle focus (search · tree · chat · chat-search · log) |
| `Ctrl+W` then `h`/`j`/`k`/`l` (or arrows) | Move between panels **positionally** (vim window-nav); stays armed so two keys do a diagonal (`Esc` exits) |
| `Alt+F`                 | `[Alt+F]` Focus the chat **F**ilter |
| `Alt+C` / `Alt+M`       | Go to `[Alt+C]` Chats / `[Alt+M]` Messages (works mid-compose) |
| `Ctrl+F` / `Alt+L`      | Go to `[Ctrl+F]` in-chat search / `[Alt+L]` Command log |
| `Enter` / `l`           | Open conversation |
| `Alt+N`                 | New conversation |
| `Alt+Y`                 | Yank (copy) label to clipboard |
| `Alt+E`                 | Mark as sEEn (read) |
| `Alt+R` / `F5`          | Refresh inbox |
| `Alt+U`                 | Toggle mute (local only — hides the unread badge; not synced) |
| `Alt+S`                 | Toggle ★ favorite (local only — not synced to Keybase) |
| `Alt+I`                 | Ignore conversation (hidden until next message) |
| `Alt+B`                 | Block conversation (hide for good) |
| `Alt+G`                 | Report conversation (flag to Keybase + hide) |
| `Alt+H`                 | Unhide — restore a blocked/reported chat by name |
| `Alt+K`                 | Channel browser (on a team) — join/open/leave, new/rename/delete |
| `Alt+T`                 | Teams |
| `Ctrl+G`                | Global search |
| `Ctrl+K`                | Quick switcher — jump to a conversation |
| `Shift+L`               | Logout (confirmation) |

In the **Chats** tree, `↑` / `↓` move; `Enter` / `→` / `l` open the selected
conversation (or expand a group) and `←` / `h` close the open chat (or
collapse a group); the header search fuzzy-filters it.

### Conversation

| Key | Action |
|---|---|
| typing                  | Extend draft |
| `Enter`                 | Send / save edit |
| `Alt+Enter`             | New line (multi-line message) |
| `@…` then `Tab`         | Mention autocomplete (`↑`/`↓` pick) |
| `↑` / `↓` · `PgUp` / `PgDn` | Scroll history |
| `←` / `→` · `Home` / `End`  | Compose cursor |
| `Ctrl+F`                | Search this conversation (jump to a match) |
| mouse                   | Click a message to select, scroll to page history |
| `Alt+V`                 | Select mode — act on a message (edit/delete/react/pin/reply) |
| `Alt+U`                 | Unpin channel |
| `Alt+R`                 | Resend failed message |
| `Alt+A`                 | Attach a file (opens the file picker) |
| `F5` / `Ctrl+R`         | Refresh |
| `Ctrl+Y`                | Copy label |
| `Esc`                   | Cancel / back |

In **Select** mode: `o` / `l` open / copy the first link in the message · `↑` / `↓` move the cursor · `Shift+↑/↓` shade a range ·
`Space` mark / unmark (multi-select) · `y` copy selection (author + time +
body) · `c` copy bodies only · `e` / `d` / `+` / `p` edit / delete / react /
pin · `r` / `s` reply / download · `i` / `Enter` / `Esc` return to Compose.

The **command log** (Tab to focus it) has the same visual multi-select:
`↑/↓` move · `Shift+↑/↓` range · `Space` mark · `y` copy full line(s) · `c`
copy detail only · `Esc` clear / leave.

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
    * `Alt+S` toggles a local ★.
    * `Alt+U` toggles a local mute, which only suppresses secretbase's own
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

[theme]
name         = "nord"              # bundled preset: nord (default),
                                    # catppuccin-mocha, dracula, catppuccin-latte (light)
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
frame.

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
│   ├── filter.rs           # StatusFilter (All/Unread)
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
    ├── flows/              # auth, chat, copy
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
