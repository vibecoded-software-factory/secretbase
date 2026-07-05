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
| `keybase login [username]` | Authenticate an existing account (device provisioning) |
| `keybase logout` | Log out the current device |
| `keybase status` | Show session / device / login status (`--json` for machine-readable) |
| `keybase help` | Access help documentation |

### `keybase login` — two paths (what the CLI actually supports)

Login is **device provisioning**, not a username+password call. Verified
against `keybase/client` `go/client/cmd_login.go` + `ui.go`:

- **Non-interactive (paper key)** — the *only* scriptable path, and only on a
  device **never** provisioned for the account:
  `keybase login --devicename <device> <username>` with the **paper key on
  stdin** (or `KEYBASE_PAPERKEY` / `KEYBASE_DEVICENAME` env). The CLI reads it
  via `PromptPasswordMaybeScripted`, which falls back to stdin when stdin is
  not a TTY. secretbase drives this from the Login form
  (`KeybasePort::login_paperkey`, one-shot spawn, paper key held in a
  `Zeroizing` buffer). A device that was merely logged out is **still
  provisioned** and rejects this with *"already provisioned this device"*.
- **Interactive (passphrase)** — for an already-provisioned device (e.g. after
  logout), keybase asks for the passphrase through `SecretUI` →
  pinentry/terminal (`SecretEntry.Get`), which **never reads stdin**, so it
  can't be scripted. secretbase therefore **cedes the terminal** to interactive
  `keybase login <username>` (suspends the TUI, runs it, restores) — the "Log
  in in terminal" button.

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

The chat / team / wallet / kvstore / contact-settings **stdin/stdout JSON
API** is what this app wraps. This section is the **complete** reference for
every `keybase … api` family (not only the parts secretbase wires today), so
a future feature can look up the exact method + options here first. Each
family's doc is verbatim from its `*_api_doc.go` in `keybase/client`
(`go/client/{chat,team,wallet,kvstore,contact_settings}_api_doc.go`), fetched
via e.g. `gh api repos/keybase/client/contents/go/client/chat_api_doc.go -H
"Accept: application/vnd.github.raw"`. Where a method/subcommand is **not**
wired yet, it's flagged as a future-feature candidate.

### `keybase chat api` methods (verbatim from `chat_api_doc.go`)

`list` · `read` · `get` · `send` · `delete` (one message by `message_id`)
· `edit` · `reaction` · `attach` · `download` · `mark` · `setstatus` ·
`searchinbox` · `searchregexp` · `newconv` · `listconvsonname` · `join` ·
`leave` · `addtochannel` · `removefromchannel` · `loadflip` ·
`getunfurlsettings` · `setunfurlsettings` · `advertisecommands` ·
`clearcommands` · `listcommands` · `pin` · `unpin` (**note:** `read` output
strips the pin payload — `chat_svc_handler.go::convertMsgBody` maps every
`MessageBody` field except `Pin__`, so a pin message arrives as
`{"type":"pin"}` with **no target id**; and `unpin` posts a `DELETE`
superseding the pin message, so a cleared pin simply disappears from the
history. Presence of a `pin` message = an active pin; the *target* is only
knowable for pins set by this session) · `getdeviceinfo` ·
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
- `listconvsonname` `{"topic_type":"CHAT","members_type":"team","name":TEAM}` —
  lists **every channel of a team** (joined or not), same `result.conversations[]`
  (`ConvSummary`) shape as `list` (verified: `ListConvsOnNameV1` → `ExportToSummary`),
  so the tolerant parser is reused. `member_status == Active` = you're a member.
  secretbase drives it from the **channel browser** (`c`); `join`/`leave`
  `{"channel":{name:TEAM,members_type:"team",topic_name:CH}}` join/leave a channel,
  and **create** reuses `newconv` on a team channel (`{name:TEAM,members_type:
  "team",topic_name:NEW}`) — there's no dedicated `create-channel` API method
  (that's a CLI subcommand), and `newconv` creates the channel just the same.
- **`keybase chat rename-channel <team> <old> <new>`** / **`delete-channel
  <team> <channel>`** — CLI subcommands (no API method), so one-shot spawns.
  `delete-channel` is **non-interactive** (verified: resolves non-interactively +
  `DeleteConversationLocal`, no prompt) but destructive → secretbase confirms
  inline first. Both are driven from the channel browser (`r` / `d`).
- **`keybase chat default-channels <team> [--channel C]…`** — CLI subcommand
  (no API method), plain-text output (not JSON). No `--channel` = **get**;
  repeated `--channel` = **replace** the default set, then print it. The default
  channels are the ones new team members auto-join; `#general` is always default
  and printed first (not part of the settable set). secretbase gets them on
  browser load (badges `★ default`) and toggles with `t` (recomputes + sets the
  whole set). ⚠️ The CLI **can't clear the set to empty** (no `--channel` is a
  get), so removing the last default is refused. Verified vs source
  (`SetDefaultTeamChannelsLocal` / `GetDefaultTeamChannelsLocal`).
