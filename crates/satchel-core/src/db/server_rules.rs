//! Persistence for server-side rules (ManageSieve), one row per IMAP account.

use std::collections::BTreeMap;

use sqlx::{FromRow, SqlitePool};

use crate::providers::managesieve::{SieveEndpoint, SieveTls, DEFAULT_PORT};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerRulesState {
    pub account_id: String,
    pub host_override: Option<String>,
    pub port_override: Option<u16>,
    pub tls_override: Option<SieveTls>,
    pub detected: Option<SieveEndpoint>,
    pub detected_source: Option<String>,
    /// A server autoconfig named on another domain than the IMAP host. Not
    /// probed: the password would go to it. Shown for the person to accept.
    pub suggested: Option<SieveEndpoint>,
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
    /// Rule id -> fingerprint of the version in the script last verified
    /// active. Kept through a failed or unfinished push: the old script may
    /// still run. Cleared only when Satchel knows it no longer does.
    pub pushed_rules: BTreeMap<String, String>,
    /// Rule id -> since when the server runs the version in `pushed_rules`
    /// (RFC 3339, taken before the push that first made it live).
    pub pushed_rules_since: BTreeMap<String, String>,
    /// Versions a push wrote or was writing that were never verified active:
    /// an unfinished push, or one that failed after it may have written.
    /// Until [`crate::server_rules::service::verify`] reads the server's
    /// script back, the server may run them as well as `pushed_rules`.
    pub possibly_live: Vec<LiveVersion>,
    /// Rule id -> the forward of the verified version: what the local engine
    /// compares against, not the whole rule (an added flag does not stop the
    /// server forwarding).
    pub pushed_forwards: BTreeMap<String, LiveForward>,
    /// Hash of Satchel's script as last verified active.
    pub script_hash: Option<String>,
    /// When the active script was last read back.
    pub verified_at: Option<String>,
    pub pushed_vacation: bool,
    pub last_push_at: Option<String>,
    /// A message key.
    pub last_error: Option<String>,
}

/// One version of a rule a push may have put on the server.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LiveVersion {
    pub rule_id: String,
    pub fingerprint: String,
    /// RFC 3339, since when this version (or an identical earlier one) runs.
    pub since: String,
    /// Hash of the script it was part of.
    pub script: String,
    /// Its forward, if it forwards.
    #[serde(default)]
    pub forward: Option<LiveForward>,
}

/// What decides whether the server forwards a message: the rule's trigger and
/// the address. Two versions that differ only elsewhere (a flag, a move) are
/// the same forward.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LiveForward {
    pub trigger_type: String,
    /// The raw JSON column.
    pub trigger_config: String,
    pub to: String,
    /// RFC 3339, since when this forward runs on the server.
    pub since: String,
}

impl LiveForward {
    pub fn same_forward(&self, trigger_type: &str, trigger_config: &str, to: &str) -> bool {
        self.trigger_type == trigger_type
            && self.trigger_config == trigger_config
            && self.to.trim().eq_ignore_ascii_case(to.trim())
    }
}

