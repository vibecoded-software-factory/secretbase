# CLI.md — Keybase command-line reference

Mirror of the Keybase CLI docs at <https://book.keybase.io/docs/cli>
(fetched 2026-06-26). **Read this before adding or editing any
functionality** (see `CLAUDE.md`): secretbase is a wrapper over this
binary, so every feature maps to a `keybase` command. When in doubt
about exact syntax/flags, confirm against `keybase help <cmd>` on a real
install — the built-in help is the authoritative source.

> The website page (`/docs/cli`) covers only account / identity / device /
> crypto / Tor — it has **no chat content**, and the chat doc pages 403 the
> fetcher. **For chat/team, the authoritative source is the keybase
> `client` GitHub repo** (`go/client/chat_api_doc.go` + `cmd_chat_*.go`),
> read via `gh api`. The verified chat reference is in **Related** at the
> bottom.

---

## Overview

The Keybase CLI provides cryptographic operations and identity
verification. Consult the built-in help for any command:

```sh
keybase help
keybase help follow
keybase help pgp
keybase help prove
```

## Account management

| Command | Action |
|---|---|
| `keybase version` | Display the version number |
| `keybase signup` | Create a new account |
| `keybase login` | Authenticate an existing account (device provisioning) |
| `keybase logout` | Log out the current device |
| `keybase status` | Show session / device / login status (`--json` for machine-readable) |
| `keybase help` | Access help documentation |

## Identity proofs

```sh
keybase prove twitter
keybase prove github
keybase prove reddit
keybase prove facebook
keybase prove hackernews
keybase prove https you.com   # prove a website via HTTPS
keybase prove http  you.com   # prove a website without a certificate
keybase prove dns   you.com   # prove ownership via a DNS entry
```

## User lookup & following

```sh
keybase id max                # verify a user's identity
keybase id maxtaco@twitter    # look up by social proof
keybase follow max            # publicly track a user's identity
keybase follow maxtaco@reddit # follow by social proof
```

## Device & key management

```sh
keybase device list           # list all devices and paper keys
keybase device remove [ID]    # revoke a device
keybase device add            # provision a new device
keybase paperkey              # generate a new paper key
```

Every device gets a unique key; paper keys function identically to device
keys.

## Cryptographic commands

Flag conventions shared across the crypto verbs:

| Flag | Meaning |
|---|---|
| `-m` | message argument (inline) |
| `-i` | input file |
| `-o` | output file |
| `-b` | binary output format |

### Encryption (Keybase-native, for Keybase users)

```sh
keybase encrypt max -m "message"                       # encrypt a message
echo "secret" | keybase encrypt max                    # pipe input
echo "secret" | keybase encrypt maxtaco@twitter        # via social proof
keybase encrypt max -i ~/file.ext -o ~/file.encrypted  # encrypt a file
```

### Decryption

```sh
keybase decrypt -i file.encrypted -o file   # decrypt a file
keybase decrypt -i encrypted.txt            # decrypt to stdout
cat encrypted.txt | keybase decrypt         # stream decryption
```

### Signing

```sh
keybase sign -m "statement"                      # sign a message
keybase sign -i file.exe -b -o file.exe.signed   # sign a binary file
```

### Verification

```sh
cat statement.txt | keybase verify       # verify a signed statement
keybase verify -i file.signed -o file    # extract + verify
```

## PGP operations

### PGP encryption

```sh
keybase pgp encrypt chris -m "secret"               # encrypt with a PGP key
keybase pgp encrypt maxtaco@twitter -m "secret"     # via social identity
keybase pgp encrypt chris -s -m "secret"            # encrypt AND sign
keybase pgp encrypt chris -i file.txt               # encrypt file → .asc
keybase pgp encrypt chris -i file.txt -o output.asc # custom output
echo 'secret' | keybase pgp encrypt chris           # stream encryption
```

### PGP decryption

```sh
keybase pgp decrypt -i file.asc                 # decrypt to stdout
keybase pgp decrypt -i file.asc -o file.txt     # decrypt to a file
cat file.asc | keybase pgp decrypt              # stream decryption
```

### PGP signing

```sh
keybase pgp sign -m "Hello"                   # sign a message
keybase pgp sign --clearsign -m "Hello"       # sign with visible content
keybase pgp sign -i file.txt --detached       # separate signature
keybase pgp sign -i file.txt                  # embedded signature
echo "text" | keybase pgp sign                # stream signing
```

