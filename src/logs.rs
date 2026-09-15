use anyhow::Result;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct LogStore {
    dir: PathBuf,
    files: HashMap<String, File>,
}

fn safe_name(window: &str) -> String {
    window
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '#' || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

impl LogStore {
    pub fn new(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self { dir: dir.to_path_buf(), files: HashMap::new() })
    }

    fn path(&self, window: &str) -> PathBuf {
        self.dir.join(format!("{}.log", safe_name(window)))
    }

    pub fn append(&mut self, window: &str, line: &str) {
        let path = self.path(window);
        let f = self.files.entry(window.to_string()).or_insert_with(|| {
            OpenOptions::new().create(true).append(true).open(&path).expect("open log")
        });
        let _ = writeln!(f, "{line}");
    }

    pub fn tail(&self, window: &str, n: usize) -> Vec<String> {
        let Ok(f) = File::open(self.path(window)) else { return vec![] };
        let lines: Vec<String> = BufReader::new(f).lines().map_while(Result::ok).collect();
        let start = lines.len().saturating_sub(n);
        lines[start..].to_vec()
    }
}
