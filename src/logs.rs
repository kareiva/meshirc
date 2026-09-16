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

/// Private-window logs are named `<name>_<12 hex chars of pubkey prefix>`.
/// Returns the name and prefix when `log_name` follows that pattern.
pub fn parse_query_log(log_name: &str) -> Option<(String, [u8; 6])> {
    let (name, hex_prefix) = log_name.rsplit_once('_')?;
    if name.is_empty() || hex_prefix.len() != 12 {
        return None;
    }
    let prefix: [u8; 6] = hex::decode(hex_prefix).ok()?.try_into().ok()?;
    Some((name.to_string(), prefix))
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

    /// Names (without `.log`) of every log file on disk.
    pub fn list(&self) -> Vec<String> {
        let Ok(rd) = fs::read_dir(&self.dir) else { return vec![] };
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|f| f.strip_suffix(".log").map(String::from))
            .collect();
        names.sort();
        names
    }

    /// Delete the log file for `window`; returns whether a file was removed.
    pub fn remove(&mut self, window: &str) -> bool {
        // The file may have been opened under a different (unsanitised) window
        // name, so drop every cached handle pointing at the same path.
        let path = self.path(window);
        let dir = self.dir.clone();
        self.files.retain(|w, _| dir.join(format!("{}.log", safe_name(w))) != path);
        fs::remove_file(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_log_names() {
        let (name, prefix) = parse_query_log("Justinas_2280c4b5d081").unwrap();
        assert_eq!(name, "Justinas");
        assert_eq!(prefix, [0x22, 0x80, 0xc4, 0xb5, 0xd0, 0x81]);
        let (name, _) = parse_query_log("Some_Body_2280c4b5d081").unwrap();
        assert_eq!(name, "Some_Body");
        assert!(parse_query_log("#lietuva").is_none());
        assert!(parse_query_log("Public").is_none());
        assert!(parse_query_log("x_notahexstring").is_none());
        assert!(parse_query_log("_2280c4b5d081").is_none());
    }

    #[test]
    fn list_and_remove() {
        let dir = std::env::temp_dir().join(format!("meshirc-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut store = LogStore::new(&dir).unwrap();
        store.append("#a", "one");
        store.append("Bob Smith_2280c4b5d081", "two");
        assert_eq!(store.list(), vec!["#a".to_string(), "Bob_Smith_2280c4b5d081".to_string()]);
        assert!(store.remove("Bob Smith_2280c4b5d081"));
        assert!(!store.remove("Bob Smith_2280c4b5d081"));
        assert_eq!(store.list(), vec!["#a".to_string()]);
        // Appending after a wipe starts a fresh file.
        store.append("#a", "three");
        assert!(store.remove("#a"));
        assert!(store.tail("#a", 10).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
