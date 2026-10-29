//! Authenticated IMAP sessions kept for a short while between reads.
//!
//! Opening a message whose body the sync did not keep, or one of its inline
//! images, used to cost a TCP connection, a TLS handshake and a LOGIN each.
//! Moving through older mail opened one connection per message, which runs
//! into server connection limits and makes the message the reader stops on
//! wait behind the others. Reads now borrow an idle session for the account
//! and hand it back; one that sits unused past the TTL is logged out.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::time::Duration;

/// How long an idle session stays open. Far below the 30 minutes RFC 9051
/// lets a server wait, and long enough to cover reading a thread.
pub const IDLE_SESSION_TTL: Duration = Duration::from_secs(120);
/// Idle sessions kept per account; a second covers a body and an image at once.
const MAX_IDLE_PER_ACCOUNT: usize = 2;

struct IdleSession<S> {
    session: S,
    since: Instant,
}

pub struct SessionPool<S> {
    idle: Mutex<HashMap<String, Vec<IdleSession<S>>>>,
    ttl: Duration,
}

impl<S> SessionPool<S> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            idle: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<IdleSession<S>>>> {
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The most recently returned live session for `key`, if any.
    pub fn take(&self, key: &str) -> Option<S> {
        let mut idle = self.lock();
        let sessions = idle.get_mut(key)?;
        let ttl = self.ttl;
        sessions.retain(|entry| entry.since.elapsed() < ttl);
        let session = sessions.pop().map(|entry| entry.session);
        if sessions.is_empty() {
            idle.remove(key);
        }
        session
    }

    /// Hand a session back; returns whatever did not fit, to be closed.
    pub fn put(&self, key: &str, session: S) -> Vec<S> {
        let mut idle = self.lock();
        let sessions = idle.entry(key.to_string()).or_default();
        sessions.push(IdleSession {
            session,
            since: Instant::now(),
        });
        let overflow = sessions.len().saturating_sub(MAX_IDLE_PER_ACCOUNT);
        sessions
            .drain(..overflow)
            .map(|entry| entry.session)
            .collect()
    }

