use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const SECRET_LEN: usize = 16;
pub const MAX_TEXT_BYTES: usize = 160;

pub const PUBLIC_KEY: [u8; SECRET_LEN] = [
    0x8b, 0x33, 0x87, 0xe9, 0xc5, 0xcd, 0xea, 0x6a, 0xc9, 0xe5, 0xed, 0xba, 0xa1, 0x15, 0xcd, 0x72,
];

pub fn hashtag_secret(name: &str) -> [u8; SECRET_LEN] {
    let h = Sha256::digest(name.as_bytes());
    h[..SECRET_LEN].try_into().unwrap()
}

pub fn normalize_name(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('#') {
        raw.to_string()
    } else {
        format!("#{raw}")
    }
}

pub fn same_channel(a: &str, b: &str) -> bool {
    a.trim_start_matches('#').eq_ignore_ascii_case(b.trim_start_matches('#'))
}

pub fn is_public(name: &str) -> bool {
    name.eq_ignore_ascii_case("#public")
}

pub fn secret_for(name: &str) -> [u8; SECRET_LEN] {
    if is_public(name) {
        PUBLIC_KEY
    } else {
        hashtag_secret(name)
    }
}

/// Whether anyone who knows the channel name can derive `secret`: the well-known Public key, or
/// `sha256("#name")` (with or without the `#` / case as stored in the slot). Anything else is a
/// custom key, i.e. a private channel.
pub fn key_is_derived(name: &str, secret: &[u8; SECRET_LEN]) -> bool {
    let bare = name.trim().trim_start_matches('#');
    *secret == PUBLIC_KEY
        || [bare.to_string(), bare.to_lowercase()]
            .iter()
            .any(|b| hashtag_secret(&format!("#{b}")) == *secret)
}

#[derive(Debug, Default)]
pub struct SlotTable {
    pub max: u8,
    slots: BTreeMap<u8, String>,
    /// Slots whose secret is not derived from the name (`key_is_derived`).
    custom_key: BTreeSet<u8>,
}

impl SlotTable {
    pub fn new(max: u8) -> Self {
        Self { max, ..Self::default() }
    }

    pub fn set(&mut self, idx: u8, name: String) {
        self.custom_key.remove(&idx);
        if name.is_empty() {
            self.slots.remove(&idx);
        } else {
            self.slots.insert(idx, name);
        }
    }

    pub fn clear(&mut self, idx: u8) {
        self.slots.remove(&idx);
        self.custom_key.remove(&idx);
    }

    /// Record the slot's secret (after `set`), marking it private if not derived from the name.
    pub fn set_secret(&mut self, idx: u8, secret: &[u8; SECRET_LEN]) {
        match self.slots.get(&idx) {
            Some(name) if !key_is_derived(name, secret) => {
                self.custom_key.insert(idx);
            }
            _ => {
                self.custom_key.remove(&idx);
            }
        }
    }

    pub fn has_custom_key(&self, idx: u8) -> bool {
        self.custom_key.contains(&idx)
    }

    pub fn name(&self, idx: u8) -> Option<&str> {
        self.slots.get(&idx).map(String::as_str)
    }

    pub fn idx_of(&self, name: &str) -> Option<u8> {
        self.slots
            .iter()
            .find(|(_, n)| same_channel(n, name))
            .map(|(i, _)| *i)
    }

    pub fn free_slot(&self) -> Option<u8> {
        (0..self.max).find(|i| !self.slots.contains_key(i))
    }

    pub fn iter(&self) -> impl Iterator<Item = (u8, &str)> {
        self.slots.iter().map(|(i, n)| (*i, n.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashtag_secret_known_vector() {
        let s = hashtag_secret("#test");
        let full = Sha256::digest(b"#test");
        assert_eq!(&s[..], &full[..16]);
        assert_ne!(s, PUBLIC_KEY);
    }

    #[test]
    fn derived_and_custom_keys() {
        assert!(key_is_derived("Public", &PUBLIC_KEY));
        assert!(key_is_derived("#test", &hashtag_secret("#test")));
        assert!(key_is_derived("test", &hashtag_secret("#test")));
        assert!(key_is_derived("Hiking", &hashtag_secret("#hiking")));
        assert!(!key_is_derived("#test", &[7; SECRET_LEN]));
        let mut t = SlotTable::new(3);
        t.set(1, "#team".into());
        t.set_secret(1, &[7; SECRET_LEN]);
        assert!(t.has_custom_key(1));
        t.set(1, "#team".into());
        t.set_secret(1, &hashtag_secret("#team"));
        assert!(!t.has_custom_key(1));
        t.set_secret(1, &[7; SECRET_LEN]);
        t.clear(1);
        assert!(!t.has_custom_key(1));
    }

    #[test]
    fn normalize_adds_hash() {
        assert_eq!(normalize_name("foo"), "#foo");
        assert_eq!(normalize_name(" #foo "), "#foo");
    }

    #[test]
    fn slot_allocation() {
        let mut t = SlotTable::new(3);
        t.set(0, "#public".into());
        assert_eq!(t.free_slot(), Some(1));
        t.set(1, "#a".into());
        t.set(2, "#b".into());
        assert_eq!(t.free_slot(), None);
        assert_eq!(t.idx_of("#A"), Some(1));
        assert_eq!(t.idx_of("#Public"), Some(0));
        t.set(0, "Public".into());
        assert_eq!(t.idx_of("#public"), Some(0));
        t.clear(1);
        assert_eq!(t.free_slot(), Some(1));
    }
}
