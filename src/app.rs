use crate::channels::{self, SlotTable, MAX_TEXT_BYTES};
use crate::commands::{self, Command, HELP};
use crate::config::Config;
use crate::contacts::{self, ContactBook, Resolve};
use crate::event::{AppEvent, RadioCmd, RadioReply};
use crate::logs::LogStore;
use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use meshcore_rs::events::{Contact, DeviceInfoData, EventPayload, SelfInfo, StatusData};
use meshcore_rs::PayloadType;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

#[derive(Debug, Clone, PartialEq)]
pub enum WindowKind {
    Status,
    Channel { idx: u8, name: String },
    Query { prefix: [u8; 6], name: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    Pending,
    Heard,
    Acked,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PktKind {
    Channel,
    Direct,
}

struct PendingSend {
    window: usize,
    line: usize,
    kind: PktKind,
    payload_len: usize,
    deadline: Instant,
}

struct HeardPacket {
    kind: PktKind,
    payload_len: usize,
    at: Instant,
}

const REPEAT_WINDOW: Duration = Duration::from_secs(60);
const HEARD_GRACE: Duration = Duration::from_millis(1500);
const FW_MAX_TEXT_LEN: usize = 160;

fn pad16(n: usize) -> usize {
    n.div_ceil(16) * 16
}

// Expected over-the-air payload sizes, used only to correlate repeats heard in the RX log.
fn channel_payload_len(sender: &str, text: &str) -> usize {
    let prefix = sender.len() + 2;
    let text_len = text.len().min(FW_MAX_TEXT_LEN.saturating_sub(prefix));
    1 + 2 + pad16(5 + prefix + text_len)
}

fn direct_payload_len(text: &str) -> usize {
    2 + 2 + pad16(5 + text.len().min(FW_MAX_TEXT_LEN))
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    Msg { nick: String, own: bool, mark: Option<Mark> },
    Notice,
    Error,
    History,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub time: String,
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Input,
    Chat,
    Contacts,
}

pub struct Window {
    pub kind: WindowKind,
    pub lines: Vec<Line>,
    pub scroll: usize,
    pub activity: u8,
}

impl Window {
    pub fn name(&self) -> String {
        match &self.kind {
            WindowKind::Status => "status".into(),
            WindowKind::Channel { name, .. } => name.clone(),
            WindowKind::Query { name, .. } => name.clone(),
        }
    }

    fn log_name(&self) -> String {
        match &self.kind {
            WindowKind::Status => "status".into(),
            WindowKind::Channel { name, .. } => name.clone(),
            WindowKind::Query { name, prefix } => format!("{}_{}", name, hex::encode(prefix)),
        }
    }
}

pub struct App {
    pub cfg: Config,
    pub windows: Vec<Window>,
    pub active: usize,
    pub input: Input,
    history: Vec<String>,
    history_pos: Option<usize>,
    pub contacts: ContactBook,
    pub slots: SlotTable,
    pub me: Option<SelfInfo>,
    pub dev: Option<DeviceInfoData>,
    pub battery: Option<(u16, u8)>,
    pub connected: bool,
    logs: LogStore,
    radio: mpsc::Sender<RadioCmd>,
    pending_acks: HashMap<[u8; 4], (usize, usize)>,
    pending_sends: Vec<PendingSend>,
    heard: Vec<HeardPacket>,
    pending_joins: Vec<String>,
    ticks: u64,
    pub should_quit: bool,
    pub page_height: usize,
    pub focus: Focus,
    pub selected_contact: Option<[u8; 32]>,
    pub sidebar_height: usize,
}

fn now_hms() -> String {
    Local::now().format("%H:%M").to_string()
}

fn now_unix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

impl App {
    pub fn new(cfg: Config, radio: mpsc::Sender<RadioCmd>) -> anyhow::Result<Self> {
        let logs = LogStore::new(&cfg.log_dir)?;
        let mut app = App {
            windows: vec![Window { kind: WindowKind::Status, lines: vec![], scroll: 0, activity: 0 }],
            active: 0,
            input: Input::default(),
            history: vec![],
            history_pos: None,
            contacts: ContactBook::default(),
            slots: SlotTable::new(8),
            me: None,
            dev: None,
            battery: None,
            connected: false,
            logs,
            radio,
            pending_acks: HashMap::new(),
            pending_sends: Vec::new(),
            heard: Vec::new(),
            pending_joins: cfg.auto_join.iter().map(|n| channels::normalize_name(n)).collect(),
            ticks: 0,
            should_quit: false,
            page_height: 20,
            focus: Focus::Input,
            selected_contact: None,
            sidebar_height: 20,
            cfg,
        };
        app.notice(0, "MeshIRC — /help for commands");
        app.notice(0, &format!("connecting to {} @ {}...", app.cfg.port, app.cfg.baud));
        Ok(app)
    }

    // ----- output helpers -----

    fn push(&mut self, win: usize, line: Line, log: bool, level: u8) {
        let Some(w) = self.windows.get_mut(win) else { return };
        if log {
            let entry = match &line.kind {
                LineKind::Msg { nick, .. } => format!("{} <{}> {}", Local::now().format("%Y-%m-%d %H:%M:%S"), nick, line.text),
                _ => format!("{} -!- {}", Local::now().format("%Y-%m-%d %H:%M:%S"), line.text),
            };
            self.logs.append(&w.log_name(), &entry);
        }
        w.lines.push(line);
        if w.scroll > 0 {
            w.scroll += 1;
        }
        if win != self.active {
            w.activity = w.activity.max(level);
        }
    }

    pub fn notice(&mut self, win: usize, text: &str) {
        self.push(win, Line { time: now_hms(), kind: LineKind::Notice, text: text.to_string() }, false, 1);
    }

    fn error(&mut self, text: &str) {
        let w = self.active;
        self.push(w, Line { time: now_hms(), kind: LineKind::Error, text: text.to_string() }, false, 1);
    }

    fn message(&mut self, win: usize, nick: &str, text: &str, own: bool, mark: Option<Mark>) -> usize {
        self.push(
            win,
            Line { time: now_hms(), kind: LineKind::Msg { nick: nick.to_string(), own, mark }, text: text.to_string() },
            true,
            2,
        );
        self.windows[win].lines.len() - 1
    }

    // ----- windows -----

    fn find_channel(&self, idx: u8) -> Option<usize> {
        self.windows.iter().position(|w| matches!(&w.kind, WindowKind::Channel { idx: i, .. } if *i == idx))
    }

    fn find_query(&self, prefix: &[u8; 6]) -> Option<usize> {
        self.windows.iter().position(|w| matches!(&w.kind, WindowKind::Query { prefix: p, .. } if p == prefix))
    }

    fn open_window(&mut self, kind: WindowKind) -> usize {
        let w = Window { kind, lines: vec![], scroll: 0, activity: 0 };
        let history = self.logs.tail(&w.log_name(), self.cfg.history_lines);
        self.windows.push(w);
        let idx = self.windows.len() - 1;
        for h in history {
            let (time, text) = h.split_at(h.len().min(19));
            let time = time.get(11..16).unwrap_or("").to_string();
            self.windows[idx].lines.push(Line { time, kind: LineKind::History, text: text.trim_start().to_string() });
        }
        idx
    }

    fn switch(&mut self, idx: usize) {
        if idx < self.windows.len() {
            self.active = idx;
            self.windows[idx].activity = 0;
        }
    }

    fn close_active(&mut self) {
        if self.active == 0 {
            self.error("cannot close status window");
            return;
        }
        if let WindowKind::Channel { idx, name } = self.windows[self.active].kind.clone() {
            let _ = self.radio.try_send(RadioCmd::PartChannel { idx, name });
            return;
        }
        self.windows.remove(self.active);
        self.active = self.active.min(self.windows.len() - 1);
    }

    fn query_window(&mut self, contact: Option<&Contact>, prefix: &[u8; 6]) -> usize {
        if let Some(i) = self.find_query(prefix) {
            return i;
        }
        let name = contact.map(|c| c.adv_name.clone()).unwrap_or_else(|| hex::encode(prefix));
        self.open_window(WindowKind::Query { prefix: *prefix, name })
    }

    // ----- input -----

    pub fn handle(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(k) => self.on_key(k),
            AppEvent::Paste(text) => {
                for c in text.chars() {
                    self.input.handle(InputRequest::InsertChar(if c == '\n' || c == '\r' { ' ' } else { c }));
                }
            }
            AppEvent::Resize => {}
            AppEvent::Tick => self.on_tick(),
            AppEvent::Reply(r) => self.on_reply(r),
            AppEvent::Radio(ev) => self.on_radio(ev.payload),
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match (k.code, ctrl, alt) {
            (KeyCode::Char('c'), true, _) => self.should_quit = true,
            (KeyCode::Char(c), _, true) if c.is_ascii_digit() => {
                let n = c.to_digit(10).unwrap() as usize;
                self.switch(n);
            }
            (KeyCode::Char('n'), true, _) | (KeyCode::Right, _, true) => {
                let n = (self.active + 1) % self.windows.len();
                self.switch(n);
            }
            (KeyCode::Char('p'), true, _) | (KeyCode::Left, _, true) => {
                let n = (self.active + self.windows.len() - 1) % self.windows.len();
                self.switch(n);
            }
            (KeyCode::PageUp, _, _) if self.focus == Focus::Contacts => self.move_selection(-(self.sidebar_height as i64)),
            (KeyCode::PageDown, _, _) if self.focus == Focus::Contacts => self.move_selection(self.sidebar_height as i64),
            (KeyCode::PageUp, _, _) => self.scroll_chat(self.page_height as i64 / 2),
            (KeyCode::PageDown, _, _) => self.scroll_chat(-(self.page_height as i64) / 2),
            (KeyCode::Tab, _, _) if self.focus != Focus::Input || self.input.value().is_empty() => {
                self.focus = match self.focus {
                    Focus::Input => Focus::Chat,
                    Focus::Chat => Focus::Contacts,
                    Focus::Contacts => Focus::Input,
                };
                if self.focus == Focus::Contacts && self.selected_contact.is_none() {
                    self.selected_contact = self.contacts.sorted().first().map(|c| c.public_key);
                }
            }
            (KeyCode::BackTab, _, _) => {
                self.focus = match self.focus {
                    Focus::Input => Focus::Contacts,
                    Focus::Chat => Focus::Input,
                    Focus::Contacts => Focus::Chat,
                };
            }
            (KeyCode::Esc, _, _) => self.focus = Focus::Input,
            _ => match self.focus {
                Focus::Input => self.on_input_key(k),
                Focus::Chat => self.on_chat_key(k),
                Focus::Contacts => self.on_contacts_key(k),
            },
        }
    }

    fn on_input_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Tab => self.complete(),
            KeyCode::Up if k.modifiers.is_empty() => self.history_nav(-1),
            KeyCode::Down if k.modifiers.is_empty() => self.history_nav(1),
            KeyCode::Enter => {
                let line = self.input.value_and_reset();
                self.history_pos = None;
                if !line.trim().is_empty() {
                    self.history.push(line.clone());
                }
                if let Some(cmd) = commands::parse(&line) {
                    self.execute(cmd);
                }
            }
            _ => {
                self.input.handle_event(&Event::Key(k));
            }
        }
    }

