//! One-time setup between two foreground clients. Only the native importer
//! sees passwords. The webview can select local IDs and review email addresses.
mod discovery;
mod mail;
mod wire;

use super::error::{CommandError, CommandResult};
use crate::sync::idle::IdleControl;
use discovery::platform::Discovery;
use rand::Rng;
use satchel_core::db::accounts::{AccountRepository, CreateEmailAccount};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddrV4, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::{timeout, Instant},
};

const LIFETIME: Duration = Duration::from_secs(300);
const HANDSHAKE: Duration = Duration::from_secs(30);

fn failed() -> CommandError {
    CommandError::new("backend.errors.accountTransfer")
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/account_transfer/")
)]
pub struct Nearby {
    name: String,
    address: String,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/account_transfer/")
)]
pub enum TransferPhase {
    #[default]
    Idle,
    Waiting,
    Searching,
    Connecting,
    Preview,
    Sent,
    Imported,
    Failed,
    Expired,
}

#[derive(Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/account_transfer/")
)]
pub struct TransferStatus {
    phase: TransferPhase,
    code: Option<String>,
    address: Option<String>,
    nearby: Vec<Nearby>,
    emails: Vec<String>,
    sign_in: Vec<String>,
    created: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/account_transfer/")
)]
pub enum TransferAction {
    Receive,
    Discover,
    Status,
    Send,
    Import,
    Cancel,
}

struct Session {
    id: String,
    expires: Instant,
    status: TransferStatus,
    staged: Vec<CreateEmailAccount>,
    discovery: Option<Discovery>,
    worker: Option<tokio::task::AbortHandle>,
    expiry: Option<tokio::task::AbortHandle>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(task) = self.worker.take() {
            task.abort();
        }
        if let Some(task) = self.expiry.take() {
            task.abort();
        }
    }
}

#[derive(Default, Clone)]
pub struct Transfers(Arc<Mutex<Option<Session>>>);

async fn receive(
    state: Transfers,
    id: &str,
    listener: TcpListener,
    code: String,
) -> CommandResult<()> {
    let socket = listener.accept().await.map_err(|_| failed())?.0;
    drop(listener);
    {
        let mut lock = state.0.lock().await;
        let session = lock.as_mut().filter(|s| s.id == id).ok_or_else(failed)?;
        session.discovery.take();
        session.status.code = None;
        session.status.address = None;
        session.status.phase = TransferPhase::Connecting;
    }
    let (mut channel, incoming) = timeout(HANDSHAKE, async {
        let mut channel = wire::Channel::open(socket, &code, false).await?;
        let incoming = mail::decode(&channel.read(b"accounts").await?)?;
        Ok::<_, CommandError>((channel, incoming))
    })
    .await
    .map_err(|_| failed())??;
    {
        let mut lock = state.0.lock().await;
        let session = lock
            .as_mut()
            .filter(|s| s.id == id && s.expires > Instant::now())
            .ok_or_else(failed)?;
        session.status.emails = incoming.iter().map(|a| a.email.clone()).collect();
        session.staged = incoming;
        session.status.phase = TransferPhase::Preview;
    }
    // A lost receipt cannot invalidate the recipient's local preview.
    let _ = timeout(HANDSHAKE, channel.write(b"staged", Vec::new())).await;
    Ok(())
}

async fn send(
    pool: sqlx::SqlitePool,
    address: SocketAddrV4,
    code: String,
    ids: Vec<String>,
) -> CommandResult<()> {
    let socket = TcpStream::connect(address).await.map_err(|_| failed())?;
    let mut channel = wire::Channel::open(socket, &code, true).await?;
    let selected = mail::selected(&pool, &ids).await?;
    channel
        .write(
            b"accounts",
            serde_json::to_vec(&selected).map_err(|_| failed())?,
        )
        .await?;
    if !channel.read(b"staged").await?.is_empty() {
        return Err(failed());
    }
    Ok(())
}

