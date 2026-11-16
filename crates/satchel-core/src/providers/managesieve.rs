//! ManageSieve (RFC 5804): the protocol for storing Sieve scripts on a mail
//! server, so filters and the vacation reply run while Satchel is closed.
//!
//! The client speaks to any byte stream; [`connect`] opens the real one (TCP,
//! then STARTTLS or implicit TLS through the same `rustls` configuration IMAP
//! uses). Tests drive it over an in-memory duplex instead of a loopback socket.
//!
//! Credentials are only ever sent over TLS: a server that offers no STARTTLS
//! on a plain connection is refused, not logged into in the clear.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

/// The IANA port for ManageSieve.
pub const DEFAULT_PORT: u16 = 4190;

/// Longest string or literal accepted from a server. A Sieve script is a few
/// kilobytes; servers cap them far below this.
const MAX_SERVER_STRING: usize = 4 * 1024 * 1024;
/// Longest quoted string a client may send (RFC 5804 section 4).
const MAX_CLIENT_QUOTED: usize = 1024;
/// Most lines a single response may carry before it counts as a runaway.
const MAX_RESPONSE_LINES: usize = 10_000;

const OP_TIMEOUT: Duration = Duration::from_secs(20);

/// How the connection is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SieveTls {
    /// Plain TCP, upgraded with STARTTLS before anything else (RFC 5804 default).
    Starttls,
    /// TLS from the first byte, for servers configured that way.
    Implicit,
}

impl SieveTls {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starttls => "starttls",
            Self::Implicit => "tls",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "starttls" => Some(Self::Starttls),
            "tls" | "implicit" | "ssl" => Some(Self::Implicit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SieveEndpoint {
    pub host: String,
    pub port: u16,
    pub tls: SieveTls,
}

/// What the server says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub implementation: Option<String>,
    /// Sieve extensions (`fileinto`, `vacation`, `imap4flags`, …), lower case.
    pub sieve: Vec<String>,
    /// SASL mechanisms, upper case.
    pub sasl: Vec<String>,
    pub starttls: bool,
    /// Present on RFC 5804 servers; older ones (pre-RFC drafts) leave it out
    /// and do not know CHECKSCRIPT.
    pub version: Option<String>,
    pub owner: Option<String>,
}

impl Capabilities {
    pub fn has_extension(&self, name: &str) -> bool {
        self.sieve.iter().any(|ext| ext.eq_ignore_ascii_case(name))
    }

    fn has_mechanism(&self, name: &str) -> bool {
        self.sasl.iter().any(|mech| mech.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInfo {
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bye,
}

/// The final line of a response: `OK`, `NO` or `BYE`, an optional response
/// code in parentheses and optional human-readable text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerResponse {
    pub status: Status,
    /// Upper-cased response code atom, e.g. `QUOTA/MAXSIZE`, `NONEXISTENT`.
    pub code: Option<String>,
    /// The string argument some codes carry (`SASL`, `TAG`).
    pub code_arg: Option<String>,
    pub text: Option<String>,
}

impl std::fmt::Display for ServerResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.code, &self.text) {
            (Some(code), Some(text)) => write!(f, "({code}) {text}"),
            (Some(code), None) => write!(f, "({code})"),
            (None, Some(text)) => f.write_str(text),
            (None, None) => f.write_str("no detail"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SieveError {
    #[error("managesieve connect failed: {0}")]
    Connect(String),
    #[error("managesieve tls failed: {0}")]
    Tls(String),
    #[error("managesieve server offers no STARTTLS")]
    NoStartTls,
    #[error("managesieve server offers no supported SASL mechanism")]
    NoMechanism,
    #[error("managesieve refuses to authenticate without TLS")]
    Insecure,
    #[error("managesieve authentication failed: {0}")]
    AuthFailed(ServerResponse),
    #[error("managesieve server refused: {0}")]
    Refused(ServerResponse),
    #[error("managesieve server closed the connection: {0}")]
    Bye(ServerResponse),
    #[error("managesieve protocol error: {0}")]
    Protocol(String),
    #[error("managesieve io failed: {0}")]
    Io(String),
    #[error("managesieve timed out")]
    Timeout,
}

impl SieveError {
    /// The response code of a refusal, if the server gave one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused(response) | Self::AuthFailed(response) | Self::Bye(response) => {
                response.code.as_deref()
            }
            _ => None,
        }
    }