    /// Remove and return every session idle past the TTL.
    pub fn expired(&self) -> Vec<S> {
        let mut idle = self.lock();
        let ttl = self.ttl;
        let mut expired = Vec::new();
        for sessions in idle.values_mut() {
            let (old, fresh): (Vec<_>, Vec<_>) = std::mem::take(sessions)
                .into_iter()
                .partition(|entry| entry.since.elapsed() >= ttl);
            *sessions = fresh;
            expired.extend(old.into_iter().map(|entry| entry.session));
        }
        idle.retain(|_, sessions| !sessions.is_empty());
        expired
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// What a read did with the session it borrowed: handed back if it is still
/// usable, dropped if the connection failed.
pub type SessionOutcome<S, T> = (Option<S>, Result<T, String>);

/// Run `read` on a pooled session for `key`, logging in only when none is idle.
///
/// A borrowed session may have been closed by the server while it waited; when
/// a read on it fails, the session is closed and the read runs once more on a
/// fresh login.
pub async fn read_with_pool<S, T, C, CF, R, RF, L, LF>(
    pool: &Arc<SessionPool<S>>,
    key: &str,
    connect: C,
    read: R,
    close: L,
) -> Result<T, String>
where
    S: Send + 'static,
    C: FnOnce() -> CF,
    CF: Future<Output = Result<S, String>>,
    R: Fn(S) -> RF,
    RF: Future<Output = SessionOutcome<S, T>>,
    L: Fn(S) -> LF + Clone + Send + Sync + 'static,
    LF: Future<Output = ()> + Send + 'static,
{
    if let Some(session) = pool.take(key) {
        match read(session).await {
            (Some(session), Ok(value)) => {
                give_back(pool, key, session, close);
                return Ok(value);
            }
            (None, Ok(value)) => return Ok(value),
            // A connection the server dropped can surface as an empty reply rather
            // than an I/O error, so any failure on a reused session retries once.
            (session, Err(error)) => {
                tracing::debug!(%error, "pooled imap session failed; logging in again");
                if let (Some(session), Ok(runtime)) =
                    (session, tokio::runtime::Handle::try_current())
                {
                    runtime.spawn(close.clone()(session));
                }
            }
        }
    }

    let session = connect().await?;
    let (session, result) = read(session).await;
    if let Some(session) = session {
        give_back(pool, key, session, close);
    }
    result
}

fn give_back<S, L, LF>(pool: &Arc<SessionPool<S>>, key: &str, session: S, close: L)
where
    S: Send + 'static,
    L: Fn(S) -> LF + Clone + Send + Sync + 'static,
    LF: Future<Output = ()> + Send + 'static,
{
    let overflow = pool.put(key, session);
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let pool = Arc::clone(pool);
    let ttl = pool.ttl();
    runtime.spawn(async move {
        for session in overflow {
            close(session).await;
        }
        tokio::time::sleep(ttl + Duration::from_millis(50)).await;
        for session in pool.expired() {
            close(session).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::imap::{fetch_body_on_session, ParsedBody};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    type PlainSession = async_imap::Session<TcpStream>;

    #[derive(Default)]
    struct Counters {
        connections: AtomicUsize,
        logins: AtomicUsize,
        fetches: AtomicUsize,
        generation: AtomicUsize,
    }

    /// Answers LOGIN, SELECT, UID FETCH and LOGOUT like a small server would.
    /// With `drop_after_first_fetch` it hangs up on a connection once that
    /// connection served one fetch, as a server timing out idle clients does.
    async fn fake_server(drop_after_first_fetch: bool) -> (std::net::SocketAddr, Arc<Counters>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let counters = Arc::new(Counters::default());
        counters.generation.store(7, Ordering::SeqCst);
        let shared = Arc::clone(&counters);
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                shared.connections.fetch_add(1, Ordering::SeqCst);
                let counters = Arc::clone(&shared);
                tokio::spawn(async move {
                    let (reader, mut writer) = socket.into_split();
                    let mut reader = tokio::io::BufReader::new(reader);
                    writer.write_all(b"* OK fake IMAP ready\r\n").await.unwrap();
                    let mut served = 0;
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        let mut parts = line.split_whitespace();
                        let tag = parts.next().unwrap_or("*").to_string();
                        let command = parts.next().unwrap_or("").to_ascii_uppercase();
                        let reply = match command.as_str() {
                            "LOGIN" => {
                                counters.logins.fetch_add(1, Ordering::SeqCst);
                                format!("{tag} OK LOGIN completed\r\n")
                            }
                            "SELECT" => {
                                if drop_after_first_fetch && served > 0 {
                                    return;
                                }
                                format!("* 3 EXISTS\r\n* OK [UIDVALIDITY {}] ok\r\n{tag} OK [READ-WRITE] SELECT completed\r\n", counters.generation.load(Ordering::SeqCst))
                            }
                            "UID" => {
                                counters.fetches.fetch_add(1, Ordering::SeqCst);
                                served += 1;
                                let header = "Content-Type: text/plain; charset=utf-8\r\n\r\n";
                                let text = if counters.generation.load(Ordering::SeqCst) == 7 {
                                    "Body from the fake server"
                                } else {
                                    "Payload of another message after UID reuse"
                                };
                                format!(
                                    "* 1 FETCH (UID 42 BODY[HEADER] {{{}}}\r\n{header} BODY[TEXT] {{{}}}\r\n{text})\r\n{tag} OK FETCH completed\r\n",
                                    header.len(),
                                    text.len()
                                )
                            }
                            "LOGOUT" => {
                                let _ = writer
                                    .write_all(
                                        format!("* BYE\r\n{tag} OK LOGOUT completed\r\n")
                                            .as_bytes(),
                                    )
                                    .await;
                                return;
                            }
                            _ => format!("{tag} BAD unknown\r\n"),
                        };
                        if writer.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        (address, counters)
    }

    async fn plain_login(address: std::net::SocketAddr) -> Result<PlainSession, String> {
        let socket = TcpStream::connect(address)
            .await
            .map_err(|err| err.to_string())?;
        let mut client = async_imap::Client::new(socket);
        client
            .read_response()
            .await
            .map_err(|err| err.to_string())?;
        client
            .login("user", "password")
            .await
            .map_err(|(err, _)| err.to_string())
    }

    async fn read_body(
        pool: &Arc<SessionPool<PlainSession>>,
        address: std::net::SocketAddr,
    ) -> Result<ParsedBody, String> {
        read_with_pool(
            pool,
            "fake:993/user",
            || plain_login(address),
            |session| {
                fetch_body_on_session(session, "INBOX".to_string(), "42".to_string(), Some(7))
            },
            |mut session: PlainSession| async move {
                let _ = session.logout().await;
            },
        )
        .await
    }

    #[tokio::test]
    async fn pooled_body_fetch_rejects_uid_reuse_after_folder_reset() {
        let (address, counters) = fake_server(false).await;
        let pool = Arc::new(SessionPool::new(IDLE_SESSION_TTL));
        read_body(&pool, address).await.unwrap();
        counters.generation.store(8, Ordering::SeqCst);
        let result = read_body(&pool, address).await;
        assert!(
            result.is_err(),
            "UID 42 now belongs to another message: {result:?}"
        );
        assert_eq!(
            counters.fetches.load(Ordering::SeqCst),
            1,
            "generation must be checked before fetching another message's body"
        );
    }

    /// Holding `j` through 30 old threads fetches 30 bodies: one login.
    #[tokio::test]
    async fn thirty_body_fetches_share_one_login() {
        // Without reuse, which is what every fetch did before: one login each.
        let (address, counters) = fake_server(false).await;
        let no_reuse = Arc::new(SessionPool::new(Duration::ZERO));
        for _ in 0..30 {
            read_body(&no_reuse, address).await.unwrap();
        }
        assert_eq!(counters.logins.load(Ordering::SeqCst), 30);

        let (address, counters) = fake_server(false).await;
        let pool = Arc::new(SessionPool::new(IDLE_SESSION_TTL));

        for _ in 0..30 {
            let body = read_body(&pool, address).await.unwrap();
            assert_eq!(body.text.as_deref(), Some("Body from the fake server"));
        }

        assert_eq!(counters.fetches.load(Ordering::SeqCst), 30);
        assert_eq!(counters.logins.load(Ordering::SeqCst), 1);
        assert_eq!(counters.connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_session_the_server_closed_is_replaced_by_a_fresh_login() {
        let (address, counters) = fake_server(true).await;
        let pool = Arc::new(SessionPool::new(IDLE_SESSION_TTL));

        for _ in 0..3 {
            read_body(&pool, address).await.unwrap();
        }

        assert_eq!(counters.fetches.load(Ordering::SeqCst), 3);
        assert_eq!(counters.logins.load(Ordering::SeqCst), 3);
    }

    /// macOS can report a dropped connection as an empty reply on the old session.
    #[tokio::test]
    async fn a_failed_read_on_a_reused_session_retries_on_a_fresh_login() {
        let pool = Arc::new(SessionPool::new(IDLE_SESSION_TTL));
        let _ = pool.put("key", 1_u32);
        let logins = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&logins);
        let result = read_with_pool(
            &pool,
            "key",
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok(2_u32) }
            },
            |session| async move {
                if session == 1 {
                    (
                        Some(session),
                        Err("imap message not found on the server".to_string()),
                    )
                } else {
                    (Some(session), Ok(session))
                }
            },
            |_session: u32| async {},
        )
        .await;
        assert_eq!(result, Ok(2));
        assert_eq!(logins.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_idle_session_past_its_ttl_is_not_reused() {
        let (address, counters) = fake_server(false).await;
        let pool = Arc::new(SessionPool::new(Duration::from_millis(20)));

        read_body(&pool, address).await.unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        read_body(&pool, address).await.unwrap();

        assert_eq!(counters.logins.load(Ordering::SeqCst), 2);
    }
}
