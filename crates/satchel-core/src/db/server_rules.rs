//! Persistence for server-side rules (ManageSieve), one row per IMAP account.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::providers::managesieve::{SieveEndpoint, SieveTls, DEFAULT_PORT};

/// The version of a rule that went live on the server, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushedRule {
    pub fingerprint: String,
    /// RFC 3339. Mail that arrived earlier was never seen by this version.
    pub since: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerRulesState {
    pub account_id: String,
    pub host_override: Option<String>,
    pub port_override: Option<u16>,
    pub tls_override: Option<SieveTls>,
    pub detected: Option<SieveEndpoint>,
    pub detected_source: Option<String>,
    /// `unknown`, `available` or `unavailable`.
    pub availability: String,
    pub implementation: Option<String>,
    pub extensions: Vec<String>,
    pub checked_at: Option<String>,
    pub enabled: bool,
    pub server_active: bool,
    pub activation_mode: Option<String>,
    pub previous_active_script: Option<String>,
    /// The foreign active script a push stopped at, waiting for a choice.
    pub consent_script: Option<String>,
    pub pushed_rules: BTreeMap<String, PushedRule>,
    pub pushed_vacation: bool,
    pub last_push_at: Option<String>,
    /// A message key.
    pub last_error: Option<String>,
}

impl ServerRulesState {
    pub fn empty(account_id: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            availability: "unknown".to_string(),
            ..Default::default()
        }
    }

    /// The endpoint the person typed in, if any. A host alone uses the
    /// standard port with STARTTLS.
    pub fn override_endpoint(&self) -> Option<SieveEndpoint> {
        let host = self.host_override.clone()?;
        Some(SieveEndpoint {
            host,
            port: self.port_override.unwrap_or(DEFAULT_PORT),
            tls: self.tls_override.unwrap_or(SieveTls::Starttls),
        })
    }
}

#[derive(FromRow)]
struct Row {
    account_id: String,
    host_override: Option<String>,
    port_override: Option<i64>,
    tls_override: Option<String>,
    detected_host: Option<String>,
    detected_port: Option<i64>,
    detected_tls: Option<String>,
    detected_source: Option<String>,
    availability: String,
    implementation: Option<String>,
    extensions: String,
    checked_at: Option<String>,
    enabled: i64,
    server_active: i64,
    activation_mode: Option<String>,
    previous_active_script: Option<String>,
    consent_script: Option<String>,
    pushed_rules: String,
    pushed_vacation: i64,
    last_push_at: Option<String>,
    last_error: Option<String>,
}

fn port(value: Option<i64>) -> Option<u16> {
    value.and_then(|port| u16::try_from(port).ok())
}

impl From<Row> for ServerRulesState {
    fn from(row: Row) -> Self {
        let detected = match (row.detected_host, port(row.detected_port)) {
            (Some(host), Some(port)) => Some(SieveEndpoint {
                host,
                port,
                tls: row
                    .detected_tls
                    .as_deref()
                    .and_then(SieveTls::parse)
                    .unwrap_or(SieveTls::Starttls),
            }),
            _ => None,
        };
        Self {
            account_id: row.account_id,
            host_override: row.host_override,
            port_override: port(row.port_override),
            tls_override: row.tls_override.as_deref().and_then(SieveTls::parse),
            detected,
            detected_source: row.detected_source,
            availability: row.availability,
            implementation: row.implementation,
            extensions: serde_json::from_str(&row.extensions).unwrap_or_default(),
            checked_at: row.checked_at,
            enabled: row.enabled == 1,
            server_active: row.server_active == 1,
            activation_mode: row.activation_mode,
            previous_active_script: row.previous_active_script,
            consent_script: row.consent_script,
            pushed_rules: serde_json::from_str(&row.pushed_rules).unwrap_or_default(),
            pushed_vacation: row.pushed_vacation == 1,
            last_push_at: row.last_push_at,
            last_error: row.last_error,
        }
    }
}

const COLUMNS: &str = "account_id, host_override, port_override, tls_override, detected_host, \
    detected_port, detected_tls, detected_source, availability, implementation, extensions, \
    checked_at, enabled, server_active, activation_mode, previous_active_script, consent_script, \
    pushed_rules, pushed_vacation, last_push_at, last_error";