impl ServerRulesState {
    /// Since when a version of the rule identical to `fingerprint` may run on
    /// the server: the verified one, or one an unverified push may have
    /// written. `None` when no such version may run. The earliest start wins.
    pub fn live_since(
        &self,
        rule_id: &str,
        fingerprint: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let parse = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&chrono::Utc))
                // A version pushed before Satchel recorded starts: since always.
                .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
        };
        let verified = (self.pushed_rules.get(rule_id).map(String::as_str) == Some(fingerprint))
            .then(|| {
                self.pushed_rules_since
                    .get(rule_id)
                    .map(|since| parse(since))
                    .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
            });
        self.possibly_live
            .iter()
            .filter(|version| version.rule_id == rule_id && version.fingerprint == fingerprint)
            .map(|version| parse(&version.since))
            .chain(verified)
            .min()
    }

    /// Every forward of the rule that may run on the server: verified, or
    /// written by an unverified push.
    pub fn live_forwards(&self, rule_id: &str) -> Vec<&LiveForward> {
        self.pushed_forwards
            .get(rule_id)
            .into_iter()
            .chain(
                self.possibly_live
                    .iter()
                    .filter(|version| version.rule_id == rule_id)
                    .filter_map(|version| version.forward.as_ref()),
            )
            .collect()
    }

    /// Since when an identical forward may run on the server (earliest).
    pub fn forward_since(&self, rule_id: &str, forward: &LiveForward) -> Option<String> {
        self.live_forwards(rule_id)
            .into_iter()
            .filter(|live| {
                live.same_forward(&forward.trigger_type, &forward.trigger_config, &forward.to)
            })
            .map(|live| live.since.clone())
            .min_by_key(|since| {
                chrono::DateTime::parse_from_rfc3339(since)
                    .map(|value| value.timestamp())
                    .unwrap_or(i64::MIN)
            })
    }

    /// Forget every version: Satchel knows its script does not run.
    pub fn clear_live(&mut self) {
        self.pushed_rules.clear();
        self.pushed_rules_since.clear();
        self.pushed_forwards.clear();
        self.possibly_live.clear();
        self.script_hash = None;
    }

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
    suggested_host: Option<String>,
    suggested_port: Option<i64>,
    suggested_tls: Option<String>,
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
    pushed_rules_since: String,
    possibly_live_rules: String,
    pushed_forwards: String,
    script_hash: Option<String>,
    verified_at: Option<String>,
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
        let suggested = match (row.suggested_host, port(row.suggested_port)) {
            (Some(host), Some(port)) => Some(SieveEndpoint {
                host,
                port,
                tls: row
                    .suggested_tls
                    .as_deref()
                    .and_then(SieveTls::parse)
                    .unwrap_or(SieveTls::Starttls),
            }),
            _ => None,
        };
        Self {
            account_id: row.account_id,
            suggested,
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
            pushed_rules_since: serde_json::from_str(&row.pushed_rules_since).unwrap_or_default(),
            possibly_live: serde_json::from_str(&row.possibly_live_rules).unwrap_or_default(),
            pushed_forwards: serde_json::from_str(&row.pushed_forwards).unwrap_or_default(),
            script_hash: row.script_hash,
            verified_at: row.verified_at,
            pushed_vacation: row.pushed_vacation == 1,
            last_push_at: row.last_push_at,
            last_error: row.last_error,
        }
    }
}

