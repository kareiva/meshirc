# MeshIRC

A text user interface client, like irssi, to connect to a MeshCore USB companion.

Written in Rust. Talks to the radio through the
[`meshcore-rs`](https://docs.rs/meshcore-rs/latest/meshcore_rs/) crate
(Rust port of `meshcore_py`); never hand-roll the companion serial protocol.

## Features

- Connecting to meshcore USB companion (serial port, default `/dev/ttyACM0`, 115200 baud)
- Joining channels IRC-style (`/join #name` programs a radio channel slot)
- List of contacts is like Discord — a persistent sidebar of *all* known
  contacts; we cannot determine whether a node is "in" a channel, so there is
  no per-channel nick list
- Private messages (`/msg`, `/query`)
- `/whois` — shows cached contact info; `/status` requests live status from the mesh

## Design decisions (settled)

| Topic | Decision |
|---|---|
| Channel slots full | **Refuse** `/join` with an error telling the user to `/part` first. No eviction. |
| History | **Plain-text logs per window**, irssi-style, in `~/.local/share/meshirc/logs/<window>.log`. Append on send/receive; on startup re-read the last N (default 200) lines into scrollback. |
| Layout | **irssi windows + Discord sidebar.** Numbered windows (0 = status, channels/queries from 1 in open order) switched with `Alt+0..9`, `Ctrl+N`/`Ctrl+P`, `/win N`. Persistent right pane lists every contact. Top bar: node name, serial port, radio settings, GPS/advertised location, battery. Bottom bar: clock, `N:name` for every window (names shortened with `…` when too narrow), `[act: 2,4!]` for unread, `[disconnected]`. |
| `/whois` / `/status` | `/whois` prints cached contact info only (pubkey shown as first 16 bytes). `/status` sends a binary status request and prints the response (or a timeout notice) when it arrives. Only repeaters/rooms answer. |
| Contact addressing | Names may contain spaces, so `/msg` / `/query` / `/whois` accept: exact name, unique case-insensitive name prefix, or public-key hex prefix (≥ 4 hex chars). Ambiguity → error listing the candidates. Tab-completion in the input line resolves names. |
| Config | CLI flags via `clap`, with optional `~/.config/meshirc/config.toml` for the same values (`port`, `baud`, `log_dir`, `history_lines`, `auto_join = ["#foo"]`). CLI overrides file. |

## MeshCore facts that shape the code

These come from the MeshCore companion protocol and from reading `meshcore-rs` source:

- **Channels are radio slots.** The companion holds a fixed number of channel
  slots (`DeviceInfoData::max_channels`; 40 on fw v1.17 Heltec V3, 8 on older
  builds). The radio only decrypts messages for programmed slots, so `/join`
  must call `set_channel(idx, name, secret)`.
- **Slot names may lack the `#`** (`Public`, `NTA404`). Compare channel names
  with `channels::same_channel` (case-insensitive, ignores leading `#`).
- **Channel secret derivation.** Hashtag channel: `secret = sha256("#name")[0..16]`
  (hash includes the `#`). The default **Public** channel uses the well-known key
  `8b3387e9c5cdea6ac9e5edbaa115cd72`. Hashtag channels are *not* private —
  anyone who knows the name can derive the key.
- **Channel messages carry no sender field.** `ChannelMessage.text` is the raw
  on-air text; by MeshCore convention the sender's node name is prepended as
  `"<Name>: <text>"`. Split on the first `": "` for display; when *sending* the
  firmware prepends our own name, so send the bare text.
- **Slot 0 is normally "Public".** All pre-programmed slots are treated as
  joined at startup (windows opened for each).
- **The `Advertisement` push parser in meshcore-rs 0.2 yields a garbage name**
  (`"$"`) on fw v1.17; only its `prefix` is trusted. Contact names come from
  `get_contacts` / `NewContact`.
- **Mesh clocks are skewed.** `last_advert` can be hours in the future; `age()`
  renders that as `now`.
- **Contacts are pushed, not polled.** Contact list is fetched once with
  `get_contacts(0)`; new nodes arrive as `EventType::NewContact` /
  `EventType::Advertisement` pushes. Keep a `HashMap<[u8;32], Contact>` in app state.
- **`Contact.contact_type`**: `1` = chat (companion), `2` = repeater, `3` = room server.
  Show as icons in the sidebar (`●` chat, `▲` repeater, `■` room).
- **Incoming messages need pulling.** The radio pushes `MessagesWaiting`; the
  library's `start_auto_message_fetching()` then loops `get_msg()` until
  `NoMoreMessages`, emitting `ContactMsgRecv` / `ChannelMsgRecv` events.
- **DM delivery ACKs.** `send_msg` returns `MsgSentInfo { expected_ack: [u8;4], suggested_timeout }`.
  A later `EventType::Ack { tag }` with a matching tag means delivered.
- **Repeat detection without crypto.** The firmware pushes every received packet
  as `LogData` (before dedup) but never our own TX, and never delivers repeats
  of our own packets as messages (they are marked seen at send). So a
  `GroupText`/`TextMsg` `LogData` with `path_len >= 1` whose payload size equals
  our recent send (`channel_payload_len` / `direct_payload_len` in `app.rs`) and
  that is *not* followed within 1.5 s by a `ChannelMsgRecv`/`ContactMsgRecv`
  is treated as "repeat heard". Do **not** decrypt packets in the client —
  the device owns packet formats.
- **Markers**: `○` pending, green `●` heard/`●●` acked, red `●` failed (60 s
  without a repeat, or DM ack timeout). Failed never overrides heard/acked.
- **Text limit** is 160 bytes per message. Reject or split longer input.
- **`send_msg` needs a `Contact`** (or 6-byte pubkey prefix) — never a name.

## Crates (verified to resolve together and `cargo check` on Rust 1.98)

```toml
[package]
name = "meshirc"
version = "0.1.0"
edition = "2021"

[dependencies]
meshcore-rs = { version = "0.2", default-features = false, features = ["serial"] }  # no BLE → no btleplug/dbus
tokio       = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "fs", "io-util"] }
ratatui     = "0.30"
crossterm   = { version = "0.29", features = ["event-stream"] }  # async EventStream
tui-input   = "0.15"        # single-line input widget with cursor/history handling
futures     = "0.3"         # StreamExt for event_stream()
sha2        = "0.11"        # channel secret derivation
hex         = "0.4"
clap        = { version = "4", features = ["derive"] }
serde       = { version = "1", features = ["derive"] }
toml        = "1"
directories = "6"           # ~/.config / ~/.local/share
chrono      = "0.4"         # timestamps in log lines
tracing     = "0.1"
tracing-appender = "0.2"    # debug log to file (stdout is the TUI)
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow      = "1"
```

Notes:
- `ratatui 0.30` depends on `crossterm 0.29` and `tui-input 0.15` on `ratatui 0.30`; a single copy of each ends up in the graph.
- Keep BLE off; `btleplug` pulls in D-Bus and is irrelevant for a USB client.

## Architecture

```
src/
  main.rs         clap args → load config → init tracing (file) → run app
  config.rs       Config struct (serde), merge file + CLI
  radio.rs        RadioTask: owns MeshCore, translates RadioCmd → meshcore-rs calls,
                  forwards MeshCoreEvents as AppEvent::Radio(...)
  channels.rs     secret derivation, slot bookkeeping, name↔idx map
  contacts.rs     ContactBook: HashMap by pubkey, name/prefix resolution, sort for sidebar
  commands.rs     parse "/join #x", "/msg", ... into Command enum
  app.rs          App state: windows, active idx, unread flags, input, contact book
  ui/
    mod.rs        draw(): layout = [main chat | sidebar] / status bar / input
    chat.rs       scrollback Paragraph with wrap + scroll offset
    sidebar.rs    contact list
    statusbar.rs  "[1:status] [2:#lt] … [act: 3]  MyNode  bat 87%"
  logs.rs         per-window append-only log writer + tail reader
  event.rs        AppEvent enum, terminal input task
```

### Concurrency model

Three tokio tasks talk over channels; the UI never awaits the radio.

```rust
enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Resize,
    Radio(meshcore_rs::MeshCoreEvent),          // raw pass-through from event_stream()
    RadioResult(RadioReply),                     // reply to a RadioCmd (join ok, whois status, error…)
    Tick,
}

enum RadioCmd {
    SendDm { contact: Contact, text: String, window: usize },
    SendChannel { idx: u8, text: String },
    JoinChannel { idx: u8, name: String, secret: [u8; 16] },
    PartChannel { idx: u8 },
    Whois { contact: Contact },                 // request_status
    RefreshContacts,
    Advert,
}
```

- **terminal task**: `crossterm::event::EventStream` → `AppEvent::Key`.
- **radio task**: `MeshCore::serial()`, `send_appstart()`, `send_device_query()`,
  `get_contacts(0)`, then `start_auto_message_fetching()`. Runs
  `select!` over `event_stream()` (→ `AppEvent::Radio`) and the `RadioCmd`
  receiver. Every `commands().lock().await.<call>()` happens here only, so the
  mutex never blocks the UI.
- **main loop**: `while let Some(ev) = rx.recv().await { app.handle(ev); terminal.draw(|f| ui::draw(f, &app))?; }`

### Startup sequence (radio task)

```rust
let mc = MeshCore::serial(&cfg.port, cfg.baud).await?;
let me = mc.commands().lock().await.send_appstart().await?;          // SelfInfo: name, pubkey, tx_power
let dev = mc.commands().lock().await.send_device_query().await?;     // max_channels, fw version
let contacts = mc.commands().lock().await.get_contacts(0).await?;
for idx in 0..dev.max_channels.unwrap_or(8) {                        // discover already-programmed slots
    if let Ok(ch) = mc.commands().lock().await.get_channel(idx).await {
        if !ch.name.is_empty() { tx.send(AppEvent::RadioResult(RadioReply::SlotInUse(idx, ch.name))).await?; }
    }
}
mc.start_auto_message_fetching().await;
let mut events = mc.event_stream();
```

### Channel secret

```rust
use sha2::{Digest, Sha256};

pub const PUBLIC_KEY: [u8; 16] = [
    0x8b, 0x33, 0x87, 0xe9, 0xc5, 0xcd, 0xea, 0x6a, 0xc9, 0xe5, 0xed, 0xba, 0xa1, 0x15, 0xcd, 0x72,
];

pub fn hashtag_secret(name: &str) -> [u8; 16] {
    // name must include the leading '#'
    let h = Sha256::digest(name.as_bytes());
    h[..16].try_into().unwrap()
}
```

### Sending

```rust
// DM
let info = cmds.send_msg(contact.clone(), &text, None).await?;
pending_acks.insert(info.expected_ack, (window, line_id, Instant::now() + Duration::from_millis(info.suggested_timeout as u64)));

// channel — firmware prepends "<our name>: "
cmds.send_channel_msg(idx, &text, None).await?;
```

### Receiving (in main loop, from `AppEvent::Radio`)

```rust
match ev.payload {
    EventPayload::ContactMessage(m) => {
        let who = contacts.by_prefix(&m.sender_prefix);          // Option<&Contact>
        let win = app.query_window_for(who, &m.sender_prefix);   // create query window on first DM
        app.push_line(win, Line::msg(m.sender_timestamp, who.name(), &m.text));
    }
    EventPayload::ChannelMessage(m) => {
        let (nick, body) = m.text.split_once(": ").unwrap_or(("?", &m.text));
        let win = app.channel_window(m.channel_idx);
        app.push_line(win, Line::msg(m.sender_timestamp, nick, body));
    }
    EventPayload::Contact(c) /* NewContact */ => contacts.upsert(c),
    EventPayload::Advertisement(a) => contacts.touch(&a.prefix, a.name),
    EventPayload::Ack { tag } => app.mark_delivered(tag),
    _ => {}
}
```

## Commands

| Command | Behaviour |
|---|---|
| `/join #name [key]` | Derive secret from the name (or use the given 32-hex-char key verbatim; name kept as typed), find free slot (or existing slot with same name), `set_channel`, open window. Error if no free slot. |
| `/part [#name]` | `set_channel(idx, "", [0;16])` to clear the slot, close window. |
| `/msg <who> <text>` | Resolve contact, send DM, open/activate query window. |
| `/query <who>` | Open a query window without sending. |
| `/whois <who>` | Print cached fields. |
| `/status <who>` | `send_binary_req(Status)` async; print result in the issuing window. |
| `/contacts` | Force `get_contacts(0)` refresh. |
| `/advert` | `send_advert(false)` — local zero-hop advert. `/advert flood` for flood. |
| `/win N`, `/close` | Window management. |
| `/nick <name>` | `set_name` on the radio (optional, low priority). |
| `/help`, `/quit` | Obvious. |
| plain text | Send to the active window: channel → `send_channel_msg`, query → `send_msg`, status → error. |

## Keybindings

`Alt+0..9` / `Ctrl+N` / `Ctrl+P` switch windows · `PgUp`/`PgDn` scroll · `Tab` complete contact/channel names · `Up`/`Down` input history · `Ctrl+C` / `/quit` exit.

Focus model (`App::focus`): with an empty input, `Tab` cycles Input → Chat → Contacts; `Shift+Tab` reverses; `Esc` or typing a printable char returns to Input. Chat focus: `Up`/`Down`/`Home`/`End` scroll. Contacts focus: `Up`/`Down` move `selected_contact` (tracked by pubkey so re-sorting doesn't lose it), `Enter` opens/activates the query window. The prompt shows `[win|scroll]` / `[win|contacts]` and the sidebar title turns yellow when focused.

## Logging

- Chat logs: `~/.local/share/meshirc/logs/<window>.log`, one line per message:
  `2026-09-15 14:03:11 <Name> text` (queries: `<Name>` / `<me>`).
- Debug log: `~/.local/share/meshirc/meshirc.log` via `tracing-appender`
  (`RUST_LOG=meshcore_rs=debug` for wire-level tracing). **Never log to stdout** — it is the TUI.

## Status

All planned items are implemented and verified against a real Heltec V3
(fw v1.17.1): connect, slot discovery, contacts, `/join`/`/part`, queries,
`/whois`, `/status`, window management, tab completion, history,
logs, battery, reconnect loop, unit tests. `examples/probe.rs` is the
no-TUI smoke test. Not yet exercised on air by an automated test: sending
channel/DM text and receiving replies (needs a second node).

## Conventions

- `cargo test` must pass; no `cargo fmt`/`clippy` installed on this machine yet.
- Never print to stdout/stderr while the TUI runs; use `tracing`.
- Bracketed paste is enabled; `Event::Paste` is turned into `AppEvent::Paste` and inserted into the input line.
- Radio calls only happen in `radio.rs`; `app.rs` talks to it via `RadioCmd`
  and never awaits.
- Test the TUI headlessly with tmux: `tmux new -d -s m -x 100 -y 30 ./target/debug/meshirc`,
  `tmux send-keys -t m "/join #x" Enter`, `tmux capture-pane -t m -p`.

## Open questions (decide when reached)

- `/part` currently clears the radio slot (implemented). Alternative: keep it programmed and only hide the window.
- Messages > 160 bytes are rejected (implemented). Alternative: auto-split.
- `set_time` at startup if the radio clock is far off — not implemented.
- Sidebar has no scrolling; with 300+ contacts only the most recent fit.
