//! Tauri-free persistence for local webhooks and their delivery queue.

use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Clone)]
pub struct WebhookUpsertRequest {
    pub id: Option<String>,
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookRecord {
    pub id: String,
    pub url: String,
    pub events_json: String,
    pub enabled: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// Data needed by the app-owned HTTP worker. This type deliberately does not
/// implement `Serialize`: the secret must never cross an IPC boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct DueWebhookDelivery {
    pub id: String,
    pub event_name: String,
    pub idempotency_key: String,
    pub payload_json: String,
    pub attempt_count: i64,
    pub url: String,
    pub secret: Option<String>,
}

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("webhook payload serialization failed: {0}")]
    PayloadSerialization(#[source] serde_json::Error),
    #[error("webhook persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for WebhookError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

impl From<serde_json::Error> for WebhookError {
    fn from(error: serde_json::Error) -> Self {
        Self::PayloadSerialization(error)
    }
}

#[derive(Debug, FromRow)]
struct StoredWebhook {
    id: String,
    url: String,
    events_json: String,
    enabled: i64,
    created_at: String,
    updated_at: String,
}

impl From<StoredWebhook> for WebhookRecord {
    fn from(row: StoredWebhook) -> Self {
        Self {
            id: row.id,
            url: row.url,
            events_json: row.events_json,
            enabled: row.enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Owns the local webhook definitions and delivery queue while leaving URL
/// validation, HTTP, secret/header behavior, and worker scheduling to the app.
pub struct WebhookRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> WebhookRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<WebhookRecord>, WebhookError> {
        sqlx::query_as::<_, StoredWebhook>(
            "SELECT id, url, events_json, enabled, created_at, updated_at
             FROM local_webhooks ORDER BY created_at DESC",
        )
        .fetch_all(self.pool)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
    }

    pub async fn upsert(&self, input: WebhookUpsertRequest) -> Result<WebhookRecord, WebhookError> {
        let id = input
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("wh-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        let events_json = serde_json::to_string(&input.events)?;
        let secret = input.secret.filter(|value| !value.trim().is_empty());

        sqlx::query_as::<_, StoredWebhook>(
            "INSERT INTO local_webhooks
                (id, url, events_json, enabled, secret, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                url = excluded.url,
                events_json = excluded.events_json,
                enabled = excluded.enabled,
                secret = excluded.secret,
                updated_at = excluded.updated_at
             RETURNING id, url, events_json, enabled, created_at, updated_at",
        )
        .bind(&id)
        .bind(&input.url)
        .bind(&events_json)
        .bind(if input.enabled { 1 } else { 0 })
        .bind(secret.as_deref())
        .bind(&now)
        .bind(&now)
        .fetch_one(self.pool)
        .await
        .map(Into::into)
        .map_err(Into::into)
    }

    pub async fn delete(&self, id: &str) -> Result<(), WebhookError> {
        sqlx::query("DELETE FROM local_webhooks WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Enqueue all enabled webhooks subscribed to `event_name` atomically.
    /// Invalid event lists are treated as empty, matching the old worker
    /// behavior. A failure in one insert rolls back every delivery in this
    /// fan-out, so a partial event cannot be observed by the worker.
    pub async fn enqueue_deliveries(
        &self,
        event_name: &str,
        idempotency_key: &str,
        payload: &serde_json::Value,
    ) -> Result<(), WebhookError> {
        let payload_json = serde_json::to_string(payload)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, events_json FROM local_webhooks WHERE enabled = 1")
                .fetch_all(&mut *transaction)
                .await?;
        let now = chrono::Utc::now().to_rfc3339();

        for (webhook_id, events_json) in rows {
            let events: Vec<String> = serde_json::from_str(&events_json).unwrap_or_default();
            if !events.iter().any(|event| event == event_name) {
                continue;
            }
            sqlx::query(
                "INSERT INTO webhook_deliveries
                    (id, webhook_id, event_name, idempotency_key, payload_json,
                     status, attempt_count, next_attempt_at, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?)",
            )
            .bind(format!("wd-{}", uuid::Uuid::new_v4()))
            .bind(&webhook_id)
            .bind(event_name)
            .bind(idempotency_key)
            .bind(&payload_json)
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    /// Load due deliveries in creation order. URL and secret are selected in
    /// this one query so the worker cannot observe a stale webhook definition
    /// between separate lookups. There is intentionally no claim/lease or
    /// deduplication here; those were not part of the existing semantics.
    pub async fn load_due(&self) -> Result<Vec<DueWebhookDelivery>, WebhookError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query_as::<_, DueWebhookDeliveryRow>(
            "SELECT d.id, d.event_name, d.idempotency_key, d.payload_json,
                    d.attempt_count, w.url, w.secret
             FROM webhook_deliveries d
             JOIN local_webhooks w ON w.id = d.webhook_id
             WHERE d.status = 'pending' AND d.next_attempt_at <= ? AND w.enabled = 1
             ORDER BY d.created_at ASC
             LIMIT 25",
        )
        .bind(now)
        .fetch_all(self.pool)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
    }

    pub async fn mark_delivered(&self, delivery_id: &str) -> Result<(), WebhookError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'delivered', attempt_count = attempt_count + 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(delivery_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn schedule_retry(
        &self,
        delivery_id: &str,
        attempts: i64,
        error: &str,
    ) -> Result<(), WebhookError> {
        let next_attempt = attempts + 1;
        let status = if next_attempt >= 5 {
            "failed"
        } else {
            "pending"
        };
        let delay_seconds = 2_i64.pow(next_attempt.clamp(1, 5) as u32) * 30;
        let now = chrono::Utc::now();
        let next_at = (now + chrono::Duration::seconds(delay_seconds)).to_rfc3339();
        let now = now.to_rfc3339();
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = ?, attempt_count = ?, next_attempt_at = ?, last_error = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(status)
        .bind(next_attempt)
        .bind(next_at)
        .bind(error)
        .bind(now)
        .bind(delivery_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[derive(FromRow)]
struct DueWebhookDeliveryRow {
    id: String,
    event_name: String,
    idempotency_key: String,
    payload_json: String,
    attempt_count: i64,
    url: String,
    secret: Option<String>,
}

impl From<DueWebhookDeliveryRow> for DueWebhookDelivery {
    fn from(row: DueWebhookDeliveryRow) -> Self {
        Self {
            id: row.id,
            event_name: row.event_name,
            idempotency_key: row.idempotency_key,
            payload_json: row.payload_json,
            attempt_count: row.attempt_count,
            url: row.url,
            secret: row.secret,
        }
    }
}
