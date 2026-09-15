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
}

#[derive(Deserialize, Debug, Default)]
struct FileConfig {
    port: Option<String>,
    baud: Option<u32>,
    log_dir: Option<PathBuf>,
    history_lines: Option<usize>,
    auto_join: Option<Vec<String>>,
}

pub const DEFAULT_PORT: &str = if cfg!(windows) { "COM3" } else { "/dev/ttyACM0" };

#[derive(Debug, Clone)]
pub struct Config {
    pub port: String,
    pub baud: u32,
    pub log_dir: PathBuf,
    pub data_dir: PathBuf,
    pub history_lines: usize,
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
            auto_join: file.auto_join.unwrap_or_default(),
        })
    }
}
