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
* Sidebar filters (All / Unread / DMs / Teams / Archived).
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
| `Tab` / `Shift+Tab`     | Cycle focus (search · filters · list · log) |
| `/`                     | Focus search |
| `Enter` / `l`           | Open conversation |
| `Alt+N`                 | New conversation |
| `Alt+C`                 | Copy label to clipboard |
| `Alt+M`                 | Mark as read |
| `Alt+R` / `F5`          | Refresh inbox |
| `Alt+U` / `Alt+O`       | Mute / unmute |
| `Alt+I`                 | Ignore conversation (hide from inbox) |
| `Alt+T`                 | Teams |
| `Ctrl+G`                | Global search |
| `Shift+L`               | Logout (confirmation) |

In the **Filters** panel, `↑` / `↓` change the active filter.

### Conversation

| Key | Action |
|---|---|
| typing                  | Extend draft |
| `Enter`                 | Send / save edit |
| `↑` / `↓` · `PgUp` / `PgDn` | Scroll history |
| `←` / `→` · `Home` / `End`  | Compose cursor |
| `Alt+V`                 | Select mode |
| `Alt+E` / `Alt+D`       | Edit / delete last own message |
| `Alt+J` / `Alt+P`       | React / pin last message |
| `Alt+U`                 | Unpin channel |
| `F5` / `Ctrl+R`         | Refresh |
| `Ctrl+Y`                | Copy label |
| `Esc`                   | Cancel / back |

In **Select** mode: `↑` / `↓` move the cursor · `e` / `d` / `:` / `p`
edit / delete / react / pin · `r` / `s` reply / download ·
`i` / `Enter` / `Esc` return to Compose.

### Global

| Key | Action |
|---|---|
| `F1`                    | Toggle help |
| `Ctrl+C`                | Quit |

## Configuration

`~/.config/secretbase/config.toml`:

```toml
clipboard_clear_secs = 30          # clipboard auto-clear (0 disables)
list_inbox_timeout_secs = 30       # wall-clock budget for `keybase chat api list`
download_timeout_secs = 300        # wall-clock budget for attachment downloads
auto_mark_read = true              # mark conversations as read when opened

[theme]
accent       = "#cba6f7"
inactive     = "#6c7086"
selected_bg  = "#313244"
# … see src/tui/theme.rs for the full key list
```

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
│   ├── filter.rs           # ConversationFilter (All/Unread/DMs/Teams/Archived)
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

To be decided.