    fn on_chat_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up => self.scroll_chat(1),
            KeyCode::Down => self.scroll_chat(-1),
            KeyCode::Home => self.scroll_chat(i64::MAX / 2),
            KeyCode::End => self.scroll_chat(i64::MIN / 2),
            KeyCode::Char(_) => self.type_into_input(k),
            _ => {}
        }
    }

    fn on_contacts_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Home => self.move_selection(i64::MIN / 2),
            KeyCode::End => self.move_selection(i64::MAX / 2),
            KeyCode::Enter => {
                if let Some(c) = self.selected_contact.and_then(|k| self.contacts.by_prefix(&k).cloned()) {
                    let prefix = c.prefix();
                    let win = self.query_window(Some(&c), &prefix);
                    self.switch(win);
                    self.focus = Focus::Input;
                }
            }
            KeyCode::Char(_) => self.type_into_input(k),
            _ => {}
        }
    }

    fn type_into_input(&mut self, k: KeyEvent) {
        if k.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
            self.focus = Focus::Input;
            self.input.handle_event(&Event::Key(k));
        }
    }

    fn scroll_chat(&mut self, delta: i64) {
        let w = &mut self.windows[self.active];
        let max = w.lines.len().saturating_sub(1) as i64;
        w.scroll = (w.scroll as i64).saturating_add(delta).clamp(0, max) as usize;
    }

    fn move_selection(&mut self, delta: i64) {
        let sorted = self.contacts.sorted();
        if sorted.is_empty() {
            self.selected_contact = None;
            return;
        }
        let cur = self.selected_contact.and_then(|k| sorted.iter().position(|c| c.public_key == k)).unwrap_or(0) as i64;
        let next = cur.saturating_add(delta).clamp(0, sorted.len() as i64 - 1) as usize;
        self.selected_contact = Some(sorted[next].public_key);
    }

    fn history_nav(&mut self, dir: i32) {
        if self.history.is_empty() {
            return;
        }
        let pos = match (self.history_pos, dir) {
            (None, -1) => self.history.len() - 1,
            (None, _) => return,
            (Some(p), -1) => p.saturating_sub(1),
            (Some(p), _) if p + 1 >= self.history.len() => {
                self.history_pos = None;
                self.input.reset();
                return;
            }
            (Some(p), _) => p + 1,
        };
        self.history_pos = Some(pos);
        self.input = Input::new(self.history[pos].clone());
    }

    fn complete(&mut self) {
        let value = self.input.value().to_string();
        let mut splits: Vec<usize> = vec![0];
        splits.extend(value.match_indices(' ').map(|(i, _)| i + 1));
        for at in splits {
            let (head, word) = value.split_at(at);
            if word.is_empty() || word.starts_with('/') {
                continue;
            }
            let mut cands: Vec<String> = if word.starts_with('#') {
                self.slots.iter().map(|(_, n)| channels::normalize_name(n)).filter(|n| n.to_lowercase().starts_with(&word.to_lowercase())).collect()
            } else {
                self.contacts.complete(word)
            };
            cands.sort();
            cands.dedup();
            match cands.len() {
                0 => continue,
                1 => {
                    let suffix = if head.is_empty() { ": " } else { " " };
                    self.input = Input::new(format!("{head}{}{suffix}", cands[0]));
                }
                _ => {
                    let common = common_prefix(&cands);
                    if common.len() > word.len() {
                        self.input = Input::new(format!("{head}{common}"));
                    } else {
                        let shown: Vec<&str> = cands.iter().take(20).map(String::as_str).collect();
                        self.notice(self.active, &shown.join("  "));
                    }
                }
            }
            return;
        }
    }

    // ----- commands -----

    fn execute(&mut self, cmd: Command) {
        match cmd {
            Command::Quit => self.should_quit = true,
            Command::Help => {
                for l in HELP {
                    self.notice(self.active, l);
                }
            }
            Command::Unknown(msg) => self.error(&msg),
            Command::Win(n) => {
                if n >= self.windows.len() {
                    self.error("no such window");
                } else {
                    self.switch(n);
                }
            }
            Command::Close => self.close_active(),
            Command::Join { name, key } => match key {
                Some(k) => self.join(&name, Some(k)),
                None => self.join(&channels::normalize_name(&name), None),
            },
            Command::Part(name) => {
                let target = match name {
                    Some(n) => self.slots.idx_of(&channels::normalize_name(&n)).map(|i| (i, channels::normalize_name(&n))),
                    None => match &self.windows[self.active].kind {
                        WindowKind::Channel { idx, name } => Some((*idx, name.clone())),
                        _ => None,
                    },
                };
                match target {
                    Some((idx, name)) => {
                        self.send_radio(RadioCmd::PartChannel { idx, name });
                    }
                    None => self.error("not a joined channel"),
                }
            }
            Command::Msg { who, text } => {
                if let Some(c) = self.resolve_or_error(&who) {
                    let prefix = c.prefix();
                    let win = self.query_window(Some(&c), &prefix);
                    self.switch(win);
                    self.send_dm(win, c, &text);
                }
            }
            Command::Query(who) => {
                if let Some(c) = self.resolve_or_error(&who) {
                    let prefix = c.prefix();
                    let win = self.query_window(Some(&c), &prefix);
                    self.switch(win);
                }
            }
            Command::Whois(who) => {
                if let Some(c) = self.resolve_or_error(&who) {
                    self.print_whois(&c);
                }
            }
            Command::Status(who) => {
                if let Some(c) = self.resolve_or_error(&who) {
                    let window = self.active;
                    if self.send_radio(RadioCmd::Whois { contact: c.clone(), window }) {
                        self.notice(window, &format!("requesting status from {}...", c.adv_name));
                    }
                }
            }
            Command::Contacts => {
                self.send_radio(RadioCmd::RefreshContacts);
                self.notice(self.active, "refreshing contacts...");
            }
            Command::Advert { flood } => {
                self.send_radio(RadioCmd::Advert { flood });
            }
            Command::Nick(name) => {
                self.send_radio(RadioCmd::SetName(name));
            }
            Command::Say(text) => self.say(&text),
        }
    }

    fn send_radio(&mut self, cmd: RadioCmd) -> bool {
        if !self.connected {
            self.error("radio not connected");
            return false;
        }
        if self.radio.try_send(cmd).is_err() {
            self.error("radio busy, try again");
            return false;
        }
        true
    }

    fn resolve_or_error(&mut self, who: &str) -> Option<Contact> {
        match self.contacts.resolve(who) {
            Resolve::One(c) => Some(c.clone()),
            Resolve::Ambiguous(v) => {
                let names: Vec<String> = v.iter().map(|c| format!("{} ({})", c.adv_name, c.prefix_hex())).collect();
                self.error(&format!("ambiguous: {}", names.join(", ")));
                None
            }
            Resolve::None => {
                self.error(&format!("no contact matching '{who}'"));
                None
            }
        }
    }

    fn join(&mut self, name: &str, key: Option<[u8; 16]>) {
        if let Some(idx) = self.slots.idx_of(name) {
            let win = self.find_channel(idx).unwrap_or_else(|| self.open_window(WindowKind::Channel { idx, name: name.to_string() }));
            self.switch(win);
            return;
        }
        if !self.connected {
            self.pending_joins.push(name.to_string());
            self.notice(self.active, &format!("will join {name} when connected"));
            return;
        }
        match self.slots.free_slot() {
            Some(idx) => {
                let secret = key.unwrap_or_else(|| channels::secret_for(name));
                self.send_radio(RadioCmd::JoinChannel { idx, name: name.to_string(), secret });
            }
            None => self.error(&format!("all {} channel slots are in use; /part one first", self.slots.max)),
        }
    }

    fn say(&mut self, text: &str) {
        if text.len() > MAX_TEXT_BYTES {
            self.error(&format!("message is {} bytes, max is {MAX_TEXT_BYTES}", text.len()));
            return;
        }
        let win = self.active;
        match self.windows[win].kind.clone() {
            WindowKind::Status => self.error("not in a channel or query; use /join or /msg"),
            WindowKind::Channel { idx, .. } => {
                if !self.connected {
                    return self.error("radio not connected");
                }
                let me = self.me_name();
                let line = self.message(win, &me, text, true, Some(Mark::Pending));
                if !self.send_radio(RadioCmd::SendChannel { idx, text: text.to_string(), window: win, line }) {
                    self.set_mark(win, line, Mark::Failed);
                }
            }
            WindowKind::Query { prefix, .. } => match self.contacts.by_prefix(&prefix).cloned() {
                Some(c) => self.send_dm(win, c, text),
                None => self.error("contact no longer known"),
            },
        }
    }

    fn send_dm(&mut self, win: usize, contact: Contact, text: &str) {
        if text.len() > MAX_TEXT_BYTES {
            self.error(&format!("message is {} bytes, max is {MAX_TEXT_BYTES}", text.len()));
            return;
        }
        if !self.connected {
            return self.error("radio not connected");
        }
        let me = self.me_name();
        let line = self.message(win, &me, text, true, Some(Mark::Pending));
        if !self.send_radio(RadioCmd::SendDm { contact, text: text.to_string(), window: win, line }) {
            self.set_mark(win, line, Mark::Failed);
        }
    }

    fn me_name(&self) -> String {
        self.me.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| "me".into())
    }

    fn print_whois(&mut self, c: &Contact) {
        let now = now_unix();
        let last = self.contacts.last_advert(c);
        let lines = vec![
            format!("{} — {}", c.adv_name, contacts::type_name(c.contact_type)),
            format!("  pubkey    {}", hex::encode(&c.public_key[..16])),
            format!("  last seen {} ago", contacts::age(now, last)),
            format!("  location  {:.5}, {:.5}", c.latitude(), c.longitude()),
            format!("  path      {}", if c.path_len < 0 { "flood".to_string() } else { format!("{} hops via {}", c.path_len, hex::encode(&c.out_path)) }),
        ];
        for l in lines {
            self.notice(self.active, &l);
        }
    }

    fn print_status(&mut self, win: usize, name: &str, s: &StatusData) {
        let up = s.uptime;
        let lines = vec![
            format!("{name} status:"),
            format!("  battery   {:.2} V", s.battery_mv as f32 / 1000.0),
            format!("  uptime    {}d {:02}h {:02}m", up / 86400, (up % 86400) / 3600, (up % 3600) / 60),
            format!("  radio     rssi {} dBm, snr {:.1} dB, noise {} dBm", s.last_rssi, s.snr, s.noise_floor),
            format!("  packets   recv {}, sent {}, dup {}, txq {}", s.nb_recv, s.nb_sent, s.dup_count, s.tx_queue_len),
            format!("  airtime   tx {} s, rx {} s", s.airtime, s.rx_airtime),
        ];
        for l in lines {
            self.notice(win, &l);
        }
    }

    // ----- radio -----

    fn on_reply(&mut self, r: RadioReply) {
        match r {
            RadioReply::Connected { me, dev } => {
                self.connected = true;
                self.notice(0, &format!("connected: {} ({}, fw {})", me.name, dev.model.clone().unwrap_or_default(), dev.version.clone().unwrap_or_default()));
                self.notice(0, &format!("pubkey {}  freq {:.3} MHz  sf{} bw{}k tx {} dBm", hex::encode(me.public_key), me.radio_freq as f64 / 1000.0, me.sf, me.radio_bw as f64 / 1000.0, me.tx_power));
                self.slots = SlotTable::new(dev.max_channels.unwrap_or(8));
                self.me = Some(me);
                self.dev = Some(dev);
            }
            RadioReply::Disconnected(e) => {
                self.connected = false;
                self.notice(0, &format!("disconnected: {e} — reconnecting"));
            }
            RadioReply::SlotInUse { idx, name } => {
                self.slots.set(idx, name.clone());
                if self.find_channel(idx).is_none() {
                    self.open_window(WindowKind::Channel { idx, name: name.clone() });
                }
                self.notice(0, &format!("channel slot {idx}: {name}"));
            }
            RadioReply::Contacts(list) => {
                self.contacts.replace_all(list);
                self.notice(0, &format!("{} contacts loaded", self.contacts.len()));
                for name in std::mem::take(&mut self.pending_joins) {
                    self.join(&name, None);
                }
            }
            RadioReply::Joined { idx, name } => {
                self.slots.set(idx, name.clone());
                let win = self.open_window(WindowKind::Channel { idx, name: name.clone() });
                self.switch(win);
                self.notice(win, &format!("joined {name} (slot {idx})"));
            }
            RadioReply::Parted { idx, name } => {
                self.slots.clear(idx);
                if let Some(i) = self.find_channel(idx) {
                    self.windows.remove(i);
                    if self.active >= self.windows.len() || self.active == i {
                        self.active = i.saturating_sub(1).min(self.windows.len() - 1);
                    } else if self.active > i {
                        self.active -= 1;
                    }
                }
                self.notice(0, &format!("left {name}"));
            }
            RadioReply::DmSent { window, line, tag, timeout_ms } => {
                self.pending_acks.insert(tag, (window, line));
                let text = self.line_text(window, line);
                self.pending_sends.push(PendingSend {
                    window,
                    line,
                    kind: PktKind::Direct,
                    payload_len: direct_payload_len(&text),
                    deadline: Instant::now() + Duration::from_millis(timeout_ms.max(5000) as u64).max(REPEAT_WINDOW),
                });
            }
            RadioReply::ChannelSent { window, line } => {
                let text = self.line_text(window, line);
                let me = self.me_name();
                self.pending_sends.push(PendingSend {
                    window,
                    line,
                    kind: PktKind::Channel,
                    payload_len: channel_payload_len(&me, &text),
                    deadline: Instant::now() + REPEAT_WINDOW,
                });
            }
            RadioReply::SendFailed { window, line, error } => {
                self.set_mark(window, line, Mark::Failed);
                self.error(&error);
            }
            RadioReply::Status { window, name, status } => self.print_status(window, &name, &status),
            RadioReply::Battery { mv, pct } => self.battery = Some((mv, pct)),
            RadioReply::Notice { window, text } => {
                let w = window.filter(|w| *w < self.windows.len()).unwrap_or(self.active);
                self.notice(w, &text)
            }
            RadioReply::Error(t) => self.error(&t),
        }
    }

    fn on_radio(&mut self, p: EventPayload) {
        match p {
            EventPayload::LogData(log) => {
                if let Some(h) = &log.header {
                    let kind = match h.payload_type {
                        PayloadType::GroupText => PktKind::Channel,
                        PayloadType::TextMsg => PktKind::Direct,
                        _ => return,
                    };
                    if h.path_len >= 1 && !self.pending_sends.is_empty() {
                        self.heard.push(HeardPacket { kind, payload_len: log.payload.len(), at: Instant::now() });
                    }
                }
            }
            EventPayload::ContactMessage(m) => {
                self.forget_heard(PktKind::Direct);
                let c = self.contacts.by_prefix(&m.sender_prefix).cloned();
                let win = self.query_window(c.as_ref(), &m.sender_prefix);
                let nick = self.windows[win].name();
                self.message(win, &nick, &m.text, false, None);
            }
            EventPayload::ChannelMessage(m) => {
                self.forget_heard(PktKind::Channel);
                let win = match self.find_channel(m.channel_idx) {
                    Some(w) => w,
                    None => {
                        let name = self.slots.name(m.channel_idx).map(String::from).unwrap_or_else(|| format!("#slot{}", m.channel_idx));
                        self.open_window(WindowKind::Channel { idx: m.channel_idx, name })
                    }
                };
                let (nick, body) = m.text.split_once(": ").unwrap_or(("?", m.text.as_str()));
                let (nick, body) = (nick.to_string(), body.to_string());
                self.message(win, &nick, &body, false, None);
            }
            EventPayload::Contact(c) => {
                self.notice(0, &format!("new contact: {} ({})", c.adv_name, contacts::type_name(c.contact_type)));
                self.contacts.upsert(c);
            }
            EventPayload::Contacts(list) => self.contacts.replace_all(list),
            EventPayload::Advertisement(a) => {
                self.contacts.touch(&a.prefix, now_unix());
                if self.contacts.by_prefix(&a.prefix).is_none() && self.connected {
                    let _ = self.radio.try_send(RadioCmd::RefreshContacts);
                }
            }
            EventPayload::Ack { tag } => {
                if let Some((win, line)) = self.pending_acks.remove(&tag) {
                    self.set_mark(win, line, Mark::Acked);
                }
            }
            _ => {}
        }
    }

    fn line_text(&self, win: usize, line: usize) -> String {
        self.windows.get(win).and_then(|w| w.lines.get(line)).map(|l| l.text.clone()).unwrap_or_default()
    }

    fn mark_of(&self, win: usize, line: usize) -> Option<Mark> {
        match self.windows.get(win).and_then(|w| w.lines.get(line)).map(|l| &l.kind) {
            Some(LineKind::Msg { mark, .. }) => *mark,
            _ => None,
        }
    }

    fn set_mark(&mut self, win: usize, line: usize, state: Mark) {
        if let Some(l) = self.windows.get_mut(win).and_then(|w| w.lines.get_mut(line)) {
            if let LineKind::Msg { mark, .. } = &mut l.kind {
                if state == Mark::Failed && matches!(mark, Some(Mark::Heard) | Some(Mark::Acked)) {
                    return;
                }
                if state == Mark::Heard && *mark == Some(Mark::Acked) {
                    return;
                }
                *mark = Some(state);
            }
        }
    }

    // A packet that the radio then delivered as a message was someone else's, not our repeat.
    fn forget_heard(&mut self, kind: PktKind) {
        if let Some(i) = self.heard.iter().rposition(|h| h.kind == kind) {
            self.heard.remove(i);
        }
    }

    // Repeats of our own packets show up in the RX log but are never delivered as messages
    // (the radio has already seen them), so a matching packet that survives the grace period
    // counts as "heard repeat".
    fn match_heard(&mut self) {
        let now = Instant::now();
        let mut i = 0;
        while i < self.heard.len() {
            if now.duration_since(self.heard[i].at) < HEARD_GRACE {
                i += 1;
                continue;
            }
            let h = self.heard.remove(i);
            let hit = self
                .pending_sends
                .iter()
                .position(|p| p.kind == h.kind && p.payload_len == h.payload_len && self.mark_of(p.window, p.line) == Some(Mark::Pending));
            if let Some(pi) = hit {
                let (w, l) = (self.pending_sends[pi].window, self.pending_sends[pi].line);
                self.set_mark(w, l, Mark::Heard);
                if h.kind == PktKind::Channel {
                    self.pending_sends.remove(pi);
                }
            }
        }
    }

    fn on_tick(&mut self) {
        self.ticks += 1;
        self.match_heard();
        let now = Instant::now();
        let expired: Vec<(usize, usize)> = self.pending_sends.iter().filter(|p| p.deadline <= now).map(|p| (p.window, p.line)).collect();
        self.pending_sends.retain(|p| p.deadline > now);
        for (w, l) in expired {
            self.set_mark(w, l, Mark::Failed);
        }
        if self.ticks % 60 == 0 && self.connected {
            let _ = self.radio.try_send(RadioCmd::Battery);
        }
    }

    pub fn now_unix(&self) -> u32 {
        now_unix()
    }
}

