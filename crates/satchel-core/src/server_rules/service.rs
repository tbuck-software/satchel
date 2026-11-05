//! Detect, push and remove server rules for an account: credentials from the
//! account, rules and the vacation reply from the database, state back into
//! `managesieve_accounts`.

use std::collections::BTreeMap;

use sqlx::SqlitePool;

use super::compiler::{
    compile_script, LocalReason, RuleCompilation, RuleSpec, VacationCompilation, VacationSpec,
};
use super::{push, remove, ActivationChoice, PushError, PushOutcome};
use crate::db::accounts::{AccountError, AccountRepository};
use crate::db::server_rules::{PushedRule, ServerRulesRepository, ServerRulesState};
use crate::db::vacation::VacationRepository;
use crate::providers::imap::ImapProvider;
use crate::providers::managesieve::{
    self, ManageSieveClient, SieveEndpoint, SieveError, SieveTls, DEFAULT_PORT,
};

/// One push, detection or removal at a time: they read, change and write the
/// same row, and two pushes racing on one server could interleave
/// PUTSCRIPT and SETACTIVE.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("account was not found")]
    NotFound,
    #[error("account does not use IMAP")]
    NotImap,
    #[error("no ManageSieve server is known for the account")]
    NotConfigured,
    #[error("account credentials: {0}")]
    Credentials(String),
    #[error(transparent)]
    Push(#[from] PushError),
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
}

impl From<SieveError> for ServiceError {
    fn from(error: SieveError) -> Self {
        Self::Push(PushError::Sieve(error))
    }
}

impl ServiceError {
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::NotFound => "backend.errors.accountNotFound",
            Self::NotImap => "backend.errors.accountNotImap",
            Self::NotConfigured => "backend.errors.serverRules.notConfigured",
            Self::Credentials(_) => "backend.errors.accountCredentialAccessFailed",
            Self::Push(error) => error.message_key(),
            Self::Db(_) => "backend.errors.serverRules.persistence",
        }
    }
}

struct Credentials {
    imap_host: String,
    username: String,
    password: String,
    email: String,
}

async fn credentials(pool: &SqlitePool, account_id: &str) -> Result<Credentials, ServiceError> {
    let email: Option<String> = sqlx::query_scalar("SELECT email FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    let email = email.ok_or(ServiceError::NotFound)?;
    let account = AccountRepository::new(pool)
        .load_provider_config(account_id)
        .await
        .map_err(|error| match error {
            AccountError::NotFound => ServiceError::NotFound,
            other => ServiceError::Credentials(other.to_string()),
        })?
        .ok_or(ServiceError::NotFound)?;
    if account.provider != "imap" {
        return Err(ServiceError::NotImap);
    }
    let provider = ImapProvider::from_account_config(account.config.as_str(), &email)
        .map_err(ServiceError::Credentials)?;
    let config = provider
        .config
        .ok_or_else(|| ServiceError::Credentials("missing IMAP config".into()))?;
    Ok(Credentials {
        imap_host: config.host,
        username: config.username,
        password: config.password,
        email,
    })
}

async fn open(
    endpoint: &SieveEndpoint,
    creds: &Credentials,
) -> Result<ManageSieveClient, SieveError> {
    let mut client = managesieve::connect(endpoint, &creds.username).await?;
    client
        .authenticate(&creds.username, &creds.password)
        .await?;
    Ok(client)
}

/// Where to look, in order: the person's override alone if there is one;
/// otherwise what autoconfig advertised, then the IMAP host on 4190.
fn candidates(
    state: &ServerRulesState,
    imap_host: &str,
    autoconfig: &[SieveEndpoint],
) -> Vec<(SieveEndpoint, &'static str)> {
    if let Some(endpoint) = state.override_endpoint() {
        return vec![(endpoint, "override")];
    }
    let mut list: Vec<(SieveEndpoint, &'static str)> = autoconfig
        .iter()
        .cloned()
        .map(|endpoint| (endpoint, "autoconfig"))
        .collect();
    let guess = SieveEndpoint {
        host: imap_host.to_string(),
        port: DEFAULT_PORT,
        tls: SieveTls::Starttls,
    };
    if !list.iter().any(|(endpoint, _)| *endpoint == guess) {
        list.push((guess, "imap_host"));
    }
    list
}

/// Probe for a ManageSieve server and record what was found.
pub async fn detect(
    pool: &SqlitePool,
    account_id: &str,
    autoconfig: &[SieveEndpoint],
) -> Result<ServerRulesState, ServiceError> {
    let _guard = LOCK.lock().await;
    let creds = credentials(pool, account_id).await?;
    let repo = ServerRulesRepository::new(pool);
    let mut state = repo.get_or_empty(account_id).await?;
    let now = chrono::Utc::now().to_rfc3339();
    state.checked_at = Some(now);
    let mut last_error = None;
    for (endpoint, source) in candidates(&state, &creds.imap_host, autoconfig) {
        match open(&endpoint, &creds).await {
            Ok(client) => {
                let capabilities = client.capabilities().clone();
                let _ = client.logout().await;
                state.availability = "available".into();
                state.detected = Some(endpoint);
                state.detected_source = Some(source.to_string());
                state.implementation = capabilities.implementation;
                state.extensions = capabilities.sieve;
                state.last_error = None;
                repo.save(&state).await?;
                return Ok(state);
            }
            // Wrong credentials stay wrong on the next host; stop asking.
            Err(error @ SieveError::AuthFailed(_)) => {
                tracing::warn!(%error, host = %endpoint.host, "managesieve login refused");
                state.detected = Some(endpoint);
                state.detected_source = Some(source.to_string());
                last_error = Some(error);
                break;
            }
            Err(error) => {
                tracing::info!(%error, host = %endpoint.host, port = endpoint.port, "no managesieve here");
                last_error = Some(error);
            }
        }
    }
    state.availability = "unavailable".into();
    state.last_error = last_error.map(|error| error.message_key().to_string());
    repo.save(&state).await?;
    Ok(state)
}

/// Save (or clear, with `host: None`) the endpoint the person typed in. The
/// next detection uses it alone.
pub async fn set_override(
    pool: &SqlitePool,
    account_id: &str,
    host: Option<String>,
    port: Option<u16>,
    tls: Option<SieveTls>,
) -> Result<ServerRulesState, ServiceError> {
    let _guard = LOCK.lock().await;
    let repo = ServerRulesRepository::new(pool);
    let mut state = repo.get_or_empty(account_id).await?;
    let host = host
        .map(|host| host.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|host| !host.is_empty());
    state.port_override = host.as_ref().and(port);
    state.tls_override = host.as_ref().and(tls);
    state.host_override = host;
    state.availability = "unknown".into();
    state.last_error = None;
    repo.save(&state).await?;
    Ok(state)
}

/// The active rules in local priority order.
pub async fn load_rules(pool: &SqlitePool) -> Result<Vec<RuleSpec>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT id, name, trigger_type, trigger_config, actions
         FROM rules WHERE is_active = 1 ORDER BY priority DESC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, trigger_type, trigger_config, actions)| RuleSpec {
                id,
                name,
                trigger_type,
                trigger_config,
                actions,
            },
        )
        .collect())
}