impl Transfers {
    async fn start(
        &self,
        app: &tauri::AppHandle,
        id: String,
        receiving: bool,
    ) -> CommandResult<TransferStatus> {
        if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(failed());
        }
        let mut lock = self.0.lock().await;
        if lock
            .as_ref()
            .is_some_and(|s| s.expires > Instant::now() && s.id != id)
        {
            return Err(failed());
        }
        let mut status = TransferStatus::default();
        let mut worker = None;
        let mut port = None;
        if receiving {
            let listener = TcpListener::bind("0.0.0.0:0").await.map_err(|_| failed())?;
            let inbound_port = listener.local_addr().map_err(|_| failed())?.port();
            port = Some(inbound_port);
            status.address = discovery::local_address(inbound_port);
            let code = format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000));
            status.code = Some(code.clone());
            status.phase = TransferPhase::Waiting;
            let state = self.clone();
            let owner = id.clone();
            worker = Some(
                tokio::spawn(async move {
                    if receive(state.clone(), &owner, listener, code)
                        .await
                        .is_err()
                    {
                        state.finish(&owner, TransferPhase::Failed).await;
                    }
                })
                .abort_handle(),
            );
        } else {
            status.phase = TransferPhase::Searching;
        }
        let discovery_id = uuid::Uuid::new_v4().simple().to_string();
        let discovery = Discovery::start(app, &discovery_id, port).await;
        let expires = Instant::now() + LIFETIME;
        let owner = id.clone();
        let state = self.clone();
        let expiry = tokio::spawn(async move {
            tokio::time::sleep_until(expires).await;
            let mut lock = state.0.lock().await;
            if lock
                .as_ref()
                .is_some_and(|s| s.id == owner && s.expires == expires)
            {
                lock.take();
            }
        })
        .abort_handle();
        *lock = Some(Session {
            id,
            expires,
            status: status.clone(),
            discovery,
            worker,
            expiry: Some(expiry),
            staged: Vec::new(),
        });
        Ok(status)
    }
    async fn finish(&self, id: &str, phase: TransferPhase) {
        if let Some(s) = self.0.lock().await.as_mut().filter(|s| s.id == id) {
            s.status.phase = phase;
            s.status.code = None;
            s.status.address = None;
            s.discovery.take();
        }
    }
}

