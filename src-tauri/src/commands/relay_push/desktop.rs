use super::{origin, CommandError, CommandResult, PushDevice, RelayAction, RelayStatus};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use satchel_core::utils::crypto::{decrypt_secret, encrypt_secret, SecretContext};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    sync::{Mutex, Notify},
};

const SERVICE: &str = "_satchel-pair._tcp.local.";
fn failed() -> CommandError {
    CommandError::new("settings.relay.failed")
}

struct Offer {
    id: String,
    secret: String,
    expires: Instant,
    dns: ServiceDaemon,
    service: String,
    listener: tauri::async_runtime::JoinHandle<()>,
}
impl Drop for Offer {
    fn drop(&mut self) {
        let _ = self.dns.unregister(&self.service);
        let _ = self.dns.shutdown();
        self.listener.abort();
    }
}

pub struct Desktop {
    pool: SqlitePool,
    client: reqwest::Client,
    offer: Arc<Mutex<Option<Offer>>>,
    arrivals: Arc<Notify>,
}

async fn post(
    client: &reqwest::Client,
    path: &str,
    secret: &str,
    payload: Value,
) -> Result<Value, u16> {
    let res = client
        .post(format!("{}/v1/{path}", origin()))
        .bearer_auth(secret)
        .json(&payload)
        .send()
        .await
        .map_err(|_| 503u16)?;
    if !res.status().is_success() {
        return Err(res.status().as_u16());
    }
    res.json().await.map_err(|_| 503)
}

impl Desktop {
    pub fn new(pool: SqlitePool) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("push HTTP client");
        let arrivals = Arc::new(Notify::new());
        if !origin().is_empty() {
            tauri::async_runtime::spawn(deliver(pool.clone(), client.clone(), arrivals.clone()));
        }
        Self {
            pool,
            client,
            offer: Arc::default(),
            arrivals,
        }
    }
    pub fn mail_arrived(&self) {
        self.arrivals.notify_one();
    }

    async fn show(&self) -> CommandResult<Offer> {
        let socket = TcpListener::bind(("0.0.0.0", 0))
            .await
            .map_err(|_| failed())?;
        let port = socket.local_addr().map_err(|_| failed())?.port();
        let result = post(&self.client, "offers", "", json!({}))
            .await
            .map_err(|_| failed())?;
        let id = result["id"]
            .as_str()
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(failed)?
            .to_owned();
        let secret = result["sender"].as_str().ok_or_else(failed)?.to_owned();
        let name: String = whoami::devicename()
            .chars()
            .filter(|c| !c.is_control())
            .take(50)
            .collect();
        let dns = ServiceDaemon::new().map_err(|_| failed())?;
        let info = ServiceInfo::new(
            SERVICE,
            &id,
            &format!("satchel-{id}.local."),
            "",
            port,
            &[("name", name.as_str()), ("id", id.as_str())][..],
        )
        .map_err(|_| failed())?
        .enable_addr_auto();
        let service = info.get_fullname().to_owned();
        dns.register(info).map_err(|_| failed())?;
        let public = id.clone();
        let listener = tauri::async_runtime::spawn(async move {
            while let Ok((mut stream, _)) = socket.accept().await {
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    stream.write_all(public.as_bytes()),
                )
                .await;
            }
        });
        Ok(Offer {
            id,
            secret,
            dns,
            service,
            listener,
            expires: Instant::now() + Duration::from_secs(300),
        })
    }

    pub async fn apply(
        &self,
        action: RelayAction,
        id: Option<String>,
    ) -> CommandResult<RelayStatus> {
        let mut guard = self.offer.lock().await;
        if guard.as_ref().is_some_and(|o| o.expires <= Instant::now()) {
            guard.take();
        }
        if matches!(action, RelayAction::Start) && guard.is_none() {
            let new = self.show().await?;
            let offers = self.offer.clone();
            let visible = new.id.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(300)).await;
                let mut slot = offers.lock().await;
                if slot.as_ref().is_some_and(|o| o.id == visible) {
                    slot.take();
                }
            });
            *guard = Some(new);
        }
        let mut state = RelayStatus {
            phase: "idle".into(),
            ..Default::default()
        };
        if matches!(action, RelayAction::Remove) {
            let id = id.as_deref().ok_or_else(failed)?;
            let enc: String =
                sqlx::query_scalar("SELECT credential FROM desktop_push_links WHERE id=?")
                    .bind(id)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|_| failed())?;
            let secret =
                decrypt_secret(&enc, SecretContext::push_link(id)).map_err(|_| failed())?;
            match post(
                &self.client,
                &format!("links/{id}/remove"),
                &secret,
                json!({}),
            )
            .await
            {
                Ok(_) | Err(410) => (),
                _ => return Err(failed()),
            }
            sqlx::query("DELETE FROM desktop_push_links WHERE id=?")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|_| failed())?;
        }
        if let Some(offer) = guard.as_ref() {
            let path = format!("links/{}/", offer.id);
            if matches!(action, RelayAction::Cancel) {
                // Closing settings after approval must not revoke a completed pairing.
                let saved: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM desktop_push_links WHERE id=?)",
                )
                .bind(&offer.id)
                .fetch_one(&self.pool)
                .await
                .map_err(|_| failed())?;
                if !saved {
                    let _ = post(&self.client, &(path + "remove"), &offer.secret, json!({})).await;
                }
                guard.take();
            } else {
                let status = post(
                    &self.client,
                    &(path.clone() + "status"),
                    &offer.secret,
                    json!({}),
                )
                .await;
                match status {
                    Err(410) => {
                        guard.take();
                        state.phase = "expired".into();
                    }
                    Err(_) => return Err(failed()),
                    Ok(value) => {
                        state.phase = value["phase"].as_str().ok_or_else(failed)?.into();
                        state.code = value["code"].as_str().map(str::to_owned);
                        state.name = value["phone"].as_str().map(str::to_owned);
                        state.request = value["request"].as_str().map(str::to_owned);
                        if matches!(action, RelayAction::Approve) {
                            if state.phase != "requested" || id.is_none() || id != state.request {
                                return Err(failed());
                            }
                            let enc =
                                encrypt_secret(&offer.secret, SecretContext::push_link(&offer.id))
                                    .map_err(|_| failed())?;
                            sqlx::query("INSERT OR IGNORE INTO desktop_push_links(id,name,credential) VALUES(?,?,?)")
                                .bind(&offer.id).bind(state.name.as_deref().unwrap_or("Android")).bind(enc)
                                .execute(&self.pool).await.map_err(|_| failed())?;
                            post(
                                &self.client,
                                &(path + "approve"),
                                &offer.secret,
                                json!({"request":id}),
                            )
                            .await
                            .map_err(|_| failed())?;
                            state.phase = "verifying".into();
                            self.mail_arrived();
                        } else if state.phase == "active" {
                            sqlx::query("UPDATE desktop_push_links SET approved=1 WHERE id=?")
                                .bind(&offer.id)
                                .execute(&self.pool)
                                .await
                                .map_err(|_| failed())?;
                            guard.take();
                            state.phase = "idle".into();
                        }
                    }
                }
            }
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id,name FROM desktop_push_links WHERE approved=1 ORDER BY name,id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| failed())?;
        state.devices = rows
            .into_iter()
            .map(|(id, name)| PushDevice {
                id,
                name,
                state: "verified".into(),
            })
            .collect();
        Ok(state)
    }
}

