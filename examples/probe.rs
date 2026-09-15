//! Connects to the radio, dumps self info, channel slots and contacts, then prints raw events.
//! cargo run --example probe -- [/dev/ttyACM0] [seconds]

use futures::StreamExt;
use meshcore_rs::MeshCore;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::args().nth(1).unwrap_or_else(|| "/dev/ttyACM0".into());
    let secs: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mc = MeshCore::serial(&port, 115200).await?;
    let me = mc.commands().lock().await.send_appstart().await?;
    println!("self: {} pubkey={} freq={} sf={} tx={}", me.name, hex::encode(me.public_key), me.radio_freq, me.sf, me.tx_power);
    let dev = mc.commands().lock().await.send_device_query().await?;
    println!("device: {dev:?}");
    for idx in 0..dev.max_channels.unwrap_or(8) {
        match mc.commands().lock().await.get_channel(idx).await {
            Ok(ch) => println!("slot {idx}: {:?} secret={}", ch.name, hex::encode(ch.secret)),
            Err(e) => println!("slot {idx}: err {e}"),
        }
    }
    let contacts = mc.commands().lock().await.get_contacts(0).await?;
    println!("{} contacts", contacts.len());
    for c in &contacts {
        println!("  type={} {:<24} {} last={} path={}", c.contact_type, c.adv_name, c.prefix_hex(), c.last_advert, c.path_len);
    }
    let bat = mc.commands().lock().await.get_bat().await?;
    println!("battery {} mV", bat.battery_mv);
    mc.start_auto_message_fetching().await;
    let mut events = mc.event_stream();
    let deadline = tokio::time::sleep(Duration::from_secs(secs));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            Some(ev) = events.next() => println!("event {:?}: {:?}", ev.event_type, ev.payload),
            _ = &mut deadline => break,
        }
    }
    mc.disconnect().await?;
    Ok(())
}
