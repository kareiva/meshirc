use meshcore_rs::events::Contact;
use std::collections::HashMap;

pub const TYPE_CHAT: u8 = 1;
pub const TYPE_REPEATER: u8 = 2;
pub const TYPE_ROOM: u8 = 3;

pub fn type_icon(t: u8) -> &'static str {
    match t {
        TYPE_CHAT => "●",
        TYPE_REPEATER => "▲",
        TYPE_ROOM => "■",
        _ => "?",
    }
}

pub fn type_name(t: u8) -> &'static str {
    match t {
        TYPE_CHAT => "chat",
        TYPE_REPEATER => "repeater",
        TYPE_ROOM => "room",
        _ => "unknown",
    }
}

#[derive(Debug)]
pub enum Resolve<'a> {
    One(&'a Contact),
    Ambiguous(Vec<&'a Contact>),
    None,
}

#[derive(Default)]
pub struct ContactBook {
    by_key: HashMap<[u8; 32], Contact>,
    last_seen: HashMap<[u8; 6], u32>,
}

impl ContactBook {
    pub fn upsert(&mut self, c: Contact) {
        self.by_key.insert(c.public_key, c);
    }

    pub fn replace_all(&mut self, list: Vec<Contact>) {
        self.by_key.clear();
        for c in list {
            self.upsert(c);
        }
    }

    pub fn touch(&mut self, prefix: &[u8; 6], now: u32) {
        self.last_seen.insert(*prefix, now);
    }

    pub fn by_prefix(&self, prefix: &[u8]) -> Option<&Contact> {
        self.by_key.values().find(|c| c.public_key.starts_with(prefix))
    }

    pub fn last_advert(&self, c: &Contact) -> u32 {
        self.last_seen
            .get(&c.prefix())
            .copied()
            .unwrap_or(c.last_advert)
            .max(c.last_advert)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn sorted(&self) -> Vec<&Contact> {
        let mut v: Vec<&Contact> = self.by_key.values().collect();
        v.sort_by(|a, b| {
            self.last_advert(b)
                .cmp(&self.last_advert(a))
                .then_with(|| a.adv_name.to_lowercase().cmp(&b.adv_name.to_lowercase()))
        });
        v
    }

    pub fn resolve(&self, query: &str) -> Resolve<'_> {
        let q = query.trim();
        if q.is_empty() {
            return Resolve::None;
        }
        if let Some(c) = self.by_key.values().find(|c| c.adv_name == q) {
            return Resolve::One(c);
        }
        let ql = q.to_lowercase();
        let mut hits: Vec<&Contact> = self
            .by_key
            .values()
            .filter(|c| c.adv_name.to_lowercase().starts_with(&ql))
            .collect();
        if hits.is_empty() && ql.len() >= 4 && ql.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(bytes) = hex::decode(if ql.len() % 2 == 1 { format!("{ql}0") } else { ql.clone() }) {
                let n = ql.len() / 2;
                hits = self
                    .by_key
                    .values()
                    .filter(|c| c.public_key.starts_with(&bytes[..n]))
                    .collect();
            }
        }
        match hits.len() {
            0 => Resolve::None,
            1 => Resolve::One(hits[0]),
            _ => Resolve::Ambiguous(hits),
        }
    }

    pub fn complete(&self, prefix: &str) -> Vec<String> {
        let p = prefix.to_lowercase();
        let mut v: Vec<String> = self
            .by_key
            .values()
            .filter(|c| c.adv_name.to_lowercase().starts_with(&p))
            .map(|c| c.adv_name.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

pub fn age(now: u32, then: u32) -> String {
    if then == 0 {
        return "-".into();
    }
    if now < then {
        return "now".into();
    }
    let d = now - then;
    if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(name: &str, key0: u8) -> Contact {
        let mut public_key = [0u8; 32];
        public_key[0] = key0;
        public_key[1] = 0xab;
        Contact {
            public_key,
            contact_type: TYPE_CHAT,
            flags: 0,
            path_len: -1,
            out_path: vec![],
            adv_name: name.into(),
            last_advert: 0,
            adv_lat: 0,
            adv_lon: 0,
            last_modification_timestamp: 0,
        }
    }

    #[test]
    fn resolves_exact_prefix_and_hex() {
        let mut b = ContactBook::default();
        b.upsert(mk("Alice Node", 0x01));
        b.upsert(mk("Alicia", 0x02));
        b.upsert(mk("Bob", 0x03));
        assert!(matches!(b.resolve("Bob"), Resolve::One(c) if c.adv_name == "Bob"));
        assert!(matches!(b.resolve("ali"), Resolve::Ambiguous(v) if v.len() == 2));
        assert!(matches!(b.resolve("alice"), Resolve::One(c) if c.adv_name == "Alice Node"));
        assert!(matches!(b.resolve("03ab"), Resolve::One(c) if c.adv_name == "Bob"));
        assert!(matches!(b.resolve("zzz"), Resolve::None));
    }
}
