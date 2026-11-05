//! An in-memory ManageSieve server for tests. It runs over a `tokio::io::duplex`
//! pipe, not a loopback socket, and keeps scripts in a shared state the test
//! can inspect afterwards.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};

use super::managesieve::{BoxStream, ManageSieveClient};

#[derive(Debug, Clone)]
pub struct FakeState {
    pub extensions: Vec<&'static str>,
    pub version: bool,
    pub scripts: BTreeMap<String, String>,
    pub active: Option<String>,
    pub username: String,
    pub password: String,
    /// Every command verb the client sent, in order.
    pub log: Vec<String>,
    /// HAVESPACE refuses scripts larger than this.
    pub quota: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            extensions: vec![
                "fileinto",
                "imap4flags",
                "vacation",
                "body",
                "include",
                "date",
                "relational",
            ],
            version: true,
            scripts: BTreeMap::new(),
            active: None,
            username: "user".into(),
            password: "pass".into(),
            log: Vec::new(),
            quota: 64 * 1024,
        }
    }
}

pub type Shared = Arc<Mutex<FakeState>>;

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn literal(value: &str) -> String {
    format!("{{{}}}\r\n{value}", value.len())
}

fn capabilities(state: &FakeState) -> String {
    let mut out = String::new();
    out.push_str("\"IMPLEMENTATION\" \"Fake Sieve\"\r\n");
    out.push_str(&format!(
        "\"SIEVE\" {}\r\n",
        quote(&state.extensions.join(" "))
    ));
    out.push_str("\"SASL\" \"PLAIN LOGIN\"\r\n");
    if state.version {
        out.push_str("\"VERSION\" \"1.0\"\r\n");
    }
    out.push_str("OK \"ready\"\r\n");
    out
}

/// Read one command: atoms and strings, literals included.
async fn read_command(reader: &mut BufReader<DuplexStream>) -> Option<Vec<String>> {
    let mut args = Vec::new();
    loop {
        let mut line = Vec::new();
        if reader.read_until(b'\n', &mut line).await.ok()? == 0 {
            return None;
        }
        let text = String::from_utf8(line).ok()?;
        let text = text.trim_end_matches(['\r', '\n']);
        let mut rest = text;
        let mut literal_len = None;
        while !rest.is_empty() {
            rest = rest.trim_start();
            if let Some(stripped) = rest.strip_prefix('"') {
                let mut value = String::new();
                let mut chars = stripped.char_indices();
                let mut end = stripped.len();
                while let Some((index, ch)) = chars.next() {
                    match ch {
                        '\\' => {
                            if let Some((_, next)) = chars.next() {
                                value.push(next);
                            }
                        }
                        '"' => {
                            end = index + 1;
                            break;
                        }
                        _ => value.push(ch),
                    }
                }
                args.push(value);
                rest = &stripped[end..];
            } else if rest.starts_with('{') && rest.ends_with("+}") {
                literal_len = rest[1..rest.len() - 2].parse::<usize>().ok();
                rest = "";
            } else {
                let end = rest.find(' ').unwrap_or(rest.len());
                args.push(rest[..end].to_string());
                rest = &rest[end..];
            }
        }
        match literal_len {
            Some(length) => {
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).await.ok()?;
                args.push(String::from_utf8(bytes).ok()?);
            }
            None => return Some(args),
        }
    }
}

async fn serve(stream: DuplexStream, state: Shared) {
    let mut reader = BufReader::new(stream);
    let greeting = capabilities(&state.lock().unwrap());
    if reader
        .get_mut()
        .write_all(greeting.as_bytes())
        .await
        .is_err()
    {
        return;
    }
    while let Some(args) = read_command(&mut reader).await {
        let Some(verb) = args.first().map(|verb| verb.to_ascii_uppercase()) else {
            continue;
        };
        let reply = {
            let mut state = state.lock().unwrap();
            state.log.push(verb.clone());
            handle(&mut state, &verb, &args[1..])
        };
        if reader.get_mut().write_all(reply.as_bytes()).await.is_err() {
            return;
        }
        if verb == "LOGOUT" {
            return;
        }
    }
}

fn handle(state: &mut FakeState, verb: &str, args: &[String]) -> String {
    let arg = |index: usize| args.get(index).cloned().unwrap_or_default();
    match verb {
        "CAPABILITY" => capabilities(state),
        "AUTHENTICATE" => {
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(arg(1))
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            if decoded == format!("\0{}\0{}", state.username, state.password) {
                "OK \"authenticated\"\r\n".into()
            } else {
                "NO (AUTH-TOO-WEAK) \"bad credentials\"\r\n".into()
            }
        }
        "LISTSCRIPTS" => {
            let mut out = String::new();
            for name in state.scripts.keys() {
                out.push_str(&literal(name));
                if state.active.as_deref() == Some(name.as_str()) {
                    out.push_str(" ACTIVE");
                }
                out.push_str("\r\n");
            }
            out.push_str("OK\r\n");
            out
        }
        "GETSCRIPT" => match state.scripts.get(&arg(0)) {
            Some(content) => format!("{}\r\nOK\r\n", literal(content)),
            None => "NO (NONEXISTENT) \"no such script\"\r\n".into(),
        },
        "HAVESPACE" => {
            let size: usize = arg(1).parse().unwrap_or(usize::MAX);
            if size > state.quota {
                "NO (QUOTA/MAXSIZE) \"too big\"\r\n".into()
            } else {
                "OK\r\n".into()
            }
        }
        "CHECKSCRIPT" | "PUTSCRIPT" => {
            let content = if verb == "PUTSCRIPT" { arg(1) } else { arg(0) };
            if content.contains("SYNTAX-ERROR") {
                return format!("NO {}\r\n", literal("line 1: syntax error"));
            }
            if verb == "PUTSCRIPT" {
                state.scripts.insert(arg(0), content);
            }
            "OK\r\n".into()
        }
        "SETACTIVE" => {
            let name = arg(0);
            if name.is_empty() {
                state.active = None;
                "OK\r\n".into()
            } else if state.scripts.contains_key(&name) {
                state.active = Some(name);
                "OK\r\n".into()
            } else {
                "NO (NONEXISTENT) \"no such script\"\r\n".into()
            }
        }
        "DELETESCRIPT" => {
            let name = arg(0);
            if state.active.as_deref() == Some(name.as_str()) {
                "NO (ACTIVE) \"script is active\"\r\n".into()
            } else if state.scripts.remove(&name).is_some() {
                "OK\r\n".into()
            } else {
                "NO (NONEXISTENT) \"no such script\"\r\n".into()
            }
        }
        "LOGOUT" => "OK \"bye\"\r\n".into(),
        _ => "NO \"unknown command\"\r\n".into(),
    }
}

/// A logged-in client talking to a fresh fake server over `state`.
pub async fn connected(state: Shared) -> ManageSieveClient {
    let (client_end, server_end) = tokio::io::duplex(1 << 20);
    tokio::spawn(serve(server_end, state.clone()));
    let stream: BoxStream = Box::new(client_end);
    // The pipe stands in for an encrypted stream.
    let mut client = ManageSieveClient::from_stream(stream, true)
        .await
        .expect("greeting");
    let (username, password) = {
        let state = state.lock().unwrap();
        (state.username.clone(), state.password.clone())
    };
    client
        .authenticate(&username, &password)
        .await
        .expect("login");
    client
}

pub fn shared(state: FakeState) -> Shared {
    Arc::new(Mutex::new(state))
}
