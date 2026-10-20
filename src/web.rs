//! Read-only web view (`--http-server true`): thelounge-style tabs of the channel windows
//! (and private windows with `--http-private-windows true`). A tiny HTTP/1.1 server on tokio;
//! the page polls `/api/state`, which the main loop answers via `AppEvent::WebSnapshot`, so
//! `App` stays the only owner of the state.

use crate::event::{AppEvent, RadioReply};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

const INDEX: &str = include_str!("web/index.html");

pub fn spawn(tx: mpsc::Sender<AppEvent>, bind: String, private: bool) {
    tokio::spawn(async move {
        let listener = match TcpListener::bind(&bind).await {
            Ok(l) => l,
            Err(e) => {
                warn!("http: bind {bind}: {e}");
                let _ = tx.send(AppEvent::Reply(RadioReply::Error(format!("web view: can't listen on {bind}: {e}")))).await;
                return;
            }
        };
        let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or(bind);
        info!("http: serving on {addr}");
        let what = if private { "channels and private windows" } else { "channels" };
        let text = format!("web view ({what}, read-only) on http://{addr}/");
        let _ = tx.send(AppEvent::Reply(RadioReply::Notice { window: Some(0), text })).await;
        loop {
            match listener.accept().await {
                Ok((sock, _)) => {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = tokio::time::timeout(Duration::from_secs(10), serve(sock, tx)).await {
                            warn!("http: {e}");
                        }
                    });
                }
                Err(e) => warn!("http: accept: {e}"),
            }
        }
    });
}

async fn serve(mut sock: TcpStream, tx: mpsc::Sender<AppEvent>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if buf.len() > 16 * 1024 {
            return;
        }
    }
    let req = String::from_utf8_lossy(&buf);
    let mut first = req.lines().next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let path = target.split('?').next().unwrap_or("");
    let etag_in = req
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("if-none-match")).map(|(_, v)| v.trim()));

    let resp = match (method, path) {
        ("GET" | "HEAD", "/") => response("200 OK", "text/html; charset=utf-8", INDEX, None),
        ("GET" | "HEAD", "/api/state") => {
            let (rtx, rrx) = oneshot::channel();
            if tx.send(AppEvent::WebSnapshot(rtx)).await.is_err() {
                return;
            }
            match rrx.await {
                Ok(body) => {
                    let mut h = DefaultHasher::new();
                    body.hash(&mut h);
                    let etag = format!("\"{:016x}\"", h.finish());
                    if etag_in == Some(etag.as_str()) {
                        response("304 Not Modified", "application/json", "", Some(&etag))
                    } else {
                        response("200 OK", "application/json", &body, Some(&etag))
                    }
                }
                Err(_) => response("503 Service Unavailable", "text/plain", "shutting down\n", None),
            }
        }
        ("GET" | "HEAD", _) => response("404 Not Found", "text/plain", "not found\n", None),
        _ => response("405 Method Not Allowed", "text/plain", "read-only\n", None),
    };
    let resp = if method == "HEAD" { head_only(resp) } else { resp };
    let _ = sock.write_all(&resp).await;
    let _ = sock.shutdown().await;
}

fn response(status: &str, ctype: &str, body: &str, etag: Option<&str>) -> Vec<u8> {
    let etag = etag.map(|e| format!("ETag: {e}\r\n")).unwrap_or_default();
    let mut r = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n{etag}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    r.extend_from_slice(body.as_bytes());
    r
}

fn head_only(mut resp: Vec<u8>) -> Vec<u8> {
    if let Some(end) = resp.windows(4).position(|w| w == b"\r\n\r\n") {
        resp.truncate(end + 4);
    }
    resp
}