async fn deliver(pool: SqlitePool, client: reqwest::Client, arrivals: Arc<Notify>) {
    let mut retries: HashMap<String, (Instant, u64)> = HashMap::new();
    loop {
        let rows = sqlx::query_as::<_, (String, String, bool, i64)>(
            "SELECT id,credential,approved,pending FROM desktop_push_links WHERE pending>0 OR approved=0",
        )
        .fetch_all(&pool)
        .await;
        let Ok(rows) = rows else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };
        retries.retain(|id, _| rows.iter().any(|row| &row.0 == id));
        for (id, enc, approved, pending) in rows {
            if retries
                .get(&id)
                .is_some_and(|(next, _)| *next > Instant::now())
            {
                continue;
            }
            let delivered = deliver_link(&pool, &client, &id, &enc, approved, pending).await;
            if delivered {
                retries.remove(&id);
            } else {
                let seconds = retries.get(&id).map_or(5, |(_, last)| (last * 2).min(300));
                retries.insert(id, (Instant::now() + Duration::from_secs(seconds), seconds));
            }
        }
        if let Some(next) = retries.values().map(|(next, _)| *next).min() {
            tokio::select! {
                _ = arrivals.notified() => (),
                _ = tokio::time::sleep_until(next.into()) => (),
            }
        } else {
            arrivals.notified().await;
        }
    }
}

async fn deliver_link(
    pool: &SqlitePool,
    client: &reqwest::Client,
    id: &str,
    enc: &str,
    approved: bool,
    pending: i64,
) -> bool {
    let Ok(secret) = decrypt_secret(enc, SecretContext::push_link(id)) else {
        return false;
    };
    let route = if approved { "wake" } else { "status" };
    match post(client, &format!("links/{id}/{route}"), &secret, json!({})).await {
        Ok(_) if approved => {
            sqlx::query("UPDATE desktop_push_links SET pending=pending-? WHERE id=?")
                .bind(pending)
                .bind(id)
                .execute(pool)
                .await
                .is_ok()
        }
        Ok(value) if value["phase"] == "active" => {
            let updated = sqlx::query("UPDATE desktop_push_links SET approved=1 WHERE id=?")
                .bind(id)
                .execute(pool)
                .await
                .is_ok();
            updated && pending == 0
        }
        Err(410) => sqlx::query("DELETE FROM desktop_push_links WHERE id=?")
            .bind(id)
            .execute(pool)
            .await
            .is_ok(),
        _ => false,
    }
}