    /// Message key for the UI; Rust never hands out sentences.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Connect(_) | Self::Timeout | Self::Io(_) => {
                "backend.errors.serverRules.unreachable"
            }
            Self::Tls(_) | Self::NoStartTls | Self::Insecure => "backend.errors.serverRules.tls",
            Self::NoMechanism => "backend.errors.serverRules.noMechanism",
            Self::AuthFailed(_) => "backend.errors.serverRules.authFailed",
            Self::Refused(response)
                if response
                    .code
                    .as_deref()
                    .is_some_and(|code| code.starts_with("QUOTA")) =>
            {
                "backend.errors.serverRules.quota"
            }
            Self::Refused(_) => "backend.errors.serverRules.refused",
            Self::Bye(_) | Self::Protocol(_) => "backend.errors.serverRules.protocol",
        }
    }
}

fn io_error(error: std::io::Error) -> SieveError {
    SieveError::Io(error.to_string())
}

pub trait SieveStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> SieveStream for T {}

pub type BoxStream = Box<dyn SieveStream>;

/// One lexical item of a server line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Atom(String),
    Str(String),
    LParen,
    RParen,
    Eol,
}

/// A connected ManageSieve session.
pub struct ManageSieveClient {
    reader: BufReader<BoxStream>,
    capabilities: Capabilities,
    secure: bool,
}

