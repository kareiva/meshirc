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

    pub fn remove(&mut self, key: &[u8; 32]) -> Option<Contact> {
        self.by_key.remove(key)
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

    /// Name of the node whose pubkey starts with the path hash `h`, if that is unambiguous
    /// (repeaters/rooms win over chat nodes, which don't relay).
    pub fn by_hash(&self, h: &[u8]) -> Option<&Contact> {
        let hits: Vec<&Contact> = self.by_key.values().filter(|c| c.public_key.starts_with(h)).collect();
        let relays: Vec<&Contact> = hits.iter().copied().filter(|c| c.contact_type != TYPE_CHAT).collect();
        match (relays.len(), hits.len()) {
            (1, _) => Some(relays[0]),
            (0, 1) => Some(hits[0]),
            _ => None,
        }
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

/// Route for a `/trace`, as sent in `CMD_SEND_TRACE_PATH`.
#[derive(Debug, PartialEq, Eq)]
pub struct TracePath {
    /// Hop hashes, `hash_size` bytes each.
    pub path: Vec<u8>,
    /// Trace flags: low 2 bits = log2(hash size).
    pub flags: u8,
    pub hash_size: usize,
    /// Whether the contact itself is on the path (repeaters/rooms). Chat nodes don't relay
    /// traces, so for them the trace turns around at the last repeater before them.
    pub reaches_contact: bool,
}

/// Round-trip trace route to `c` from its learned out path: `a b C b a` for a repeater/room,
/// `a b b a` for a chat node. `path_len` encodes the hop count in the low 6 bits and
/// hash size − 1 in the top 2 (0xff = flood, no known route).
pub fn trace_path(c: &Contact) -> Result<TracePath, String> {
    let name = &c.adv_name;
    let b = c.path_len as u8;
    if b == 0xff {
        return Err(format!("no known route to {name} (flood); a trace needs a direct path, learned when they reply"));
    }
    let hops = (b & 0x3f) as usize;
    let hash_size = (b >> 6) as usize + 1;
    let flags = match hash_size {
        1 => 0,
        2 => 1,
        4 => 2,
        n => return Err(format!("can't trace {n}-byte path hashes")),
    };
    let out = c
        .out_path
        .get(..hops * hash_size)
        .ok_or_else(|| format!("route to {name} is incomplete ({})", hex::encode(&c.out_path)))?;
    let reaches_contact = c.contact_type != TYPE_CHAT;
    if !reaches_contact && hops == 0 {
        return Err(format!("{name} is a direct neighbour and chat nodes don't relay traces; nothing to trace"));
    }
    let mut path = out.to_vec();
    if reaches_contact {
        path.extend_from_slice(&c.public_key[..hash_size]);
    }
    path.extend(out.chunks(hash_size).rev().flatten());
    Ok(TracePath { path, flags, hash_size, reaches_contact })
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

    #[test]
    fn trace_paths() {
        let mut c = mk("Rpt", 0xc3);
        c.contact_type = TYPE_REPEATER;
        assert!(trace_path(&c).unwrap_err().contains("flood"));
        c.path_len = 0;
        assert_eq!(trace_path(&c).unwrap().path, vec![0xc3]);
        c.path_len = 2;
        c.out_path = vec![0xa1, 0xb2];
        let t = trace_path(&c).unwrap();
        assert_eq!((t.path, t.flags, t.reaches_contact), (vec![0xa1, 0xb2, 0xc3, 0xb2, 0xa1], 0, true));
        // 2-byte hashes: top bits of path_len = 1
        c.path_len = 0x42;
        c.out_path = vec![0xa1, 0xa2, 0xb1, 0xb2];
        let t = trace_path(&c).unwrap();
        assert_eq!((t.path, t.flags), (vec![0xa1, 0xa2, 0xb1, 0xb2, 0xc3, 0xab, 0xb1, 0xb2, 0xa1, 0xa2], 1));
        // chat node: turn around at the last repeater
        c.contact_type = TYPE_CHAT;
        c.path_len = 2;
        c.out_path = vec![0xa1, 0xb2];
        let t = trace_path(&c).unwrap();
        assert_eq!((t.path, t.reaches_contact), (vec![0xa1, 0xb2, 0xb2, 0xa1], false));
        c.path_len = 0;
        assert!(trace_path(&c).unwrap_err().contains("direct neighbour"));
        // out_path truncated by a 0x00 hash byte
        c.path_len = 3;
        assert!(trace_path(&c).unwrap_err().contains("incomplete"));
    }
}
