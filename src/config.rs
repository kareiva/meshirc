use anyhow::{Context, Result};
use clap::Parser;
use directories::ProjectDirs;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "meshirc", about = "irssi-style TUI for MeshCore companion radios")]
pub struct Cli {
    #[arg(short, long)]
    pub port: Option<String>,
    #[arg(short, long)]
    pub baud: Option<u32>,
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub log_dir: Option<PathBuf>,
    #[arg(long)]
    pub history_lines: Option<usize>,
    /// Log private messages and restore private windows on startup (default true)
    #[arg(long, value_name = "BOOL")]
    pub save_private: Option<bool>,
    /// Serve a read-only web view of the channels on port 8667 (default false)
    #[arg(long, value_name = "BOOL")]
    pub http_server: Option<bool>,
    /// Also show private conversations in the web view (default false)
    #[arg(long, value_name = "BOOL")]
    pub http_private_windows: Option<bool>,
    /// Address for the web view: `host:port`, or a bare address/hostname for port 8667
    /// (default 0.0.0.0:8667, all interfaces)
    #[arg(long, value_name = "ADDR")]
    pub http_bind: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct FileConfig {
    port: Option<String>,
    baud: Option<u32>,
    log_dir: Option<PathBuf>,
    history_lines: Option<usize>,
    save_private: Option<bool>,
    http_server: Option<bool>,
    http_private_windows: Option<bool>,
    http_bind: Option<String>,
    auto_join: Option<Vec<String>>,
}

pub const DEFAULT_HTTP_PORT: u16 = 8667;

/// `host:port` as given, or `host` / bare IP (v4 or v6) with the default web port added.
pub fn bind_addr(s: &str) -> String {
    let s = s.trim();
    if let Ok(ip) = s.parse::<std::net::IpAddr>() {
        return std::net::SocketAddr::new(ip, DEFAULT_HTTP_PORT).to_string();
    }
    if s.contains(':') {
        s.to_string()
    } else {
        format!("{s}:{DEFAULT_HTTP_PORT}")
    }
}

pub const DEFAULT_PORT: &str = if cfg!(windows) { "COM3" } else { "/dev/ttyACM0" };

#[derive(Debug, Clone)]
pub struct Config {
    pub port: String,
    pub baud: u32,
    pub log_dir: PathBuf,
    pub data_dir: PathBuf,
    pub history_lines: usize,
    /// Log private messages to disk and reopen private windows on startup.
    pub save_private: bool,
    /// Run the read-only web view (`web.rs`).
    pub http_server: bool,
    /// Include private windows in the web view.
    pub http_private_windows: bool,
    /// Listen address for the web view, always with a port (see `bind_addr`).
    pub http_bind: String,
    pub auto_join: Vec<String>,
}

impl Config {
    pub fn load(cli: Cli) -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "meshirc").context("no home directory")?;
        let path = cli
            .config
            .clone()
            .unwrap_or_else(|| dirs.config_dir().join("config.toml"));
        let file: FileConfig = match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?,
            Err(_) => FileConfig::default(),
        };
        let data_dir = dirs.data_dir().to_path_buf();
        Ok(Config {
            port: cli.port.or(file.port).unwrap_or_else(|| DEFAULT_PORT.into()),
            baud: cli.baud.or(file.baud).unwrap_or(115200),
            log_dir: cli.log_dir.or(file.log_dir).unwrap_or_else(|| data_dir.join("logs")),
            data_dir,
            history_lines: cli.history_lines.or(file.history_lines).unwrap_or(200),
            save_private: cli.save_private.or(file.save_private).unwrap_or(true),
            http_server: cli.http_server.or(file.http_server).unwrap_or(false),
            http_private_windows: cli.http_private_windows.or(file.http_private_windows).unwrap_or(false),
            http_bind: bind_addr(cli.http_bind.or(file.http_bind).as_deref().unwrap_or("0.0.0.0")),
            auto_join: file.auto_join.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_addr_adds_default_port() {
        assert_eq!(bind_addr("0.0.0.0"), "0.0.0.0:8667");
        assert_eq!(bind_addr("127.0.0.1:9000"), "127.0.0.1:9000");
        assert_eq!(bind_addr("::1"), "[::1]:8667");
        assert_eq!(bind_addr("[::]:8080"), "[::]:8080");
        assert_eq!(bind_addr("localhost"), "localhost:8667");
        assert_eq!(bind_addr("localhost:1234"), "localhost:1234");
    }
}
