#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Join { name: String, key: Option<[u8; 16]> },
    Part(Option<String>),
    Msg { who: String, text: String },
    Query(String),
    Whois(String),
    Status(String),
    Contacts,
    Advert { flood: bool },
    Win(usize),
    Close,
    Nick(String),
    Help,
    Quit,
    Say(String),
    Unknown(String),
}

pub fn parse(line: &str) -> Option<Command> {
    let line = line.trim_end();
    if line.is_empty() {
        return None;
    }
    if !line.starts_with('/') || line.starts_with("//") {
        let text = line.strip_prefix('/').unwrap_or(line);
        return Some(Command::Say(text.to_string()));
    }
    let mut parts = line[1..].splitn(2, ' ');
    let cmd = parts.next().unwrap_or("").to_lowercase();
    let rest = parts.next().unwrap_or("").trim();
    let arg = |c: Command| if rest.is_empty() { Command::Unknown(format!("/{cmd} needs an argument")) } else { c };
    Some(match cmd.as_str() {
        "join" | "j" => {
            let mut p = rest.split_whitespace();
            let name = p.next().unwrap_or("").to_string();
            let key = p.next();
            if name.is_empty() {
                Command::Unknown("usage: /join <#name> [32-hex-char key]".into())
            } else {
                match key.map(parse_key) {
                    None => Command::Join { name, key: None },
                    Some(Some(k)) => Command::Join { name, key: Some(k) },
                    Some(None) => Command::Unknown("channel key must be 32 hex characters (128-bit)".into()),
                }
            }
        }
        "part" | "leave" => Command::Part(if rest.is_empty() { None } else { Some(rest.to_string()) }),
        "msg" | "m" => {
            let mut p = rest.splitn(2, ' ');
            let who = p.next().unwrap_or("").to_string();
            let text = p.next().unwrap_or("").trim().to_string();
            if who.is_empty() || text.is_empty() {
                Command::Unknown("usage: /msg <who> <text>".into())
            } else {
                Command::Msg { who, text }
            }
        }
        "query" | "q" => arg(Command::Query(rest.to_string())),
        "whois" | "wi" => arg(Command::Whois(rest.to_string())),
        "status" | "st" => arg(Command::Status(rest.to_string())),
        "contacts" | "who" => Command::Contacts,
        "advert" => Command::Advert { flood: rest.eq_ignore_ascii_case("flood") },
        "win" | "window" | "w" => match rest.parse() {
            Ok(n) => Command::Win(n),
            Err(_) => Command::Unknown("usage: /win <number>".into()),
        },
        "close" | "wc" => Command::Close,
        "nick" => arg(Command::Nick(rest.to_string())),
        "help" | "h" => Command::Help,
        "quit" | "exit" => Command::Quit,
        other => Command::Unknown(format!("unknown command: /{other}")),
    })
}

fn parse_key(s: &str) -> Option<[u8; 16]> {
    hex::decode(s).ok()?.try_into().ok()
}

pub const HELP: &[&str] = &[
    "/join #name [key]  join channel; key = 32 hex chars for private channels",
    "/part [#name]      leave channel and free its slot",
    "/msg <who> <text>  private message (name, name prefix or pubkey hex)",
    "/query <who>       open a private window",
    "/whois <who>       cached contact info",
    "/status <who>      request live status over the mesh (repeaters/rooms)",
    "/contacts          refresh contact list from radio",
    "/advert [flood]    send self advertisement",
    "/win N  /close     switch / close window (also Alt+N, Ctrl+N/P)",
    "/nick <name>       set radio node name",
    "/quit              exit",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(parse("/join #lt"), Some(Command::Join { name: "#lt".into(), key: None }));
        assert_eq!(parse("/j lt"), Some(Command::Join { name: "lt".into(), key: None }));
        assert_eq!(
            parse("/join Secret 8b3387e9c5cdea6ac9e5edbaa115cd72"),
            Some(Command::Join { name: "Secret".into(), key: Some(crate::channels::PUBLIC_KEY) })
        );
        assert!(matches!(parse("/join Secret abcd"), Some(Command::Unknown(_))));
        assert_eq!(parse("/part"), Some(Command::Part(None)));
        assert_eq!(
            parse("/msg Bob hi there"),
            Some(Command::Msg { who: "Bob".into(), text: "hi there".into() })
        );
        assert_eq!(parse("hello"), Some(Command::Say("hello".into())));
        assert_eq!(parse("//slashy"), Some(Command::Say("/slashy".into())));
        assert_eq!(parse("/win 3"), Some(Command::Win(3)));
        assert_eq!(parse("   "), None);
        assert!(matches!(parse("/msg Bob"), Some(Command::Unknown(_))));
        assert!(matches!(parse("/nope"), Some(Command::Unknown(_))));
    }
}
