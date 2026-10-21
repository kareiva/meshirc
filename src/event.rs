use crossterm::event::{Event, EventStream, KeyEvent};
use futures::StreamExt;
use meshcore_rs::events::{Contact, DeviceInfoData, MeshCoreEvent, SelfInfo, StatusData};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    Resize,
    Radio(MeshCoreEvent),
    Reply(RadioReply),
    Tick,
    /// Web view asks for the current state as JSON (`App::web_snapshot`).
    WebSnapshot(oneshot::Sender<String>),
}

#[derive(Debug)]
pub enum RadioCmd {
    SendDm { contact: Contact, text: String, window: usize, line: usize },
    /// Repeater remote-control command: a DM with `txt_type` = CLI data (no ACK; the repeater
    /// answers with a CLI-data DM).
    SendCommand { contact: Contact, text: String, window: usize, line: usize },
    Login { contact: Contact, password: String, window: usize },
    SendChannel { idx: u8, text: String, window: usize, line: usize },
    JoinChannel { idx: u8, name: String, secret: [u8; 16] },
    PartChannel { idx: u8, name: String },
    Whois { contact: Contact, window: usize },
    /// `path` / `flags` as built by `contacts::trace_path`.
    Trace { name: String, path: Vec<u8>, flags: u8, window: usize },
    RefreshContacts,
    /// Delete the contact from the radio's contact table.
    RemoveContact { contact: Contact },
    Advert { flood: bool },
    SetName(String),
    Battery,
    Gps,
    Shutdown,
}

#[derive(Debug)]
pub enum RadioReply {
    Connected { me: SelfInfo, dev: DeviceInfoData },
    Disconnected(String),
    SlotInUse { idx: u8, name: String, secret: [u8; 16] },
    Contacts(Vec<Contact>),
    ContactRemoved { key: [u8; 32], name: String },
    Joined { idx: u8, name: String, secret: [u8; 16] },
    Parted { idx: u8, name: String },
    DmSent { window: usize, line: usize, tag: [u8; 4], timeout_ms: u32 },
    ChannelSent { window: usize, line: usize },
    CommandSent { window: usize, line: usize, prefix: [u8; 6] },
    /// `ok`: `Some(true)` logged in, `Some(false)` rejected, `None` no reply.
    Login { window: usize, key: [u8; 32], name: String, ok: Option<bool> },
    SendFailed { window: usize, line: usize, error: String },
    Status { window: usize, name: String, status: StatusData },
    /// SNR (dB) at which each hop of `path` heard the trace, then at which we heard the last hop.
    Trace { window: usize, name: String, path: Vec<u8>, flags: u8, snrs: Vec<f32>, final_snr: f32 },
    Battery { mv: u16, pct: u8 },
    /// Reply to the firmware `gps` CLI command (`on, active, fix, 7 sats` / `off`); `None` if unsupported.
    Gps(Option<String>),
    Notice { window: Option<usize>, text: String },
    Error(String),
}

pub fn spawn_terminal_task(tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut stream = EventStream::new();
        while let Some(Ok(ev)) = stream.next().await {
            let msg = match ev {
                Event::Key(k) => AppEvent::Key(k),
                Event::Paste(s) => AppEvent::Paste(s),
                Event::Resize(_, _) => AppEvent::Resize,
                _ => continue,
            };
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    });
}

pub fn spawn_tick_task(tx: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            iv.tick().await;
            if tx.send(AppEvent::Tick).await.is_err() {
                break;
            }
        }
    });
}