async fn load_vacation(
    pool: &SqlitePool,
    account_id: &str,
    email: &str,
) -> Result<VacationSpec, ServiceError> {
    let record = VacationRepository::new(pool)
        .get(account_id)
        .await
        .map_err(|error| match error {
            crate::db::vacation::VacationError::Persistence(error) => ServiceError::Db(error),
        })?;
    let parse = |value: Option<&str>| {
        value
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&chrono::Utc))
    };
    Ok(match record {
        Some(record) => VacationSpec {
            enabled: record.enabled == 1,
            subject: record.subject.clone(),
            body: record.body.clone(),
            starts_at: parse(record.starts_at.as_deref()),
            ends_at: parse(record.ends_at.as_deref()),
            addresses: vec![email.to_string()],
        },
        None => VacationSpec::default(),
    })
}

/// Compile and push the account's script. `choice` decides what happens when
/// another script is active; background pushes pass [`ActivationChoice::Ask`].
pub async fn push_account(
    pool: &SqlitePool,
    account_id: &str,
    choice: ActivationChoice,
) -> Result<ServerRulesState, ServiceError> {
    let _guard = LOCK.lock().await;
    let repo = ServerRulesRepository::new(pool);
    let mut state = repo.get_or_empty(account_id).await?;
    let endpoint = state
        .override_endpoint()
        .or_else(|| state.detected.clone())
        .ok_or(ServiceError::NotConfigured)?;
    let creds = credentials(pool, account_id).await?;
    state.enabled = true;

    let mut client = match open(&endpoint, &creds).await {
        Ok(client) => client,
        Err(error) => {
            // Nothing was written: whatever ran on the server still does.
            state.last_error = Some(error.message_key().to_string());
            repo.save(&state).await?;
            return Err(error.into());
        }
    };
    state.availability = "available".into();
    state.implementation = client.capabilities().implementation.clone();
    state.extensions = client.capabilities().sieve.clone();

    let rules = load_rules(pool).await?;
    let vacation = load_vacation(pool, account_id, &creds.email).await?;
    let compiled = compile_script(&rules, &vacation, &state.extensions);

    let result = push(
        &mut client,
        &compiled,
        choice,
        state.previous_active_script.clone(),
    )
    .await;
    let _ = client.logout().await;
    let now = chrono::Utc::now().to_rfc3339();
    match result {
        Ok(PushOutcome::Activated {
            mode,
            previous_active,
            warnings,
        }) => {
            if let Some(warnings) = warnings {
                tracing::info!(%warnings, "managesieve accepted the script with warnings");
            }
            let previous = std::mem::take(&mut state.pushed_rules);
            state.pushed_rules = compiled
                .server_rules()
                .map(|(id, fingerprint)| {
                    let since = previous
                        .get(id)
                        .filter(|pushed| pushed.fingerprint == fingerprint && state.server_active)
                        .map(|pushed| pushed.since.clone())
                        .unwrap_or_else(|| now.clone());
                    (
                        id.to_string(),
                        PushedRule {
                            fingerprint: fingerprint.to_string(),
                            since,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();
            state.server_active = true;
            state.activation_mode = Some(mode.as_str().to_string());
            state.previous_active_script = previous_active;
            state.consent_script = None;
            state.pushed_vacation = matches!(compiled.vacation, VacationCompilation::Server { .. });
            state.last_push_at = Some(now);
            state.last_error = match &compiled.vacation {
                VacationCompilation::Unsupported(_) => {
                    Some("backend.errors.serverRules.vacationUnsupported".to_string())
                }
                _ => None,
            };
        }
        Ok(PushOutcome::NeedsConsent { active_script, .. }) => {
            // Nothing was written; the rules run locally until a choice is made.
            state.server_active = false;
            state.pushed_rules.clear();
            state.pushed_vacation = false;
            state.consent_script = Some(active_script);
            state.last_error = None;
        }
        Err(error) => {
            // A push that broke off may have replaced the script already:
            // the local engine takes every rule back rather than skip one
            // the server no longer runs.
            state.server_active = false;
            state.pushed_rules.clear();
            state.pushed_vacation = false;
            state.last_error = Some(error.message_key().to_string());
            repo.save(&state).await?;
            return Err(error.into());
        }
    }
    repo.save(&state).await?;
    Ok(state)
}

/// Turn server rules off: restore what was active before, delete Satchel's
/// scripts, and hand every rule back to the local engine.
pub async fn disable(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<ServerRulesState, ServiceError> {
    let _guard = LOCK.lock().await;
    let repo = ServerRulesRepository::new(pool);
    let mut state = repo.get_or_empty(account_id).await?;
    if state.activation_mode.is_some() {
        let endpoint = state
            .override_endpoint()
            .or_else(|| state.detected.clone())
            .ok_or(ServiceError::NotConfigured)?;
        let creds = credentials(pool, account_id).await?;
        let outcome = async {
            let mut client = open(&endpoint, &creds).await?;
            let result = remove(&mut client, state.previous_active_script.as_deref()).await;
            let _ = client.logout().await;
            result
        }
        .await;
        if let Err(error) = outcome {
            // The script may still run: keep the record so a retry finds it.
            state.last_error = Some(error.message_key().to_string());
            repo.save(&state).await?;
            return Err(error.into());
        }
    }
    state.enabled = false;
    state.server_active = false;
    state.activation_mode = None;
    state.previous_active_script = None;
    state.consent_script = None;
    state.pushed_rules.clear();
    state.pushed_vacation = false;
    state.last_error = None;
    repo.save(&state).await?;
    Ok(state)
}

/// Re-push every account that has server rules on, after a rule changed.
/// Never takes over a script: a push that finds a foreign one stops and asks.
pub async fn push_enabled_accounts(pool: &SqlitePool) {
    let states = match ServerRulesRepository::new(pool).list().await {
        Ok(states) => states,
        Err(error) => {
            tracing::warn!(%error, "server rules: could not list accounts");
            return;
        }
    };
    for state in states.into_iter().filter(|state| state.enabled) {
        if let Err(error) = push_account(pool, &state.account_id, ActivationChoice::Ask).await {
            tracing::warn!(%error, account_id = %state.account_id, "server rules push failed");
        }
    }
}

/// Where a rule runs, for the badge next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePlacement {
    pub rule_id: String,
    /// Accounts whose server runs exactly this version of the rule.
    pub server_account_ids: Vec<String>,
    /// Why it stays local on an account with server rules on, if it does.
    pub local_reason: Option<LocalReason>,
    /// Compilable, but the server still has an older version (or none).
    pub pending: bool,
}

pub fn placements(rules: &[RuleSpec], states: &[ServerRulesState]) -> Vec<RulePlacement> {
    let enabled: Vec<&ServerRulesState> = states.iter().filter(|state| state.enabled).collect();
    rules
        .iter()
        .map(|rule| {
            let fingerprint = rule.fingerprint();
            let server_account_ids: Vec<String> = enabled
                .iter()
                .filter(|state| state.server_active)
                .filter(|state| {
                    state
                        .pushed_rules
                        .get(&rule.id)
                        .is_some_and(|pushed| pushed.fingerprint == fingerprint)
                })
                .map(|state| state.account_id.clone())
                .collect();
            let mut local_reason = None;
            let mut pending = false;
            if server_account_ids.is_empty() {
                for state in &enabled {
                    match super::compiler::compile_rule(rule, &state.extensions) {
                        RuleCompilation::Local(reason) => {
                            local_reason = Some(reason);
                            break;
                        }
                        RuleCompilation::Server { .. } => pending = true,
                    }
                }
                if local_reason.is_some() {
                    pending = false;
                }
            }
            RulePlacement {
                rule_id: rule.id.clone(),
                server_account_ids,
                local_reason,
                pending,
            }
        })
        .collect()
}
