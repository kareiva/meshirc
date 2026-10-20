use crate::event::{AppEvent, RadioCmd, RadioReply};
use futures::StreamExt;
use meshcore_rs::events::{EventDispatcher, EventPayload};
use meshcore_rs::{BinaryReqType, EventType, MeshCore, PayloadType};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

type Cmds = Arc<Mutex<meshcore_rs::commands::CommandHandler>>;

pub fn spawn(port: String, baud: u32, tx: mpsc::Sender<AppEvent>, rx: mpsc::Receiver<RadioCmd>) {
    tokio::spawn(async move {
        let rx = Arc::new(Mutex::new(rx));
        loop {
            match run_connection(&port, baud, &tx, rx.clone()).await {
                Ok(()) => break,
                Err(e) => {
                    warn!("radio: {e}");
                    let _ = tx.send(AppEvent::Reply(RadioReply::Disconnected(e.to_string()))).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

async fn reply(tx: &mpsc::Sender<AppEvent>, r: RadioReply) {
    let _ = tx.send(AppEvent::Reply(r)).await;
}

async fn run_connection(
    port: &str,
    baud: u32,
    tx: &mpsc::Sender<AppEvent>,
    rx: Arc<Mutex<mpsc::Receiver<RadioCmd>>>,
) -> anyhow::Result<()> {
    let mc = MeshCore::serial(port, baud).await?;
    let cmds = mc.commands().clone();
    let me = cmds.lock().await.send_appstart().await?;
    let dev = cmds.lock().await.send_device_query().await?;
    info!("connected to {} fw {:?}", me.name, dev.version);
    reply(tx, RadioReply::Connected { me, dev: dev.clone() }).await;

    let max_channels = dev.max_channels.unwrap_or(8);
    for idx in 0..max_channels {
        match cmds.lock().await.get_channel(idx).await {
            Ok(ch) if !ch.name.is_empty() => {
                reply(tx, RadioReply::SlotInUse { idx, name: ch.name, secret: ch.secret }).await;
            }
            Ok(_) => {}
            Err(e) => warn!("get_channel {idx}: {e}"),
        }
    }

    match cmds.lock().await.get_contacts(0).await {
        Ok(list) => reply(tx, RadioReply::Contacts(list)).await,
        Err(e) => reply(tx, RadioReply::Error(format!("get_contacts: {e}"))).await,
    }
    if let Ok(b) = cmds.lock().await.get_bat().await {
        reply(tx, RadioReply::Battery { mv: b.battery_mv, pct: b.percentage() }).await;
    }
    reply(tx, RadioReply::Gps(gps_status(&cmds).await)).await;

    mc.start_auto_message_fetching().await;
    let mut events = mc.event_stream();
    let mut rx = rx.lock().await;

    loop {
        tokio::select! {
            ev = events.next() => {
                let Some(ev) = ev else { anyhow::bail!("event stream closed") };
                if ev.event_type == EventType::Disconnected {
                    anyhow::bail!("serial port closed");
                }
                let _ = tx.send(AppEvent::Radio(ev)).await;
            }
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                if matches!(cmd, RadioCmd::Shutdown) {
                    break;
                }
                handle_cmd(cmd, &cmds, mc.dispatcher(), tx).await;
            }
        }
    }
    let _ = mc.disconnect().await;
    Ok(())
}

async fn handle_cmd(cmd: RadioCmd, cmds: &Cmds, dispatcher: &Arc<EventDispatcher>, tx: &mpsc::Sender<AppEvent>) {
    match cmd {
        RadioCmd::SendDm { contact, text, window, line } => {
            match cmds.lock().await.send_msg(&contact, &text, None).await {
                Ok(info) => {
                    reply(tx, RadioReply::DmSent { window, line, tag: info.expected_ack, timeout_ms: info.suggested_timeout }).await
                }
                Err(e) => reply(tx, RadioReply::SendFailed { window, line, error: format!("send to {}: {e}", contact.adv_name) }).await,
            }
        }
        RadioCmd::SendChannel { idx, text, window, line } => {
            match cmds.lock().await.send_channel_msg(idx, &text, None).await {
                Ok(()) => reply(tx, RadioReply::ChannelSent { window, line }).await,
                Err(e) => reply(tx, RadioReply::SendFailed { window, line, error: format!("channel send: {e}") }).await,
            }
        }
        RadioCmd::JoinChannel { idx, name, secret } => {
            match cmds.lock().await.set_channel(idx, &name, &secret).await {
                Ok(()) => reply(tx, RadioReply::Joined { idx, name, secret }).await,
                Err(e) => reply(tx, RadioReply::Error(format!("join {name}: {e}"))).await,
            }
        }
        RadioCmd::PartChannel { idx, name } => {
            match cmds.lock().await.set_channel(idx, "", &[0u8; 16]).await {
                Ok(()) => reply(tx, RadioReply::Parted { idx, name }).await,
                Err(e) => reply(tx, RadioReply::Error(format!("part {name}: {e}"))).await,
            }
        }
        RadioCmd::Whois { contact, window } => {
            let cmds = cmds.clone();
            let dispatcher = dispatcher.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let name = contact.adv_name.clone();
                let sent = cmds.lock().await.send_binary_req(&contact, BinaryReqType::Status).await;
                let sent = match sent {
                    Ok(s) => s,
                    Err(e) => return reply(&tx, RadioReply::Error(format!("whois {name}: {e}"))).await,
                };
                let mut filters = HashMap::new();
                filters.insert("tag".to_string(), hex::encode(sent.expected_ack));
                let ev = dispatcher
                    .wait_for_event(Some(EventType::StatusResponse), filters, Duration::from_secs(20))
                    .await;
                match ev.map(|e| e.payload) {
                    Some(EventPayload::Status(status)) => reply(&tx, RadioReply::Status { window, name, status }).await,
                    _ => reply(&tx, RadioReply::Notice { window: Some(window), text: format!("{name}: no status reply (timeout)") }).await,
                }
            });
        }
        RadioCmd::Trace { name, path, flags, window } => {
            let cmds = cmds.clone();
            let dispatcher = dispatcher.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let r = match trace(&cmds, &dispatcher, &path, flags).await {
                    Ok(Some((snrs, final_snr))) => RadioReply::Trace { window, name, path, flags, snrs, final_snr },
                    Ok(None) => RadioReply::Notice { window: Some(window), text: format!("trace {name}: no reply (timeout)") },
                    Err(e) => RadioReply::Error(format!("trace {name}: {e}")),
                };
                reply(&tx, r).await;
            });
        }
        RadioCmd::RefreshContacts => match cmds.lock().await.get_contacts(0).await {
            Ok(list) => reply(tx, RadioReply::Contacts(list)).await,
            Err(e) => reply(tx, RadioReply::Error(format!("get_contacts: {e}"))).await,
        },
        RadioCmd::Advert { flood } => match cmds.lock().await.send_advert(flood).await {
            Ok(_) => reply(tx, RadioReply::Notice { window: None, text: if flood { "flood advert sent".into() } else { "advert sent".into() } }).await,
            Err(e) => reply(tx, RadioReply::Error(format!("advert: {e}"))).await,
        },
        RadioCmd::SetName(name) => match cmds.lock().await.set_name(&name).await {
            Ok(_) => reply(tx, RadioReply::Notice { window: None, text: format!("name set to {name}") }).await,
            Err(e) => reply(tx, RadioReply::Error(format!("set name: {e}"))).await,
        },
        RadioCmd::Battery => {
            if let Ok(b) = cmds.lock().await.get_bat().await {
                reply(tx, RadioReply::Battery { mv: b.battery_mv, pct: b.percentage() }).await;
            }
        }
        RadioCmd::Gps => reply(tx, RadioReply::Gps(gps_status(&cmds).await)).await,
        RadioCmd::Shutdown => {}
    }
}

/// Companion protocol v14+: `CMD_RUN_CLI_COMMAND` (66) runs a firmware CLI command and answers
/// with `RESP_CODE_CLI_REPLY` (29) + text. meshcore-rs 0.2 knows neither, so the reply surfaces as
/// `EventType::Unknown` with the raw frame bytes.
const CMD_RUN_CLI_COMMAND: u8 = 66;
const RESP_CODE_CLI_REPLY: u8 = 29;

async fn cli_command(cmds: &Cmds, text: &str) -> anyhow::Result<String> {
    let mut data = vec![CMD_RUN_CLI_COMMAND];
    data.extend_from_slice(text.as_bytes());
    let ev = cmds
        .lock()
        .await
        .send_multi(&data, &[EventType::Unknown, EventType::Error], Duration::from_secs(3))
        .await?;
    match ev.payload {
        EventPayload::Bytes(b) if b.first() == Some(&RESP_CODE_CLI_REPLY) => {
            Ok(String::from_utf8_lossy(&b[1..]).trim().to_string())
        }
        EventPayload::Bytes(b) => anyhow::bail!("unexpected frame {:02x?}", b.first()),
        other => anyhow::bail!("{other:?}"),
    }
}

/// `CMD_SEND_TRACE_PATH` (36): tag(4) auth(4) flags(1) path. meshcore-rs 0.2 has no call for
/// it and its `TraceData` parser doesn't match the firmware's `PUSH_CODE_TRACE_DATA` layout, so
/// the result is read from the trace packet's own `LogData` instead: every relay appends the SNR it
/// heard the packet at to the (unencrypted) path, and the copy we hear after the last hop carries
/// one SNR per hop. Returns those SNRs and our own receive SNR, or `None` on timeout.
const CMD_SEND_TRACE_PATH: u8 = 36;

async fn trace(
    cmds: &Cmds,
    dispatcher: &EventDispatcher,
    path: &[u8],
    flags: u8,
) -> anyhow::Result<Option<(Vec<f32>, f32)>> {
    let mut events = dispatcher.receiver();
    let tag = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u32).unwrap_or(0).to_le_bytes();
    let mut data = vec![CMD_SEND_TRACE_PATH];
    data.extend_from_slice(&tag);
    data.extend_from_slice(&[0; 4]);
    data.push(flags);
    data.extend_from_slice(path);
    let ev = cmds
        .lock()
        .await
        .send_multi(&data, &[EventType::MsgSent, EventType::Error], Duration::from_secs(5))
        .await?;
    let timeout = match ev.payload {
        EventPayload::MsgSent(info) => Duration::from_millis(info.suggested_timeout as u64) + Duration::from_secs(5),
        EventPayload::String(e) => anyhow::bail!("{e}"),
        other => anyhow::bail!("{other:?}"),
    };
    let hops = path.len() >> (flags & 3);
    let wait = async {
        loop {
            match events.recv().await {
                Ok(ev) => {
                    let EventPayload::LogData(log) = ev.payload else { continue };
                    let Some(h) = &log.header else { continue };
                    if h.payload_type == PayloadType::Trace && log.payload.starts_with(&tag) && h.path.len() >= hops {
                        let snrs = h.path[..hops].iter().map(|&b| b as i8 as f32 / 4.0).collect();
                        return Some((snrs, log.snr));
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    };
    Ok(tokio::time::timeout(timeout, wait).await.ok().flatten())
}

/// Same as meshcore-cli's `gps`: the firmware prints `on, {active|deactivated}, {fix|no fix}, N sats`
/// or `off`. Boards without GPS (or firmware without CLI passthrough) yield `None`.
async fn gps_status(cmds: &Cmds) -> Option<String> {
    match cli_command(cmds, "gps").await {
        Ok(s) if s.starts_with("on") || s == "off" => Some(s),
        Ok(s) => {
            info!("gps: unsupported ({s})");
            None
        }
        Err(e) => {
            info!("gps: {e}");
            None
        }
    }
}