#[tauri::command]
pub async fn account_transfer(
    app: tauri::AppHandle,
    transfers: tauri::State<'_, Transfers>,
    pool: tauri::State<'_, sqlx::SqlitePool>,
    idle: tauri::State<'_, IdleControl>,
    action: TransferAction,
    session: String,
    address: Option<String>,
    code: Option<String>,
    accounts: Option<Vec<String>>,
) -> CommandResult<TransferStatus> {
    use TransferAction::*;
    if matches!(action, Receive | Discover) {
        return transfers
            .start(&app, session, matches!(action, Receive))
            .await;
    }
    let mut lock = transfers.0.lock().await;
    let Some(active) = lock
        .as_mut()
        .filter(|s| s.id == session && s.expires > Instant::now())
    else {
        return Ok(TransferStatus {
            phase: TransferPhase::Expired,
            ..Default::default()
        });
    };
    match action {
        Cancel => {
            lock.take();
            return Ok(TransferStatus::default());
        }
        Status => {
            if active.status.phase == TransferPhase::Searching {
                if let Some(d) = &mut active.discovery {
                    active.status.nearby = d.peers().await;
                }
            }
        }
        Send if active.status.phase == TransferPhase::Searching => {
            let address = address
                .and_then(|a| a.parse::<SocketAddrV4>().ok())
                .filter(|a| {
                    a.port() != 0
                        && !a.ip().is_multicast()
                        && !a.ip().is_unspecified()
                        && !a.ip().is_broadcast()
                })
                .ok_or_else(failed)?;
            let code = code
                .filter(|c| c.len() == 6 && c.bytes().all(|v| v.is_ascii_digit()))
                .ok_or_else(failed)?;
            let ids = accounts
                .filter(|a| (1..=64).contains(&a.len()))
                .ok_or_else(failed)?;
            active.status.phase = TransferPhase::Connecting;
            active.discovery.take();
            let pool = pool.inner().clone();
            let state = transfers.inner().clone();
            active.worker = Some(
                tokio::spawn(async move {
                    let result = timeout(HANDSHAKE, send(pool, address, code, ids)).await;
                    state
                        .finish(
                            &session,
                            if matches!(result, Ok(Ok(()))) {
                                TransferPhase::Sent
                            } else {
                                TransferPhase::Failed
                            },
                        )
                        .await;
                })
                .abort_handle(),
            );
        }
        Import if active.status.phase == TransferPhase::Preview => {
            let created = AccountRepository::new(pool.inner())
                .import_email_accounts(active.staged.clone())
                .await?;
            active.staged.clear();
            active.status.created = created.len();
            for a in created {
                if matches!(a.provider.as_str(), "gmail" | "outlook") {
                    active.status.sign_in.push(a.email);
                } else {
                    idle.add_account(a.id);
                }
            }
            active.status.phase = TransferPhase::Imported;
        }
        Import if active.status.phase == TransferPhase::Imported => {}
        _ => return Err(failed()),
    }
    Ok(active.status.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

    async fn profile() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn selected_accounts_can_travel_in_both_directions_after_approval() {
        satchel_core::utils::crypto::seed_master_key_for_tests();
        let a = profile().await;
        let b = profile().await;
        for (source, destination, email) in [
            (&a, &b, "first@example.test"),
            (&b, &a, "second@example.test"),
        ] {
            let repo = AccountRepository::new(source);
            let row = repo.create_email(CreateEmailAccount{
                email:email.into(),display_name:"Fixture".into(),label:Some("Personal".into()),provider:"imap".into(),
                config:Some(json!({"host":"mail.example.test","password":"incoming","smtp_password":"outgoing","dav":{"password":"other"}})),
            }).await.unwrap();
            let gmail = repo
                .create_email(CreateEmailAccount {
                    email: format!("google-{email}"),
                    display_name: "Fixture".into(),
                    label: None,
                    provider: "gmail".into(),
                    config: Some(json!({"oauth":{"refresh_token":"source-only"}})),
                })
                .await
                .unwrap();
            let id = "synthetic";
            let state = Transfers::default();
            *state.0.lock().await = Some(Session {
                id: id.into(),
                expires: Instant::now() + LIFETIME,
                status: TransferStatus::default(),
                staged: vec![],
                discovery: None,
                worker: None,
                expiry: None,
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let std::net::SocketAddr::V4(address) = listener.local_addr().unwrap() else {
                unreachable!()
            };
            let (sent, received) = tokio::join!(
                send(
                    source.clone(),
                    address,
                    "007891".into(),
                    vec![row.id.clone(), gmail.id]
                ),
                receive(state.clone(), id, listener, "007891".into())
            );
            sent.unwrap();
            received.unwrap();
            let target = AccountRepository::new(destination);
            assert!(!target
                .list_active()
                .await
                .unwrap()
                .iter()
                .any(|a| a.email == email));
            let mut lock = state.0.lock().await;
            let session = lock.as_mut().unwrap();
            assert_eq!(session.status.emails, [email, &format!("google-{email}")]);
            let batch = std::mem::take(&mut session.staged);
            let created = target.import_email_accounts(batch.clone()).await.unwrap();
            assert!(created[0].id != row.id);
            assert_eq!(created[0].label, row.label);
            let config = target
                .load_provider_config(&created[0].id)
                .await
                .unwrap()
                .unwrap()
                .config;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&config).unwrap(),
                json!({"imap":{"host":"mail.example.test","password":"incoming","smtp_password":"outgoing"}})
            );
            let config = target
                .load_provider_config(&created[1].id)
                .await
                .unwrap()
                .unwrap()
                .config;
            assert_eq!(&*config, "{}");
            assert_eq!(
                created[1].last_oauth_error.as_deref(),
                Some("backend.errors.transferSignIn")
            );
            assert!(target
                .import_email_accounts(batch)
                .await
                .unwrap()
                .is_empty());
        }
    }
}