impl ManageSieveClient {
    /// Read the greeting (the capability list) from a fresh stream. `secure`
    /// says whether the stream is already encrypted.
    pub async fn from_stream(stream: BoxStream, secure: bool) -> Result<Self, SieveError> {
        let mut client = Self {
            reader: BufReader::new(stream),
            capabilities: Capabilities::default(),
            secure,
        };
        client.capabilities = client.read_capabilities().await?;
        Ok(client)
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// Upgrade with STARTTLS. `upgrade` performs the handshake on the raw
    /// stream; afterwards the server re-sends its capabilities (RFC 5804 2.2),
    /// which replace the plaintext ones — those were unauthenticated.
    pub async fn starttls<F, Fut>(self, upgrade: F) -> Result<Self, SieveError>
    where
        F: FnOnce(BoxStream) -> Fut,
        Fut: std::future::Future<Output = Result<BoxStream, SieveError>>,
    {
        if !self.capabilities.starttls {
            return Err(SieveError::NoStartTls);
        }
        let mut client = self;
        client.write_all(b"STARTTLS\r\n").await?;
        let (_, response) = client.read_response().await?;
        client.expect_ok(response)?;
        // Anything the server sent after its OK arrived in the clear and
        // would otherwise be read as if it came through TLS.
        if !client.reader.buffer().is_empty() {
            return Err(SieveError::Protocol(
                "data after STARTTLS response".to_string(),
            ));
        }
        let raw = client.reader.into_inner();
        let upgraded = upgrade(raw).await?;
        Self::from_stream(upgraded, true).await
    }

    /// `CAPABILITY`: ask again, e.g. after authentication.
    pub async fn refresh_capabilities(&mut self) -> Result<&Capabilities, SieveError> {
        self.write_all(b"CAPABILITY\r\n").await?;
        self.capabilities = self.read_capabilities().await?;
        Ok(&self.capabilities)
    }

    /// Authenticate with the account's password: PLAIN when offered, else LOGIN.
    pub async fn authenticate(&mut self, username: &str, password: &str) -> Result<(), SieveError> {
        if !self.secure {
            return Err(SieveError::Insecure);
        }
        let b64 = base64::engine::general_purpose::STANDARD;
        if self.capabilities.has_mechanism("PLAIN") {
            let initial = b64.encode(format!("\0{username}\0{password}"));
            let command = format!("AUTHENTICATE \"PLAIN\" {}\r\n", quote(&initial));
            self.write_all(command.as_bytes()).await?;
            return self.finish_sasl(&[]).await;
        }
        if self.capabilities.has_mechanism("LOGIN") {
            self.write_all(b"AUTHENTICATE \"LOGIN\"\r\n").await?;
            let answers = [b64.encode(username), b64.encode(password)];
            return self.finish_sasl(&answers).await;
        }
        Err(SieveError::NoMechanism)
    }

    /// Answer server challenges in order until the final response.
    async fn finish_sasl(&mut self, answers: &[String]) -> Result<(), SieveError> {
        let mut answers = answers.iter();
        loop {
            let line = self.read_line_tokens().await?;
            match line.first() {
                Some(Token::Str(_)) => {
                    let Some(answer) = answers.next() else {
                        // An unexpected challenge: cancel the exchange.
                        self.write_all(b"\"*\"\r\n").await?;
                        let response = self.read_final_after_cancel().await?;
                        return Err(SieveError::AuthFailed(response));
                    };
                    self.write_all(format!("{}\r\n", quote(answer)).as_bytes())
                        .await?;
                }
                _ => {
                    let response = parse_final_line(&line)?;
                    return match response.status {
                        Status::Ok => Ok(()),
                        Status::No => Err(SieveError::AuthFailed(response)),
                        Status::Bye => Err(SieveError::Bye(response)),
                    };
                }
            }
        }
    }

    async fn read_final_after_cancel(&mut self) -> Result<ServerResponse, SieveError> {
        let (_, response) = self.read_response().await?;
        Ok(response)
    }

    pub async fn list_scripts(&mut self) -> Result<Vec<ScriptInfo>, SieveError> {
        self.write_all(b"LISTSCRIPTS\r\n").await?;
        let (lines, response) = self.read_response().await?;
        self.expect_ok(response)?;
        let mut scripts = Vec::new();
        for line in lines {
            let mut tokens = line.into_iter();
            let Some(Token::Str(name)) = tokens.next() else {
                return Err(SieveError::Protocol(
                    "LISTSCRIPTS line without a name".into(),
                ));
            };
            let active = matches!(tokens.next(), Some(Token::Atom(atom)) if atom.eq_ignore_ascii_case("ACTIVE"));
            scripts.push(ScriptInfo { name, active });
        }
        Ok(scripts)
    }

    pub async fn get_script(&mut self, name: &str) -> Result<String, SieveError> {
        self.send_command("GETSCRIPT", &[name]).await?;
        let (lines, response) = self.read_response().await?;
        self.expect_ok(response)?;
        lines
            .into_iter()
            .flatten()
            .find_map(|token| match token {
                Token::Str(content) => Some(content),
                _ => None,
            })
            .ok_or_else(|| SieveError::Protocol("GETSCRIPT without content".into()))
    }

    /// `HAVESPACE`: whether a script of `size` octets would fit.
    pub async fn have_space(&mut self, name: &str, size: usize) -> Result<(), SieveError> {
        let command = format!("HAVESPACE {} {size}\r\n", encode_string(name));
        self.write_all(command.as_bytes()).await?;
        let (_, response) = self.read_response().await?;
        self.expect_ok(response).map(|_| ())
    }

    /// `CHECKSCRIPT`: validate without storing. Returns the server's warnings.
    pub async fn check_script(&mut self, content: &str) -> Result<Option<String>, SieveError> {
        let command = format!("CHECKSCRIPT {}\r\n", encode_literal(content));
        self.write_all(command.as_bytes()).await?;
        let (_, response) = self.read_response().await?;
        let response = self.expect_ok(response)?;
        Ok(warnings(&response))
    }

    /// `PUTSCRIPT`: store (or replace) a script. Returns the server's warnings.
    pub async fn put_script(
        &mut self,
        name: &str,
        content: &str,
    ) -> Result<Option<String>, SieveError> {
        let command = format!(
            "PUTSCRIPT {} {}\r\n",
            encode_string(name),
            encode_literal(content)
        );
        self.write_all(command.as_bytes()).await?;
        let (_, response) = self.read_response().await?;
        let response = self.expect_ok(response)?;
        Ok(warnings(&response))
    }

    /// `SETACTIVE`; an empty name deactivates whatever is active.
    pub async fn set_active(&mut self, name: &str) -> Result<(), SieveError> {
        self.send_command("SETACTIVE", &[name]).await?;
        let (_, response) = self.read_response().await?;
        self.expect_ok(response).map(|_| ())
    }

    pub async fn delete_script(&mut self, name: &str) -> Result<(), SieveError> {
        self.send_command("DELETESCRIPT", &[name]).await?;
        let (_, response) = self.read_response().await?;
        self.expect_ok(response).map(|_| ())
    }

    pub async fn logout(mut self) -> Result<(), SieveError> {
        self.write_all(b"LOGOUT\r\n").await?;
        // The server answers OK or BYE and closes; either is fine.
        let _ = self.read_response().await;
        Ok(())
    }

    async fn send_command(&mut self, verb: &str, args: &[&str]) -> Result<(), SieveError> {
        let mut command = verb.as_bytes().to_vec();
        for arg in args {
            command.push(b' ');
            command.extend_from_slice(encode_string(arg).as_bytes());
        }
        command.extend_from_slice(b"\r\n");
        self.write_all(&command).await
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), SieveError> {
        let stream = self.reader.get_mut();
        timeout(OP_TIMEOUT, async {
            stream.write_all(bytes).await?;
            stream.flush().await
        })
        .await
        .map_err(|_| SieveError::Timeout)?
        .map_err(io_error)
    }

    fn expect_ok(&self, response: ServerResponse) -> Result<ServerResponse, SieveError> {
        match response.status {
            Status::Ok => Ok(response),
            Status::No => Err(SieveError::Refused(response)),
            Status::Bye => Err(SieveError::Bye(response)),
        }
    }

    async fn read_capabilities(&mut self) -> Result<Capabilities, SieveError> {
        let (lines, response) = self.read_response().await?;
        self.expect_ok(response)?;
        Ok(parse_capabilities(lines))
    }

    /// Data lines up to and including the final `OK`/`NO`/`BYE` line.
    async fn read_response(&mut self) -> Result<(Vec<Vec<Token>>, ServerResponse), SieveError> {
        let mut lines = Vec::new();
        loop {
            let line = self.read_line_tokens().await?;
            if matches!(line.first(), Some(Token::Atom(_))) {
                return Ok((lines, parse_final_line(&line)?));
            }
            if !line.is_empty() {
                lines.push(line);
            }
            if lines.len() > MAX_RESPONSE_LINES {
                return Err(SieveError::Protocol("response too long".into()));
            }
        }
    }

    async fn read_line_tokens(&mut self) -> Result<Vec<Token>, SieveError> {
        timeout(OP_TIMEOUT, async {
            let mut tokens = Vec::new();
            loop {
                match self.next_token().await? {
                    Token::Eol => return Ok(tokens),
                    token => tokens.push(token),
                }
            }
        })
        .await
        .map_err(|_| SieveError::Timeout)?
    }

    async fn peek_byte(&mut self) -> Result<u8, SieveError> {
        let buffer = self.reader.fill_buf().await.map_err(io_error)?;
        buffer
            .first()
            .copied()
            .ok_or_else(|| SieveError::Io("connection closed".into()))
    }

    async fn read_byte(&mut self) -> Result<u8, SieveError> {
        let byte = self.peek_byte().await?;
        self.reader.consume(1);
        Ok(byte)
    }

    async fn next_token(&mut self) -> Result<Token, SieveError> {
        loop {
            match self.peek_byte().await? {
                b' ' | b'\t' => self.reader.consume(1),
                b'\r' => {
                    self.reader.consume(1);
                    if self.read_byte().await? != b'\n' {
                        return Err(SieveError::Protocol("bare CR".into()));
                    }
                    return Ok(Token::Eol);
                }
                b'\n' => {
                    self.reader.consume(1);
                    return Ok(Token::Eol);
                }
                b'(' => {
                    self.reader.consume(1);
                    return Ok(Token::LParen);
                }
                b')' => {
                    self.reader.consume(1);
                    return Ok(Token::RParen);
                }
                b'"' => {
                    self.reader.consume(1);
                    return self.read_quoted().await.map(Token::Str);
                }
                b'{' => {
                    self.reader.consume(1);
                    return self.read_literal().await.map(Token::Str);
                }
                _ => return self.read_atom().await.map(Token::Atom),
            }
        }
    }

    async fn read_quoted(&mut self) -> Result<String, SieveError> {
        let mut bytes = Vec::new();
        loop {
            match self.read_byte().await? {
                b'"' => break,
                b'\\' => bytes.push(self.read_byte().await?),
                b'\r' | b'\n' => {
                    return Err(SieveError::Protocol("line break in quoted string".into()))
                }
                byte => bytes.push(byte),
            }
            if bytes.len() > MAX_SERVER_STRING {
                return Err(SieveError::Protocol("quoted string too long".into()));
            }
        }
        String::from_utf8(bytes).map_err(|_| SieveError::Protocol("string is not UTF-8".into()))
    }

    async fn read_literal(&mut self) -> Result<String, SieveError> {
        let mut digits = String::new();
        loop {
            match self.read_byte().await? {
                b'}' => break,
                b'+' => {}
                byte if byte.is_ascii_digit() && digits.len() < 10 => digits.push(byte as char),
                _ => return Err(SieveError::Protocol("malformed literal length".into())),
            }
        }
        let length: usize = digits
            .parse()
            .map_err(|_| SieveError::Protocol("malformed literal length".into()))?;
        if length > MAX_SERVER_STRING {
            return Err(SieveError::Protocol("literal too long".into()));
        }
        match self.read_byte().await? {
            b'\r' if self.read_byte().await? == b'\n' => {}
            b'\n' => {}
            _ => {
                return Err(SieveError::Protocol(
                    "literal length not followed by CRLF".into(),
                ))
            }
        }
        let mut bytes = vec![0; length];
        self.reader.read_exact(&mut bytes).await.map_err(io_error)?;
        String::from_utf8(bytes).map_err(|_| SieveError::Protocol("literal is not UTF-8".into()))
    }

    async fn read_atom(&mut self) -> Result<String, SieveError> {
        let mut atom = String::new();
        loop {
            let byte = self.peek_byte().await?;
            if matches!(
                byte,
                b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'"' | b'{'
            ) {
                break;
            }
            self.reader.consume(1);
            atom.push(byte as char);
            if atom.len() > 1024 {
                return Err(SieveError::Protocol("atom too long".into()));
            }
        }
        Ok(atom)
    }
}

fn parse_final_line(line: &[Token]) -> Result<ServerResponse, SieveError> {
    let mut tokens = line.iter().peekable();
    let status = match tokens.next() {
        Some(Token::Atom(atom)) if atom.eq_ignore_ascii_case("OK") => Status::Ok,
        Some(Token::Atom(atom)) if atom.eq_ignore_ascii_case("NO") => Status::No,
        Some(Token::Atom(atom)) if atom.eq_ignore_ascii_case("BYE") => Status::Bye,
        other => {
            return Err(SieveError::Protocol(format!(
                "unexpected response line start: {other:?}"
            )))
        }
    };
    let mut code = None;
    let mut code_arg = None;
    if tokens.peek() == Some(&&Token::LParen) {
        tokens.next();
        for token in tokens.by_ref() {
            match token {
                Token::RParen => break,
                Token::Atom(atom) if code.is_none() => code = Some(atom.to_ascii_uppercase()),
                Token::Str(value) if code_arg.is_none() => code_arg = Some(value.clone()),
                _ => {}
            }
        }
    }
    let text = tokens.find_map(|token| match token {
        Token::Str(value) => Some(value.clone()),
        _ => None,
    });
    Ok(ServerResponse {
        status,
        code,
        code_arg,
        text,
    })
}

fn parse_capabilities(lines: Vec<Vec<Token>>) -> Capabilities {
    let mut capabilities = Capabilities::default();
    for line in lines {
        let mut tokens = line.into_iter();
        let Some(Token::Str(name)) = tokens.next() else {
            continue;
        };
        let value = tokens.find_map(|token| match token {
            Token::Str(value) => Some(value),
            _ => None,
        });
        match name.to_ascii_uppercase().as_str() {
            "IMPLEMENTATION" => capabilities.implementation = value,
            "SIEVE" => {
                capabilities.sieve = value
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_ascii_lowercase)
                    .collect();
            }
            "SASL" => {
                capabilities.sasl = value
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_ascii_uppercase)
                    .collect();
            }
            "STARTTLS" => capabilities.starttls = true,
            "VERSION" => capabilities.version = value,
            "OWNER" => capabilities.owner = value,
            _ => {}
        }
    }
    capabilities
}