pub struct ServerRulesRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ServerRulesRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, account_id: &str) -> Result<Option<ServerRulesState>, sqlx::Error> {
        let row = sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLUMNS} FROM managesieve_accounts WHERE account_id = ?"
        ))
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    pub async fn get_or_empty(&self, account_id: &str) -> Result<ServerRulesState, sqlx::Error> {
        Ok(self
            .get(account_id)
            .await?
            .unwrap_or_else(|| ServerRulesState::empty(account_id)))
    }

    pub async fn list(&self) -> Result<Vec<ServerRulesState>, sqlx::Error> {
        let rows = sqlx::query_as::<_, Row>(&format!(
            "SELECT {COLUMNS} FROM managesieve_accounts ORDER BY account_id"
        ))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Write the whole row. Callers read, change and save under the push lock.
    pub async fn save(&self, state: &ServerRulesState) -> Result<(), sqlx::Error> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO managesieve_accounts (account_id, host_override, port_override, tls_override,
                 detected_host, detected_port, detected_tls, detected_source, availability, implementation,
                 extensions, checked_at, enabled, server_active, activation_mode, previous_active_script,
                 consent_script, pushed_rules, pushed_vacation, last_push_at, last_error, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id) DO UPDATE SET
                 host_override = excluded.host_override,
                 port_override = excluded.port_override,
                 tls_override = excluded.tls_override,
                 detected_host = excluded.detected_host,
                 detected_port = excluded.detected_port,
                 detected_tls = excluded.detected_tls,
                 detected_source = excluded.detected_source,
                 availability = excluded.availability,
                 implementation = excluded.implementation,
                 extensions = excluded.extensions,
                 checked_at = excluded.checked_at,
                 enabled = excluded.enabled,
                 server_active = excluded.server_active,
                 activation_mode = excluded.activation_mode,
                 previous_active_script = excluded.previous_active_script,
                 consent_script = excluded.consent_script,
                 pushed_rules = excluded.pushed_rules,
                 pushed_vacation = excluded.pushed_vacation,
                 last_push_at = excluded.last_push_at,
                 last_error = excluded.last_error,
                 updated_at = excluded.updated_at",
        )
        .bind(&state.account_id)
        .bind(&state.host_override)
        .bind(state.port_override.map(i64::from))
        .bind(state.tls_override.map(SieveTls::as_str))
        .bind(state.detected.as_ref().map(|endpoint| endpoint.host.clone()))
        .bind(state.detected.as_ref().map(|endpoint| i64::from(endpoint.port)))
        .bind(state.detected.as_ref().map(|endpoint| endpoint.tls.as_str()))
        .bind(&state.detected_source)
        .bind(&state.availability)
        .bind(&state.implementation)
        .bind(serde_json::to_string(&state.extensions).unwrap_or_else(|_| "[]".into()))
        .bind(&state.checked_at)
        .bind(i64::from(state.enabled))
        .bind(i64::from(state.server_active))
        .bind(&state.activation_mode)
        .bind(&state.previous_active_script)
        .bind(&state.consent_script)
        .bind(serde_json::to_string(&state.pushed_rules).unwrap_or_else(|_| "{}".into()))
        .bind(i64::from(state.pushed_vacation))
        .bind(&state.last_push_at)
        .bind(&state.last_error)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

/// Rule ids (with their pushed fingerprint) the server already ran for the
/// newly received message `message_id`, which the local engine must skip.
///
/// Empty unless the account's script is verifiably active. A rule counts only
/// for mail that arrived after its pushed version went live: earlier mail was
/// delivered before the server knew the rule.
pub async fn server_handled_rules(
    conn: &mut SqliteConnection,
    message_id: &str,
) -> Result<HashMap<String, String>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, Option<f64>)>(
        "SELECT s.pushed_rules, julianday(COALESCE(m.received_at, m.date))
         FROM ingest_effects e
         JOIN managesieve_accounts s ON s.account_id = e.account_id
         JOIN messages m ON m.id = e.message_id
         WHERE e.message_id = ? AND s.enabled = 1 AND s.server_active = 1",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((pushed, Some(arrived))) = row else {
        return Ok(HashMap::new());
    };
    let pushed: BTreeMap<String, PushedRule> = serde_json::from_str(&pushed).unwrap_or_default();
    Ok(pushed
        .into_iter()
        .filter(|(_, rule)| julian_day(&rule.since).is_some_and(|since| arrived >= since))
        .map(|(id, rule)| (id, rule.fingerprint))
        .collect())
}

/// SQLite's `julianday()` for an RFC 3339 timestamp.
fn julian_day(value: &str) -> Option<f64> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    Some(parsed.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn julian_day_matches_sqlite_epoch() {
        assert_eq!(julian_day("1970-01-01T00:00:00Z"), Some(2_440_587.5));
        assert!(julian_day("not a date").is_none());
    }
}
