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

**v0.1.0 — MVP**

End-to-end feature working: **chat inbox list** (read-only).

* `keybase status --json` on boot to detect the session.
* `keybase chat api {"method":"list"}` to load conversations.
* Discord-style unified two-pane Home: a collapsible **conversation tree**
  (Direct messages + a group per team; unread in bold) on the left, the open
  **chat** on the right; the header search fuzzy-filters the tree.
* Local fuzzy search (channel name, topic name, creator).
* Mark conversation as read (`m`).
* Copy conversation label to clipboard (`y`).
* Logout flow with confirmation.

The rest of Chat + Teams is **scaffolded but not yet wired** — see
[`ROADMAP.md`](ROADMAP.md) for the planned next steps.

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
| `Alt+C` / `Alt+M`       | Go to `[Alt+C]` Chats / `[Alt+M]` Chat (works mid-compose) |
| `Ctrl+F` / `Alt+L`      | Go to `[Ctrl+F]` in-chat search / `[Alt+L]` Command log |
| `Enter` / `l`           | Open conversation |
| `Alt+N`                 | New conversation |
| `Alt+Y`                 | Yank (copy) label to clipboard |
| `Alt+E`                 | Mark as sEEn (read) |
| `Alt+R` / `F5`          | Refresh inbox |
| `Alt+U` / `Alt+O`       | Mute / unmute |
| `Alt+I`                 | Ignore conversation (hide from inbox) |
| `Alt+T`                 | Teams |
| `Ctrl+G`                | Global search |
| `Ctrl+K`                | Quick switcher — jump to a conversation |
| `Shift+L`               | Logout (confirmation) |

In the **Chats** tree, `↑` / `↓` move and `Enter` folds a group or opens the
selected conversation in the right pane; the header search fuzzy-filters it.

### Conversation

| Key | Action |
|---|---|
| typing                  | Extend draft |
| `Enter`                 | Send / save edit |
| `Alt+Enter`             | New line (multi-line message) |
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

In **Select** mode: `↑` / `↓` move the cursor · `Shift+↑/↓` shade a range ·
`Space` mark / unmark (multi-select) · `y` copy selection (author + time +
body) · `c` copy bodies only · `e` / `d` / `:` / `p` edit / delete / react /
pin · `r` / `s` reply / download · `i` / `Enter` / `Esc` return to Compose.

The **command log** (Tab to focus it) has the same visual multi-select:
`↑/↓` move · `Shift+↑/↓` range · `Space` mark · `y` copy full line(s) · `c`
copy detail only · `Esc` clear / leave.

### Global

| Key | Action |
|---|---|
| `F1`                    | Toggle help |
| `F9`                    | Open Settings (Theme preset picker; more sections coming) |
| `Ctrl+C`                | Quit |

## Configuration

`~/.config/secretbase/config.toml`:

```toml
clipboard_clear_secs = 30          # clipboard auto-clear (0 disables)
list_inbox_timeout_secs = 30       # wall-clock budget for `keybase chat api list`
download_timeout_secs = 300        # wall-clock budget for attachment downloads
auto_mark_read = true              # mark conversations as read when opened

[theme]
name         = "nord"              # bundled preset: nord (default),
                                    # catppuccin-mocha, dracula, catppuccin-latte (light)
accent       = "#cba6f7"           # optional per-key overrides on top of the preset
inactive     = "#6c7086"
selected_bg  = "#313244"
# … see src/tui/theme.rs for the full key list
```

Pick a bundled palette with `name`, then override individual keys if you want.
Omitting `name` keeps the default (Nord with terminal-inherited text). You can
also switch presets live in-app from the **Settings** screen (`F9`).

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
│   ├── filter.rs           # StatusFilter (All/Unread) + InboxSource (DMs/team)
│   ├── search.rs           # Fuzzy ranking
│   └── validation.rs       # username validators
├── ports/                  # trait abstractions
│   ├── keybase.rs          # KeybasePort (status, list/read/send/…)
│   ├── clipboard.rs        # ClipboardPort
│   └── settings.rs         # SettingsPort + UserSettings
├── adapters/               # concrete impls — only layer doing I/O
│   ├── keybase_cli/        # shells out to `keybase` binary
│   │   ├── process.rs      # subprocess + timeout polling
│   │   ├── codec.rs        # JSON request builders
│   │   └── json.rs         # parsing helpers
│   ├── clipboard_system.rs # wl-copy / xclip / xsel / pbcopy
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
cargo test          # ~150 unit tests
cargo clippy --all-targets -- -D warnings
```

## License

MIT — see [`LICENSE`](LICENSE).