const COLUMNS: &str = "account_id, host_override, port_override, tls_override, detected_host, \
    detected_port, detected_tls, detected_source, suggested_host, suggested_port, suggested_tls, availability, implementation, extensions, \
    checked_at, enabled, server_active, activation_mode, previous_active_script, consent_script, \
    pushed_rules, pushed_rules_since, possibly_live_rules, pushed_forwards, script_hash, verified_at, \
    pushed_vacation, last_push_at, last_error";

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
                 detected_host, detected_port, detected_tls, detected_source, suggested_host, suggested_port,
                 suggested_tls, availability, implementation,
                 extensions, checked_at, enabled, server_active, activation_mode, previous_active_script,
                 consent_script, pushed_rules, pushed_rules_since, possibly_live_rules, pushed_forwards,
                 script_hash, verified_at, pushed_vacation, last_push_at, last_error, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id) DO UPDATE SET
                 host_override = excluded.host_override,
                 port_override = excluded.port_override,
                 tls_override = excluded.tls_override,
                 detected_host = excluded.detected_host,
                 detected_port = excluded.detected_port,
                 detected_tls = excluded.detected_tls,
                 detected_source = excluded.detected_source,
                 suggested_host = excluded.suggested_host,
                 suggested_port = excluded.suggested_port,
                 suggested_tls = excluded.suggested_tls,
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
                 pushed_rules_since = excluded.pushed_rules_since,
                 possibly_live_rules = excluded.possibly_live_rules,
                 pushed_forwards = excluded.pushed_forwards,
                 script_hash = excluded.script_hash,
                 verified_at = excluded.verified_at,
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
        .bind(state.suggested.as_ref().map(|endpoint| endpoint.host.clone()))
        .bind(state.suggested.as_ref().map(|endpoint| i64::from(endpoint.port)))
        .bind(state.suggested.as_ref().map(|endpoint| endpoint.tls.as_str()))
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
        .bind(serde_json::to_string(&state.pushed_rules_since).unwrap_or_else(|_| "{}".into()))
        .bind(serde_json::to_string(&state.possibly_live).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&state.pushed_forwards).unwrap_or_else(|_| "{}".into()))
        .bind(&state.script_hash)
        .bind(&state.verified_at)
        .bind(i64::from(state.pushed_vacation))
        .bind(&state.last_push_at)
        .bind(&state.last_error)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_state_round_trips() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repo = ServerRulesRepository::new(&pool);
        assert!(repo.get("acc").await.unwrap().is_none());
        let mut state = ServerRulesState::empty("acc");
        state.host_override = Some("sieve.example.com".into());
        state.port_override = Some(2000);
        state.tls_override = Some(SieveTls::Implicit);
        state.suggested = Some(SieveEndpoint {
            host: "sieve.other.example".into(),
            port: 4190,
            tls: SieveTls::Starttls,
        });
        state.extensions = vec!["vacation".into()];
        state.enabled = true;
        state.activation_mode = Some("wrapper".into());
        state.previous_active_script = Some("roundcube".into());
        state.pushed_rules.insert("rule-1".into(), "abc".into());
        state
            .pushed_rules_since
            .insert("rule-1".into(), "2026-06-01T10:00:00+00:00".into());
        state.possibly_live.push(LiveVersion {
            rule_id: "rule-1".into(),
            fingerprint: "def".into(),
            since: "2026-06-02T10:00:00+00:00".into(),
            script: "hash-2".into(),
            forward: None,
        });
        state.pushed_forwards.insert(
            "rule-1".into(),
            LiveForward {
                trigger_type: "on_receive".into(),
                trigger_config: "{}".into(),
                to: "books@example.test".into(),
                since: "2026-06-01T10:00:00+00:00".into(),
            },
        );
        state.script_hash = Some("hash-1".into());
        repo.save(&state).await.unwrap();
        assert_eq!(repo.get("acc").await.unwrap(), Some(state.clone()));
        assert_eq!(repo.list().await.unwrap(), vec![state]);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    fn at(value: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn a_version_is_live_if_verified_or_possibly_written() {
        let mut state = ServerRulesState::empty("acc");
        assert_eq!(state.live_since("r", "v1"), None);
        state.pushed_rules.insert("r".into(), "v1".into());
        state
            .pushed_rules_since
            .insert("r".into(), "2026-06-01T00:00:00+00:00".into());
        assert_eq!(
            state.live_since("r", "v1"),
            Some(at("2026-06-01T00:00:00Z"))
        );
        assert_eq!(
            state.live_since("r", "v2"),
            None,
            "an edit is another version"
        );
        state.possibly_live.push(LiveVersion {
            rule_id: "r".into(),
            fingerprint: "v2".into(),
            since: "2026-06-03T00:00:00+00:00".into(),
            script: "h2".into(),
            forward: None,
        });
        assert_eq!(
            state.live_since("r", "v2"),
            Some(at("2026-06-03T00:00:00Z")),
            "a push that may have written v2"
        );
        assert_eq!(
            state.live_since("r", "v1"),
            Some(at("2026-06-01T00:00:00Z"))
        );
        // Live without a recorded start: since always.
        state.pushed_rules_since.clear();
        assert_eq!(
            state.live_since("r", "v1"),
            Some(chrono::DateTime::<chrono::Utc>::MIN_UTC)
        );
        state.clear_live();
        assert_eq!(state.live_since("r", "v1"), None);
        assert_eq!(state.live_since("r", "v2"), None);
    }
}
