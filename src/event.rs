use crossterm::event::{Event, EventStream, KeyEvent};
use futures::StreamExt;
use meshcore_rs::events::{Contact, DeviceInfoData, MeshCoreEvent, SelfInfo, StatusData};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    Resize,
    Radio(MeshCoreEvent),
    Reply(RadioReply),
    Tick,
}

#[derive(Debug)]
pub enum RadioCmd {
    SendDm { contact: Contact, text: String, window: usize, line: usize },
    SendChannel { idx: u8, text: String, window: usize, line: usize },
    JoinChannel { idx: u8, name: String, secret: [u8; 16] },
    PartChannel { idx: u8, name: String },
    Whois { contact: Contact, window: usize },
    RefreshContacts,
    Advert { flood: bool },
    SetName(String),
    Battery,
    Shutdown,
}

#[derive(Debug)]
pub enum RadioReply {
    Connected { me: SelfInfo, dev: DeviceInfoData },
    Disconnected(String),
    SlotInUse { idx: u8, name: String },
    Contacts(Vec<Contact>),
    Joined { idx: u8, name: String },
    Parted { idx: u8, name: String },
    DmSent { window: usize, line: usize, tag: [u8; 4], timeout_ms: u32 },
    ChannelSent { window: usize, line: usize },
    SendFailed { window: usize, line: usize, error: String },
    Status { window: usize, name: String, status: StatusData },
    Battery { mv: u16, pct: u8 },
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