- `listmembers` `{"channel":…}` (or `{"conversation_id":…}`) — the members of a
  conversation / team channel. Result is `ChatMembersDetails`: six role buckets
  (`owners`/`admins`/`writers`/`readers`/`bots`/`restrictedBots`), each a list of
  `{uid,username,fullName}` (verified: `ListMembersV1` → `TeamToChatMembersDetails`).
  secretbase flattens these to `domain::ChatMember` for the **Members** view (`m`
  in the channel browser, `Alt+P` on an open team channel).
- `addtochannel` / `removefromchannel` `{"channel":…,"usernames":[…]}` — add /
  remove members of a team channel. Driven from the Members view (`a` / `x`).
- `searchregexp` `{"channel":…,"query":…,"is_regex":false,"max_hits":N}` —
  server-side search **within one conversation** (full history). Result shape
  is `result.hits[]`, each `{hitMessage.valid{messageID, bodySummary,
  senderUsername, …}}` (flat — already scoped to the channel, no convID
  wrapper). secretbase drives it from the conversation search box (`Ctrl+F`)
  and jumps to the picked match.
- `get` `{"channel":…,"message_ids":[314,315,342]}` — fetch specific messages
  by id (vs `read`'s paginated window). Result is the same `Thread` shape as
  `read` (`result.messages[].msg` — verified: `GetV1` → `formatMessages`).
  secretbase uses it to resolve a **pinned** message that is older than the
  loaded window (the 📌 header's body snippet), on the background lane.
- `read` supports `pagination` (`{num,next,previous}`), `peek` (don't mark
  read), `unread_only`.
- `list` supports `topic_type` (`CHAT`/`DEV`). **There is no documented
  option to include ignored/blocked conversations** — the inbox `list`
  excludes them, so an ignored conv cannot be reached from `list` alone.
- `setstatus` `{"channel":…,"status":…}` — status enum
  `chat1.ConversationStatus` (verified vs `go/protocol/chat1/common.go`):
  `unfiled`=0 · `favorite`=1 · `ignored`=2 · `blocked`=3 · `muted`=4 ·
  `reported`=5. secretbase wires only the **observable** ones: `ignored`
  (ignore), `blocked` (block), `reported` (report) — their effect is the conv
  leaving the inbox. **Undo is always `unfiled`** — confirmed by
  `cmd_chat_hide.go` (`--block`→`BLOCKED`, `--unhide`→`UNFILED`).
  **`favorite` and `muted` are intentionally NOT wired to Keybase**: the status
  can't be read back (see below), so a synced state would drift. secretbase
  keeps **local-only** ★ favourite (`s`) and mute (`u`) instead —
  persisted config keys (`favorites`/`muted`), no `setstatus` call. (Local mute
  only hides secretbase's unread indicators; it can't silence Keybase's push
  notifications, which is what the server-side `muted` does.) ⚠️
  `blocked`/`reported`/`ignored` conversations are
  **excluded from `list`** (the chat `list` JSON has no status filter —
  verified: `listOptionsV1` exposes only `topic_type`), so a blocked conv
  can't be reached from the tree; secretbase restores it by **name** via the
  Unhide popup (`setstatus unfiled` on the rebuilt channel). The list item
  (`ConvSummary`) carries **no `status` field** (the service RPC
  `ConversationInfoLocal.Status` / `IsMuted` has it, but the CLI's JSON
  projection drops it), so favourite and mute can't be read back — hence both
  are handled **locally** instead (see above).
- **No bulk "delete conversation/history" method exists in the api** —
  `delete` is per-message. Bulk delete is a CLI subcommand (below).

#### chat-api methods NOT yet wired in secretbase (available for future features)

The list above is the **complete** `chat_api_doc.go` surface; these are the
methods secretbase does not call yet — each is a candidate for a future flow
(build the request in `codec.rs`, add a `WorkerRequest`/`InFlight` pair):

- **search filters** (both `searchinbox` and `searchregexp`): `sent_by`,
  `sent_to`, `sent_after`/`sent_before` (dates, e.g. `"09/10/2017"`), `max_hits`;
  `searchinbox` also takes a free `query`, `searchregexp` an `is_regex` bool.
  secretbase currently passes only `query`/`max_hits` (+ `is_regex:false`).
- `loadflip` `{"conversation_id","flip_conversation_id","msg_id","game_id"}` —
  resolves a `/flip` game's result.
- `getunfurlsettings` / `setunfurlsettings`
  `{"mode":"always"|"never"|"whitelisted","whitelist":["example.com"]}` — the
  link-preview (unfurl) policy for sent links.
- `advertisecommands` / `clearcommands` / `listcommands` — bot command
  advertisement. `advertisecommands` `{"alias","advertisements":[{"type":…,
  "commands":[{"name","description"}]}]}` with `type` ∈ `public` ·
  `teammembers` (needs `team_name`) · `teamconvs` (needs `team_name`) · `conv`
  (needs `conv_id`); `clearcommands` takes an optional `{"filter":{"type",…}}`;
  `listcommands` takes a `{"channel":…}`.
- `getdeviceinfo` `{"username":…}` — a user's device info by username.
- `getresetconvmembers` / `addresetconvmember` `{"username","conversation_id"}`
  — list / re-add members who reset their account in your conversations.
- `emojiadd` `{"channel","alias","filename"}` (upload a custom emoji),
  `emojiaddalias` `{"channel","new_alias","existing_alias"}`,
  `emojiremove` `{"channel","alias"}` — secretbase reads `emojilist` but does
  not add/alias/remove yet.

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
- **Complete `keybase chat` subcommand list** (verified from the
  `go/client/cmd_chat_*.go` files present in the repo). Most map 1:1 to an
  api-mode method above; the ones with **no** api equivalent are the only
  reason to shell a subcommand. Grouped by area:
  - *messaging:* `send` · `read` · `list` · `list-unread` · `mark-as-read` ·
    `search-inbox` · `search-regexp` · `search-profile` · `fwdmsg` (forward a
    message to another conversation — **no api method**, subcommand only) ·
    `conv-info` · `upload` · `download`.
  - *channels & membership:* `createchannel` · `listchannels` · `joinchannel` ·
    `leavechannel` · `renamechannel` · `delete-channel` · `default-channels` ·
    `addtochannel` · `removefromchannel` · `readd-member` (re-add a reset
    member) · `min-writer-role` (set the minimum role that can post — **no api
    method**).
  - *status & retention:* `hide` (ignore/`--block`/`--unhide`) · `mute` ·
    `report` · `retention` (+ `retention-dev`; set the conversation/team
    message-retention policy — **no api method**) · `delete-history` (+
    `delete-history-dev`).
  - *archive (async export job):* `archive` · `archive-list` ·
    `archive-delete` · `archive-pause` · `archive-resume`.
  - *bots:* `add-bot-member` · `edit-bot-member` · `remove-bot-member` ·
    `bot-member-settings` · `featured-bots` · `search-bots` — bot management
    (**no api-mode equivalents**; only `advertise/clear/listcommands` are api).
  - *emoji:* `emojiadd` · `emojiaddalias` · `emojiremove` · `emojilist` (these
    *do* have api methods).
  - *notifications & misc:* `notification-settings` (per-conversation
    notification policy — **no api method**) · `kbfs-upgrade`.
  Everything secretbase currently drives is api-mode except `rename-channel`,
  `delete-channel` and `default-channels` (documented above). The rest are
  future-feature candidates — `retention`, `notification-settings`, `fwdmsg`,
  `min-writer-role`, `conv-info`, `archive`, bot management, emoji add/remove.

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

### `keybase team api` methods (verbatim from `team_api_doc.go`)

secretbase wires only `list-user-memberships` (via `list_self_memberships`,
which queries it with the caller's own username — see the port doc — because
the api's `list-self-memberships` maps to `TeamListTeammates` and returns one
row *per teammate* across every team) and `create-team` / `leave-team`. The
**full** method set:

- `list-self-memberships` — every team you're in (one row per teammate; noisy,
  so secretbase uses `list-user-memberships` instead).
- `list-team-memberships` `{"team":"phoenix"}` — the members of one team.
- `list-user-memberships` `{"username":"cleo"}` — one row per team a user is in
  (what secretbase actually calls, with its own username → one row per team).
- `create-team` `{"team":"phoenix"}` — also creates a **subteam** when the name
  is dotted (`"phoenix.bots"`).
- `add-members` `{"team":…,"emails":[{"email","role"}],"usernames":[{"username",
  "role"}]}` — add members by username/email/social-proof with a role.
- `edit-member` `{"team":…,"username":…,"role":…}` — change a member's role.
- `remove-member` `{"team":…,"username":…}`.
- `rename-subteam` `{"team":"phoenix.bots","new-team-name":"phoenix.humans"}`.
- `leave-team` `{"team":…,"permanent":true}` — secretbase's `leave_team` wraps
  this.
- `list-requests` `{"team":"phoenix"}` — pending access requests to a team.

The team CLI also has non-api subcommands (verified from `cmd_team_*.go`):
`accept-invite` · `add-member` · `add-members-bulk` · `bot-settings` ·
`create` · `delete` · `edit-member` · `ftl` · `generate-invitelink` ·
`generate-seitan` · `ignore-request` · `leave` · `list-memberships` ·
`list-requests` · `profile-load` · `remove-member` · `rename` ·
`request-access` · `rotate-key` · `search` · `settings` · `show-tree`.

### `keybase wallet api` methods (verbatim from `wallet_api_doc.go`)

Stellar wallet JSON API — **not wired** in secretbase (chat only shows the
pre-rendered `SendPayment` / `RequestPayment` message text). Documented for
completeness:

- `balances` — balances across all your accounts.
- `history` `{"account-id":…}` — payment history for an account.
- `details` `{"txid":…}` — one transaction's details.
- `lookup` `{"name":"patrick"}` — a user's primary Stellar account id.
- `get-inflation` / `set-inflation` `{"account-id":…,"destination":…}` —
  inflation destination (`"lumenaut"` / an account id / `"self"`).
- `send` `{"recipient":…,"amount":…,"currency":"USD","message":…}` — **no
  confirmation**, be careful.
- `find-payment-path` / `send-path-payment`
  `{"recipient","amount","source-asset","destination-asset","source-max-amount"}`
  — cross-asset path payments.
- `cancel` `{"txid":…}` — cancel an unclaimed payment (recipient has no wallet
  yet); the XLM returns to your account.
- `setup-wallet` — initialise the wallet for an account.

### `keybase kvstore api` methods (verbatim from `kvstore_api_doc.go`)

Fast, encrypted key-value storage (per-team-key encrypted `entryValue`;
`namespace`/`entryKey` are server-visible). **Not wired** in secretbase.
Defaults to your implicit self-team when `team` is omitted.

- `put` `{"team":…,"namespace":…,"entryKey":…,"entryValue":…,"revision":N?}` —
  a non-zero `revision` gives optimistic-concurrency (e.g. `1` errors if the
  entry already exists).
- `get` `{"team":…,"namespace":…,"entryKey":…}` — latest revision (a
  non-existent entry has revision `0`).
- `list` `{"team":…,"namespace":…?}` — namespaces, or entryKeys in a namespace.
- `del` `{"team":…,"namespace":…,"entryKey":…,"revision":N?}` — delete an entry.

### `keybase contact-settings api` methods (verbatim from `contact_settings_api_doc.go`)

Who may message you. **Not wired** in secretbase.

- `get` — current contact settings.
- `set` `{"settings":{"enabled":bool,"allow_followee_degrees":1|2,
  "allow_good_teams":bool,"teams":[{"team_name","enabled"}]}}` — restrict DMs to
  people you follow (degree 1) / follow-of-follows (degree 2) and/or teammates.

In-repo, every API call is built in `adapters/keybase_cli/codec.rs`, run
over the persistent stream in `adapters/keybase_cli/session.rs` (one-shot
fallback in there too), and parsed in `adapters/keybase_cli/{mod,json}.rs`.
`status` / `logout` are not API-mode and run as one-shot `keybase`
invocations.
