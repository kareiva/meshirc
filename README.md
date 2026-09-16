# MeshIRC

An irssi-style terminal client for [MeshCore](https://meshcore.co.uk) companion radios
connected over USB serial.

```
10:51 -!- connected: LY2EN C99 (Heltec V3, fw v1.17.1)            │ 331 contacts
10:51 -!- channel slot 0: Public                                  │● Mordle          now
10:51 -!- channel slot 1: #lietuva                                │▲ LT-VM Pavilnys   2m
10:52 <Someone> labas                                             │▲ VA-Druzai-2     15m
10:52 <LY2EN C99> hi ✓                                            │■ LT-KA OBS        1h
 10:52 [1:status] [2:Public] [3:#lietuva]                    LY2EN C99 bat 47% (3.44V)
[#lietuva] _
```

## Build & run

```sh
cargo build --release
./target/release/meshirc                 # uses /dev/ttyACM0 @ 115200 (COM3 on Windows)
./target/release/meshirc -p /dev/ttyUSB0
```

Prebuilt binaries for Linux (x86_64, aarch64, static musl) and Windows (x86_64) are
attached to each [GitHub release](https://github.com/kareiva/meshirc/releases); pushing a
`v*` tag builds them (`.github/workflows/release.yml`).

You need read/write access to the serial device — on most distributions add yourself
to the `dialout` (Debian/Fedora) or `uucp` (Arch) group and log in again.

Optional config at `~/.config/meshirc/config.toml`:

```toml
port = "/dev/ttyACM0"
baud = 115200
history_lines = 200
save_private = true     # log private messages and reopen private windows on start
auto_join = ["#lietuva"]
```

Chat history lives in `~/.local/share/meshirc/logs/`, one plain-text file per channel or
private window; the last `history_lines` lines are shown when a window opens. Channels and
private windows from earlier sessions are reopened automatically. `save_private = false`
(or `--save-private false`) keeps private messages off disk entirely.

## Commands

| Command | Description |
|---|---|
| `/join #name` | Join a hashtag channel (programs a free channel slot on the radio) |
| `/join <name> <key>` | Join a private channel with a shared 128-bit key (32 hex chars, as shown in the MeshCore app) |
| `/part [#name]` | Leave a channel and free its slot |
| `/msg <who> <text>` | Private message (name, unique name prefix, or pubkey hex prefix) |
| `/query <who>` | Open a private window |
| `/whois [who]` | Cached contact info (type, pubkey prefix, last seen, location, path); in a private window `who` defaults to that node |
| `/status [who]` | Live status request over the mesh (battery, uptime, RSSI/SNR, packet counters); only repeaters and rooms answer |
| `/contacts` | Reload contacts from the radio |
| `/advert [flood]` | Send a self advertisement |
| `/nick <name>` | Rename the radio node |
| `/win N`, `/close` | Window management (0 = status window) |
| `/set [save_private on\|off]` | Show or change settings for this session; put `save_private = false` in the config to persist |
| `/wipe [target]` | Delete history: the current window, a named `<window>`, all `channels`, all `private` chats, or `all` — clears the screen and deletes the log files, no confirmation |
| `/help`, `/quit` | |

Keys: `Alt+0..9` (0 = status), `Ctrl+N`/`Ctrl+P` switch windows · `PgUp`/`PgDn` scroll ·
`Tab` completes contact and channel names · `Up`/`Down` input history · `Ctrl+C` quits ·
terminal paste (bracketed) inserts into the input line. Windows with unread activity are
shown in red on the bottom bar (bold red for new messages).

With an empty input line, `Tab` cycles focus: input → chat (`Up`/`Down` scroll, `Home`/`End`)
→ contacts (`Up`/`Down` select, `Enter` opens a private window) → input. `Shift+Tab` goes
back, `Esc` returns to the input, and typing a character always returns to the input.

Typing `@` followed by the start of a contact name pops up the matching contacts above the
input line (names may contain spaces, keep typing to narrow). `Tab` completes a single match
outright; with several, `Tab`/`Up`/`Down` move the selection and `Enter` inserts it. The
result is `@[Node Name] `. `Esc` dismisses the popup.

Every message you send gets a marker: grey `○` while waiting, green `●` once a repeater
has been heard rebroadcasting it (or `●●` when the recipient ACKed a DM), red `●` if
nothing was heard within 60 s (or the DM ACK timed out). Repeat detection is a
heuristic: the RX log is matched on packet type and size against recent sends, so it
can occasionally be fooled by a same-size message in a channel you don't have.

## Files

- `~/.local/share/meshirc/logs/<window>.log` — per-window chat logs (last 200 lines reloaded on open)
- `~/.local/share/meshirc/meshirc.log` — debug log (`RUST_LOG=meshcore_rs=debug` for wire-level tracing)

## Notes

- The contact sidebar lists every node the radio knows, newest advert first. MeshCore
  has no channel membership, so there is no per-channel user list.
- Hashtag channel keys are `sha256("#name")[0..16]`; anyone who knows the name can read it.
- `cargo run --example probe` dumps radio info, slots, contacts and raw events without the TUI.