fn warnings(response: &ServerResponse) -> Option<String> {
    (response.code.as_deref() == Some("WARNINGS"))
        .then(|| response.text.clone().unwrap_or_default())
}

/// A quoted string, for values known to be short and free of line breaks.
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

/// A client string: quoted when RFC 5804 allows it, otherwise a
/// non-synchronizing literal (`{n+}`). Scripts always travel as literals.
fn encode_string(value: &str) -> String {
    let quotable = value.len() <= MAX_CLIENT_QUOTED && !value.contains(['\r', '\n', '\0']);
    if quotable {
        quote(value)
    } else {
        encode_literal(value)
    }
}

fn encode_literal(value: &str) -> String {
    format!("{{{}+}}\r\n{value}", value.len())
}

/// Open a connection and read the capabilities; STARTTLS is done here, so
/// the returned client is always encrypted.
pub async fn connect(
    endpoint: &SieveEndpoint,
    login: &str,
) -> Result<ManageSieveClient, SieveError> {
    let tcp = timeout(
        OP_TIMEOUT,
        TcpStream::connect((endpoint.host.as_str(), endpoint.port)),
    )
    .await
    .map_err(|_| SieveError::Timeout)?
    .map_err(|err| SieveError::Connect(err.to_string()))?;
    let config = crate::providers::imap::mail_tls_client_config(login).map_err(SieveError::Tls)?;
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = rustls_pki_types::ServerName::try_from(endpoint.host.clone())
        .map_err(|err| SieveError::Tls(format!("invalid server name: {err}")))?;
    let host = endpoint.host.clone();
    let port = endpoint.port;
    match endpoint.tls {
        SieveTls::Implicit => {
            let stream = handshake(connector, server_name, Box::new(tcp), &host, port).await?;
            ManageSieveClient::from_stream(stream, true).await
        }
        SieveTls::Starttls => {
            let plain = ManageSieveClient::from_stream(Box::new(tcp), false).await?;
            plain
                .starttls(|raw| handshake(connector, server_name, raw, &host, port))
                .await
        }
    }
}

async fn handshake(
    connector: TlsConnector,
    server_name: rustls_pki_types::ServerName<'static>,
    raw: BoxStream,
    host: &str,
    port: u16,
) -> Result<BoxStream, SieveError> {
    let stream = timeout(OP_TIMEOUT, connector.connect(server_name, raw))
        .await
        .map_err(|_| SieveError::Timeout)?
        .map_err(|err| {
            SieveError::Tls(crate::providers::tls_trust::handshake_error(
                "managesieve",
                host,
                port,
                &err,
            ))
        })?;
    Ok(Box::new(stream))
}

#[cfg(test)]
#[path = "managesieve_transcripts.rs"]
mod tests;