fn common_prefix(items: &[String]) -> String {
    let first = &items[0];
    let mut len = first.len();
    for s in &items[1..] {
        len = first.chars().zip(s.chars()).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count().min(len);
    }
    first.chars().take(len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshcore_rs::events::{ChannelMessage, LogData, MeshCoreEvent, MeshPacketHeader};
    use meshcore_rs::{EventType, RouteType};

    fn app() -> App {
        let dir = std::env::temp_dir().join(format!("meshirc-test-{}", std::process::id()));
        let cfg = Config {
            port: String::new(),
            baud: 0,
            log_dir: dir.join("logs"),
            data_dir: dir,
            history_lines: 0,
            auto_join: vec![],
        };
        let (tx, rx) = mpsc::channel(8);
        std::mem::forget(rx);
        let mut app = App::new(cfg, tx).unwrap();
        app.connected = true;
        app.me = Some(SelfInfo { name: "Me".into(), ..Default::default() });
        app.slots.set(2, "#test".into());
        app.open_window(WindowKind::Channel { idx: 2, name: "#test".into() });
        app.switch(1);
        app
    }

    fn log_event(payload_type: PayloadType, path_len: u8, payload_len: usize) -> AppEvent {
        AppEvent::Radio(MeshCoreEvent::new(
            EventType::LogData,
            EventPayload::LogData(LogData {
                snr: 0.0,
                rssi: 0,
                header: Some(MeshPacketHeader {
                    route_type: RouteType::Flood,
                    payload_type,
                    payload_version: 0,
                    transport_code: None,
                    path_len,
                    path_hash_size: 1,
                    path: vec![0; path_len as usize],
                }),
                advertisement: None,
                payload: vec![0; payload_len],
            }),
        ))
    }

    fn settle(app: &mut App) {
        for h in &mut app.heard {
            h.at -= HEARD_GRACE;
        }
        app.handle(AppEvent::Tick);
    }

    #[test]
    fn payload_len_formula() {
        assert_eq!(channel_payload_len("Me", "hi"), 3 + 16);
        assert_eq!(channel_payload_len("Me", "hello world"), 3 + 32);
        assert_eq!(direct_payload_len("hi"), 4 + 16);
    }

    #[test]
    fn channel_repeat_marks_heard() {
        let mut app = app();
        app.execute(Command::Say("hi".into()));
        assert_eq!(app.mark_of(1, 0), Some(Mark::Pending));
        app.handle(AppEvent::Reply(RadioReply::ChannelSent { window: 1, line: 0 }));
        app.handle(log_event(PayloadType::GroupText, 1, channel_payload_len("Me", "hi")));
        settle(&mut app);
        assert_eq!(app.mark_of(1, 0), Some(Mark::Heard));
    }

    #[test]
    fn delivered_message_is_not_our_repeat() {
        let mut app = app();
        app.execute(Command::Say("hi".into()));
        app.handle(AppEvent::Reply(RadioReply::ChannelSent { window: 1, line: 0 }));
        app.handle(log_event(PayloadType::GroupText, 1, channel_payload_len("Me", "hi")));
        app.handle(AppEvent::Radio(MeshCoreEvent::new(
            EventType::ChannelMsgRecv,
            EventPayload::ChannelMessage(ChannelMessage {
                channel_idx: 2,
                path_len: 1,
                txt_type: 0,
                sender_timestamp: 0,
                text: "Bob: yo".into(),
                snr: None,
            }),
        )));
        settle(&mut app);
        assert_eq!(app.mark_of(1, 0), Some(Mark::Pending));
        app.handle(log_event(PayloadType::GroupText, 0, channel_payload_len("Me", "hi")));
        app.handle(log_event(PayloadType::TextMsg, 2, channel_payload_len("Me", "hi")));
        app.handle(log_event(PayloadType::GroupText, 2, channel_payload_len("Me", "hi") + 16));
        settle(&mut app);
        assert_eq!(app.mark_of(1, 0), Some(Mark::Pending));
    }

    #[test]
    fn timeout_marks_failed_unless_heard() {
        let mut app = app();
        app.execute(Command::Say("a".into()));
        app.execute(Command::Say("b".into()));
        app.handle(AppEvent::Reply(RadioReply::ChannelSent { window: 1, line: 0 }));
        app.handle(AppEvent::Reply(RadioReply::ChannelSent { window: 1, line: 1 }));
        app.handle(log_event(PayloadType::GroupText, 3, channel_payload_len("Me", "a")));
        settle(&mut app);
        for p in &mut app.pending_sends {
            p.deadline = Instant::now() - Duration::from_secs(1);
        }
        app.handle(AppEvent::Tick);
        assert_eq!(app.mark_of(1, 0), Some(Mark::Heard));
        assert_eq!(app.mark_of(1, 1), Some(Mark::Failed));
    }

    #[test]
    fn dm_ack_beats_heard() {
        let mut app = app();
        let mut c = Contact {
            public_key: [7u8; 32],
            contact_type: 1,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: "Bob".into(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        };
        c.public_key[0] = 1;
        app.contacts.upsert(c);
        app.execute(Command::Msg { who: "Bob".into(), text: "hey".into() });
        let win = app.active;
        app.handle(AppEvent::Reply(RadioReply::DmSent { window: win, line: 0, tag: [1, 2, 3, 4], timeout_ms: 1000 }));
        app.handle(log_event(PayloadType::TextMsg, 1, direct_payload_len("hey")));
        settle(&mut app);
        assert_eq!(app.mark_of(win, 0), Some(Mark::Heard));
        app.handle(AppEvent::Radio(MeshCoreEvent::new(EventType::Ack, EventPayload::Ack { tag: [1, 2, 3, 4] })));
        assert_eq!(app.mark_of(win, 0), Some(Mark::Acked));
        for p in &mut app.pending_sends {
            p.deadline = Instant::now() - Duration::from_secs(1);
        }
        app.handle(AppEvent::Tick);
        assert_eq!(app.mark_of(win, 0), Some(Mark::Acked));
    }
}