### PGP verification

```sh
keybase pgp verify -i file.asc               # verify a self-signed file
keybase pgp verify -d file.asc -i file.txt   # verify with detached signature
cat file.asc | keybase pgp verify            # stream verification
```

## Bitcoin

```sh
keybase btc 1p90X3byTONYhortonETC   # publish a bitcoin address to your profile
```

## Assertions (scripting)

Assertions allow conditional encryption based on proof verification — the
operation only proceeds if **all** specified proofs pass:

```sh
cat backup.sql | keybase pgp encrypt -o enc_backup.asc \
  maria_2354@twitter+maria_booyeah@github+maria@keybase
```

---

## Tor support

> The Keybase **GUI does not support Tor**. For full application
> anonymity, run inside a [Tails VM](https://tails.boum.org). Tor support
> is in **alpha and unaudited** — the CLI warns: *"Tor support is in
> alpha; please be careful and report any issues."*

### Prerequisites

A local Tor SOCKS proxy is required (see the Tor project docs for setup).

### Enabling Tor mode

Temporary (for one running service):

```sh
keybase ctl stop
keybase --tor-mode=leaky|strict service
```

All commands in other terminals route through Tor while the service runs.

Permanent (persisted in config; restart the service, GUI not running):

```sh
keybase config set tor.mode leaky
keybase config set tor.mode strict
```

### Modes

- **Leaky** — all traffic tunnels through Tor; the server still sees your
  login credentials. Hides your IP from eavesdroppers; does **not**
  protect against a server breach.
- **Strict** — withholds user-identifying info from the server; profile
  sync disabled; prevents correlation of lookups. Some commands are
  unavailable (e.g. a fresh login). Example warning: *"Can't write
  tracking statement to server in strict Tor mode."*

### Tor hidden address

The CLI uses Keybase's onion service internally by default, avoiding exit
nodes:

```
http://keybase5wmilwokqirssclfnsqrjdsi7jdir5wy7y7iu3tanwmtp6oid.onion
```

### Limitations

- DNS and HTTP proofs are unreliable over Tor (relay nodes can fabricate
  responses).
- Strict mode is currently broken (fix in progress).

---

## Related — the chat JSON API secretbase drives (verified from source)

The chat/team/wallet **stdin/stdout JSON API** is what this app wraps.
Source of truth: `keybase/client` → `go/client/chat_api_doc.go`
(fetch via `gh api repos/keybase/client/contents/go/client/chat_api_doc.go
-H "Accept: application/vnd.github.raw"`).

### `keybase chat api` methods (verbatim from `chat_api_doc.go`)

`list` · `read` · `get` · `send` · `delete` (one message by `message_id`)
· `edit` · `reaction` · `attach` · `download` · `mark` · `setstatus` ·
`searchinbox` · `searchregexp` · `newconv` · `listconvsonname` · `join` ·
`leave` · `addtochannel` · `removefromchannel` · `loadflip` ·
`getunfurlsettings` · `setunfurlsettings` · `advertisecommands` ·
`clearcommands` · `listcommands` · `pin` · `unpin` · `getdeviceinfo` ·
`getresetconvmembers` · `addresetconvmember` · `listmembers` ·
`emojiadd` · `emojiaddalias` · `emojiremove` · `emojilist`.

- `attach` `{"channel":…,"filename":…,"title":…}` — uploads a local file
  (absolute `filename`); `title` is an optional caption (omit ⇒ keybase
  defaults it to the filename). `download` takes `message_id` + `output`
  path. secretbase drives `attach` from the file picker (Alt+A) and
  `download` from its directory-pick mode (select-mode `s`, destination
  defaulting to the OS Downloads folder); both run on the longer
  `download_timeout` budget.
- `emojilist` — result shape is `result.emojis.emojis[]` (groups), each
  group `{name, emojis:[{alias, remoteSource{…}}]}`. **In practice it
  returns only the team's *custom* emojis, not the stock unicode set**, so
  secretbase merges it with a bundled curated standard set
  (`domain::emoji::standard`) to power the reaction picker. Fetched once,
  warmed at boot, cached.
- `searchinbox` — result shape is `result.results.hits[]`, each
  `{convID, convName, hits:[{hitMessage.valid{messageID, senderUsername,
  bodySummary, …}}]}`. **Gotcha: `convID` here is standard-base64, whereas
  `list` returns the conversation id as lowercase hex** — secretbase
  decodes it to hex when parsing so a search hit re-keys into the cached
  inbox (open-from-search).
- `read` supports `pagination` (`{num,next,previous}`), `peek` (don't mark
  read), `unread_only`.
- `list` supports `topic_type` (`CHAT`/`DEV`). **There is no documented
  option to include ignored/blocked conversations** — the inbox `list`
  excludes them, so an ignored conv cannot be reached from `list` alone.
- `setstatus` `{"channel":…,"status":…}` — status enum
  `chat1.ConversationStatus`: `unfiled` · `favorite` · `ignored` ·
  `blocked` · `muted` · `reported`. (We use `muted`/`unfiled` for
  mute/unmute and `ignored`/`unfiled` for ignore/un-ignore.)
- **No bulk "delete conversation/history" method exists in the api** —
  `delete` is per-message. Bulk delete is a CLI subcommand (below).

### `keybase chat` CLI subcommands (verified from `cmd_chat_*.go`)

These are NOT api-mode (run as one-shot `keybase chat <sub>` spawns):

- **`hide [conversation]`** (`cmd_chat_hide.go`) — *"Hide or block a
  conversation."* Default → status `IGNORED`; `--block` → `BLOCKED`;
  `--unhide` → `UNFILED`. (Same effect as api `setstatus`.) Non-interactive.
- **`delete-history [conversation] --age=<2h|3d|1w>`**
  (`cmd_chat_delete_history.go`) — permanently deletes message history for
  everyone. **Prompts interactively** ("Permanently delete ALL chat history
  of […]? Hit Enter to confirm, or Ctrl-C to cancel"); **no force flag**.
  `--age` limits to messages older than the interval. ⚠️ We spawn with
  stdin nulled, so the prompt must be handled (pipe newline, or use a TTY)
  — verify before wiring.
- **`archive [conversation] [-o <dir>] [--compress]`**
  (`cmd_chat_archive.go`) — **exports** all messages of the conversation(s)
  to a file/backup on disk (async job: `archive-list` / `archive-delete` /
  `archive-pause` / `archive-resume`). It does **NOT** remove the conv from
  the inbox or change its status.
- Other relevant subcommands present: `createchannel`, `delete-channel`,
  `addtochannel`, `conv-info`, `mute`, `report`, `download`, `upload`,
  emoji*, `default-channels`.

### `keybase chat api-listen` — push stream

`keybase chat api-listen` prints chat notifications as one JSON object per
line, for as long as it runs (no stdin protocol — it only emits). secretbase
spawns it once at launch and drives the real-time inbox + open-conversation
updates from it (see `adapters/keybase_cli/listen.rs`).

Flags secretbase passes (verified against `go/client/cmd_chat_api_listen.go`):

- `--convs` — also emit a notification when a new conversation is
  created/joined.
- `--hide-exploding` — skip ephemeral (exploding) messages.

Event shapes secretbase parses (verified against
`go/client/chat_api_listen_display.go`):

- **incoming message** — `{"type":"chat","source":"remote","msg":{<MsgSummary>}}`
  (the `msg` is the same shape `keybase chat api read` returns; own/local
  messages are skipped unless `--local` is passed, which we don't).
- **new conversation** — `{"type":"chat_conv","conv":{<ConvSummary>}}`.

Only `NewChatActivity` (messages, new conversations) and joined-conversation
notifications are printed; **typing, read-state, edits-as-deltas, and the
other `NotifyChat` callbacks return `nil`** in the source, so they are *not*
available over the CLI — only the lower-level service RPC exposes them.

### Other JSON APIs
- `keybase team api` — team JSON API (`list-self-memberships`,
  `create-team`, `add-members`, `list-team-memberships`, …). Verify against
  `go/client/cmd_team_api.go` / the team api doc in source.
- `keybase wallet api` — Stellar wallet JSON API.

In-repo, every API call is built in `adapters/keybase_cli/codec.rs`, run
over the persistent stream in `adapters/keybase_cli/session.rs` (one-shot
fallback in there too), and parsed in `adapters/keybase_cli/{mod,json}.rs`.
`status` / `logout` are not API-mode and run as one-shot `keybase`
invocations.
