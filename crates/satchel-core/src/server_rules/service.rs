//! Detect, push and remove server rules for an account: credentials from the
//! account, rules and the vacation reply from the database, state back into
//! `managesieve_accounts`.
//!
//! The local engine runs every rule whatever happens here, except that a
//! forward the server already sent is not sent again; see [`crate::rules`].

use sqlx::SqlitePool;

use std::collections::BTreeMap;

use super::compiler::is_satchel_script;
use super::compiler::{
    compile_script_for, CompiledScript, LocalReason, RuleCompilation, RuleSpec, Target,
    VacationCompilation, VacationSpec,
};
use super::{
    active_name, apply, exists, plan, remove, ActivationChoice, Planned, PushError, OWN_SCRIPT,
    WRAPPER_SCRIPT,
};
use crate::db::accounts::{AccountError, AccountRepository};
use crate::db::server_rules::{ServerRulesRepository, ServerRulesState};
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
    #[error("the server cannot change while server rules are on")]
    TurnOffFirst,
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
            Self::TurnOffFirst => "backend.errors.serverRules.turnOffFirst",
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
    /// The account address first, then its aliases.
    addresses: Vec<String>,
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
    let aliases: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM account_aliases WHERE account_id = ? ORDER BY is_default DESC, email",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    let mut addresses = vec![email];
    for alias in aliases {
        if !addresses
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&alias))
        {
            addresses.push(alias);
        }
    }
    Ok(Credentials {
        imap_host: config.host,
        username: config.username,
        password: config.password,
        addresses,
    })
}

/// Connect, sign in, and ask for the capabilities again: RFC 5804 lets a
/// server show more after authentication (Dovecot's per-user extensions).
async fn open(
    endpoint: &SieveEndpoint,
    creds: &Credentials,
) -> Result<ManageSieveClient, SieveError> {
    let mut client = managesieve::connect(endpoint, &creds.username).await?;
    client
        .authenticate(&creds.username, &creds.password)
        .await?;
    client.refresh_capabilities().await?;
    Ok(client)
}

/// Second-level labels under which a country code hands out names, so that
/// `example.co.uk` counts as the site and not `co.uk`. Without a public
/// suffix list this errs towards "different site", which only means asking.
const SECOND_LEVEL_SUFFIXES: [&str; 13] = [
    "co", "com", "net", "org", "ac", "gov", "edu", "gv", "or", "ne", "go", "me", "ltd",
];

fn site(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let take = match labels.as_slice() {
        [.., second, tld] if tld.len() == 2 && SECOND_LEVEL_SUFFIXES.contains(second) => 3,
        _ => 2,
    };
    labels[labels.len().saturating_sub(take)..].join(".")
}

fn is_ip_literal(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse::<std::net::IpAddr>().is_ok()
}

/// Whether two hosts belong to the same registrable domain. An IP address
/// belongs to no domain: it is never the same site as anything.
pub fn same_site(a: &str, b: &str) -> bool {
    !is_ip_literal(a) && !is_ip_literal(b) && site(a) == site(b)
}

/// Where to look, in order: the person's override alone if there is one;
/// otherwise what autoconfig advertised on the IMAP host's own domain, then
/// the IMAP host on 4190. An autoconfig server on another domain is returned
/// separately: logging in there would hand it the password unasked.
fn candidates(
    state: &ServerRulesState,
    imap_host: &str,
    autoconfig: &[SieveEndpoint],
) -> (Vec<(SieveEndpoint, &'static str)>, Option<SieveEndpoint>) {
    if let Some(endpoint) = state.override_endpoint() {
        return (vec![(endpoint, "override")], None);
    }
    let (same, foreign): (Vec<_>, Vec<_>) = autoconfig
        .iter()
        .cloned()
        .partition(|endpoint| same_site(&endpoint.host, imap_host));
    let mut list: Vec<(SieveEndpoint, &'static str)> = same
        .into_iter()
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
    (list, foreign.into_iter().next())
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
    state.checked_at = Some(chrono::Utc::now().to_rfc3339());
    let (probes, suggested) = candidates(&state, &creds.imap_host, autoconfig);
    state.suggested = suggested;
    let mut last_error = None;
    for (endpoint, source) in probes {
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
                // Found on the mail domain itself; nothing left to ask about.
                state.suggested = None;
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

/// Save (or clear, with `host: None`) the endpoint the person typed in. What
/// was learned about the old server no longer applies and is dropped; while
/// Satchel's script runs on the old server the endpoint cannot change, or
/// turning off would talk to the wrong one.
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
    if state.enabled {
        return Err(ServiceError::TurnOffFirst);
    }
    let host = host
        .map(|host| host.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|host| !host.is_empty());
    let mut fresh = ServerRulesState::empty(account_id);
    fresh.port_override = host.as_ref().and(port);
    fresh.tls_override = host.as_ref().and(tls);
    fresh.host_override = host;
    state = fresh;
    repo.save(&state).await?;
    Ok(state)
}

/// The active rules in local priority order.
pub async fn load_rules(pool: &SqlitePool) -> Result<Vec<RuleSpec>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String, Option<String>)>(
        "SELECT id, name, trigger_type, trigger_config, actions, account_id
         FROM rules WHERE is_active = 1 ORDER BY priority DESC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, name, trigger_type, trigger_config, actions, account_id)| RuleSpec {
                id,
                name,
                trigger_type,
                trigger_config,
                actions,
                account_id,
            },
        )
        .collect())
}

/// Folder id -> IMAP mailbox name of every folder of an account, the names
/// a `fileinto` can use.
pub async fn load_folders(
    pool: &SqlitePool,
    account_id: &str,
) -> Result<BTreeMap<String, String>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, COALESCE(remote_id, full_path) FROM folders WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// `since` per server rule after a push that started at `started`: a version
/// the server may already have run (verified, or written by an unverified
/// push) keeps its earliest start, every other one starts now. Whether the
/// last push was verified does not matter: an unchanged rule never "starts"
/// again.
fn rules_since(
    previous: &ServerRulesState,
    pushed: &BTreeMap<String, String>,
    started: &str,
) -> BTreeMap<String, String> {
    pushed
        .iter()
        .map(|(id, fingerprint)| {
            let kept = previous
                .live_since(id, fingerprint)
                .map(|since| since.to_rfc3339());
            (id.clone(), kept.unwrap_or_else(|| started.to_string()))
        })
        .collect()
}

/// Identifies one script's content, to recognize it when read back.
pub fn script_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content.as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn load_vacation(
    pool: &SqlitePool,
    account_id: &str,
    addresses: &[String],
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
            addresses: addresses.to_vec(),
        },
        None => VacationSpec::default(),
    })
}

/// Compile and push the account's script.
///
/// `choice` decides what happens when another script is active; background
/// pushes pass [`ActivationChoice::Ask`]. `expected_active` is the script the
/// person saw when they chose: if a different one is active now, they are
/// asked again.
pub async fn push_account(
    pool: &SqlitePool,
    account_id: &str,
    choice: ActivationChoice,
    expected_active: Option<&str>,
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
    let folders = load_folders(pool, account_id).await?;
    let vacation = load_vacation(pool, account_id, &creds.addresses).await?;
    let compiled = compile_script_for(
        &rules,
        &vacation,
        &Target {
            extensions: &state.extensions,
            account_id: Some(account_id),
            folders: &folders,
        },
    );
    // Before anything is written: the script may run from SETACTIVE on.
    let started = chrono::Utc::now().to_rfc3339();
    let previous = state.clone();

    let planned = match plan(
        &mut client,
        choice,
        state.previous_active_script.clone(),
        expected_active,
    )
    .await
    {
        Ok(Planned::Ready(planned)) => planned,
        Ok(Planned::NeedsConsent { active_script, .. }) => {
            let _ = client.logout().await;
            // Nothing was written, and another script is active: none of
            // Satchel's runs on the server. The rules run in Satchel alone.
            state.server_active = false;
            state.clear_live();
            state.pushed_vacation = false;
            state.consent_script = Some(active_script);
            state.last_error = None;
            repo.save(&state).await?;
            return Ok(state);
        }
        Err(error) => return fail(&repo, state, error).await,
    };

    // Recorded before anything is written: a push that breaks off after
    // SETACTIVE must still let turning off find and restore the right script.
    state.activation_mode = Some(planned.mode().as_str().to_string());
    state.previous_active_script = planned.previous_active();
    state.consent_script = None;
    state.server_active = false;
    let new_hash = mark_in_flight(&mut state, &compiled, &started);
    repo.save(&state).await?;

    match apply(&mut client, &planned, &compiled).await {
        Ok(warnings) => {
            let _ = client.logout().await;
            if let Some(warnings) = warnings {
                tracing::info!(%warnings, "managesieve accepted the script with warnings");
            }
            mark_verified(&mut state, &previous, &compiled, &started, new_hash);
            state.pushed_vacation = matches!(compiled.vacation, VacationCompilation::Server { .. });
            state.last_push_at = Some(chrono::Utc::now().to_rfc3339());
            state.last_error = match &compiled.vacation {
                VacationCompilation::Unsupported(_) => Some(VACATION_UNSUPPORTED.to_string()),
                _ => None,
            };
            repo.save(&state).await?;
            Ok(state)
        }
        Err(error) => fail(&repo, state, error).await,
    }
}

/// From PUTSCRIPT on, the new script may run. Until it is read back, the
/// server may run either version of a rule: the verified ones stay recorded
/// and the new ones join them as possibly live. Returns the new script's hash.
fn mark_in_flight(
    state: &mut ServerRulesState,
    compiled: &CompiledScript,
    started: &str,
) -> String {
    let hash = script_hash(&compiled.content);
    let versions: Vec<_> = compiled
        .server_rules()
        .map(
            |(rule_id, fingerprint)| crate::db::server_rules::LiveVersion {
                rule_id: rule_id.to_string(),
                fingerprint: fingerprint.to_string(),
                // A version that may run already keeps its start: adopting this
                // push later must not make an unchanged rule "new".
                since: state
                    .live_since(rule_id, fingerprint)
                    .map(|since| since.to_rfc3339())
                    .unwrap_or_else(|| started.to_string()),
                script: hash.clone(),
                forward: compiled
                    .forwards
                    .get(rule_id)
                    .map(|forward| forward_with_since(state, rule_id, forward, started)),
            },
        )
        .collect();
    for version in versions {
        if !state.possibly_live.contains(&version) {
            state.possibly_live.push(version);
        }
    }
    hash
}

/// `forward` with the start of an identical forward that may already run,
/// else `started`.
fn forward_with_since(
    state: &ServerRulesState,
    rule_id: &str,
    forward: &crate::db::server_rules::LiveForward,
    started: &str,
) -> crate::db::server_rules::LiveForward {
    crate::db::server_rules::LiveForward {
        since: state
            .forward_since(rule_id, forward)
            .unwrap_or_else(|| started.to_string()),
        ..forward.clone()
    }
}

/// The push was read back active: only its script runs now.
fn mark_verified(
    state: &mut ServerRulesState,
    previous: &ServerRulesState,
    compiled: &CompiledScript,
    started: &str,
    hash: String,
) {
    state.pushed_rules = compiled
        .server_rules()
        .map(|(id, fingerprint)| (id.to_string(), fingerprint.to_string()))
        .collect();
    state.pushed_rules_since = rules_since(previous, &state.pushed_rules, started);
    state.pushed_forwards = compiled
        .forwards
        .iter()
        .map(|(rule_id, forward)| {
            (
                rule_id.clone(),
                forward_with_since(previous, rule_id, forward, started),
            )
        })
        .collect();
    state.possibly_live.clear();
    state.script_hash = Some(hash);
    state.verified_at = Some(chrono::Utc::now().to_rfc3339());
    state.server_active = true;
}

/// Record a push that failed after connecting. The connection is dropped,
/// not logged out: after a timeout or a broken reply it is out of step.
/// `activation_mode` and `previous_active_script` stay, so turning off still
/// cleans up whatever got written. What the server may run stays recorded
/// too: the old script, and the new one if the failure came after it was
/// written. Only reading the script back ([`verify`]) narrows that down.
async fn fail(
    repo: &ServerRulesRepository<'_>,
    mut state: ServerRulesState,
    error: PushError,
) -> Result<ServerRulesState, ServiceError> {
    state.server_active = false;
    state.pushed_vacation = false;
    state.last_error = Some(error.message_key().to_string());
    repo.save(&state).await?;
    Err(error.into())
}

/// Turn server rules off: restore what was active before, delete Satchel's
/// scripts. Runs against the server whenever server rules were on, even if no
/// push ever finished: removal touches only scripts carrying Satchel's marker
/// and is safe to repeat.
///
/// `force` skips the server, for one that is gone or refuses the account:
/// the record is cleared and retries stop, while whatever Satchel wrote stays
/// on that server. The person confirms that in the UI.
pub async fn disable(
    pool: &SqlitePool,
    account_id: &str,
    force: bool,
) -> Result<ServerRulesState, ServiceError> {
    let _guard = LOCK.lock().await;
    let repo = ServerRulesRepository::new(pool);
    let mut state = repo.get_or_empty(account_id).await?;
    let endpoint = state.override_endpoint().or_else(|| state.detected.clone());
    let was_on = state.enabled || state.activation_mode.is_some();
    if let (true, false, Some(endpoint)) = (was_on, force, endpoint) {
        let creds = credentials(pool, account_id).await?;
        let outcome = async {
            let mut client = open(&endpoint, &creds).await?;
            remove(&mut client, state.previous_active_script.as_deref()).await?;
            let _ = client.logout().await;
            Ok::<_, PushError>(())
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
    if force {
        // The server was not asked: Satchel's script may still run there, so
        // its versions stay "possibly live" and their forwards stay the
        // server's. Turning server rules on and off again clears them.
        let hash = state.script_hash.clone().unwrap_or_default();
        let verified: Vec<_> = state
            .pushed_rules
            .iter()
            .map(
                |(rule_id, fingerprint)| crate::db::server_rules::LiveVersion {
                    rule_id: rule_id.clone(),
                    fingerprint: fingerprint.clone(),
                    since: state
                        .pushed_rules_since
                        .get(rule_id)
                        .cloned()
                        .unwrap_or_default(),
                    script: hash.clone(),
                    forward: state.pushed_forwards.get(rule_id).cloned(),
                },
            )
            .collect();
        state.possibly_live.extend(verified);
        state.pushed_rules.clear();
        state.pushed_forwards.clear();
        state.pushed_rules_since.clear();
    } else {
        state.clear_live();
    }
    state.pushed_vacation = false;
    state.last_error = None;
    repo.save(&state).await?;
    Ok(state)
}

/// What [`verify`] found on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verified {
    /// Satchel's verified script is the active one.
    Current,
    /// An unverified push's script is active; it is the verified one now.
    Adopted,
    /// None of Satchel's scripts is active (someone switched it off or
    /// activated another): Satchel runs every rule itself again.
    NotActive,
    /// Satchel's script is active with content it did not write. Recorded
    /// versions stay; the next push replaces it.
    Unknown,
    /// The server could not be asked; nothing changed.
    Unreachable,
    /// Server rules are off, or another check holds the lock.
    Skipped,
}

/// Read the active script back and bring what Satchel believes runs on the
/// server in line with it: the gate for forwards ([`crate::rules`]) is only
/// as good as that belief. Afterwards, a rule set that would compile to a
/// different script than the verified one (a rule or a target folder changed
/// on another path) is pushed again.
pub async fn verify(pool: &SqlitePool, account_id: &str) -> Result<Verified, ServiceError> {
    let outcome = {
        let Ok(_guard) = LOCK.try_lock() else {
            return Ok(Verified::Skipped);
        };
        let repo = ServerRulesRepository::new(pool);
        let mut state = repo.get_or_empty(account_id).await?;
        let Some(endpoint) = state.override_endpoint().or_else(|| state.detected.clone()) else {
            return Ok(Verified::Skipped);
        };
        if !state.enabled && state.possibly_live.is_empty() {
            return Ok(Verified::Skipped);
        }
        let creds = credentials(pool, account_id).await?;
        let read = async {
            let mut client = open(&endpoint, &creds).await?;
            let scripts = client.list_scripts().await?;
            let active = active_name(&scripts);
            let own = match active.as_deref() {
                Some(OWN_SCRIPT) => Some(client.get_script(OWN_SCRIPT).await?),
                Some(WRAPPER_SCRIPT) => {
                    let wrapper = client.get_script(WRAPPER_SCRIPT).await?;
                    let includes_own = is_satchel_script(&wrapper)
                        && wrapper.contains(&format!(
                            "include :personal {};",
                            super::compiler::sieve_string(OWN_SCRIPT)
                        ));
                    if includes_own && exists(&scripts, OWN_SCRIPT) {
                        Some(client.get_script(OWN_SCRIPT).await?)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let _ = client.logout().await;
            Ok::<_, PushError>(own.filter(|content| is_satchel_script(content)))
        }
        .await;
        let outcome = match read {
            Err(error) => {
                tracing::info!(%error, %account_id, "server rules could not be read back");
                return Ok(Verified::Unreachable);
            }
            Ok(active) => read_back(&mut state, active.as_deref()),
        };
        state.verified_at = Some(chrono::Utc::now().to_rfc3339());
        repo.save(&state).await?;
        outcome
    };
    if outcome != Verified::NotActive {
        if let Ok(true) = script_is_stale(pool, account_id).await {
            push_account(pool, account_id, ActivationChoice::Ask, None).await?;
        }
    }
    Ok(outcome)
}

/// Bring the record in line with Satchel's script as the server has it
/// active (`None`: none of Satchel's scripts is active).
fn read_back(state: &mut ServerRulesState, active: Option<&str>) -> Verified {
    let Some(content) = active else {
        state.clear_live();
        state.server_active = false;
        state.pushed_vacation = false;
        return Verified::NotActive;
    };
    let hash = script_hash(content);
    if state.script_hash.as_deref() == Some(hash.as_str()) {
        state.possibly_live.clear();
        state.server_active = state.enabled;
        return Verified::Current;
    }
    if !state
        .possibly_live
        .iter()
        .any(|version| version.script == hash)
    {
        // Edited by hand, or written by a push Satchel has no record of:
        // everything recorded may run; the next push replaces it.
        state.server_active = false;
        return Verified::Unknown;
    }
    let adopted: Vec<_> = state
        .possibly_live
        .iter()
        .filter(|version| version.script == hash)
        .cloned()
        .collect();
    state.pushed_rules = adopted
        .iter()
        .map(|version| (version.rule_id.clone(), version.fingerprint.clone()))
        .collect();
    state.pushed_rules_since = adopted
        .iter()
        .map(|version| (version.rule_id.clone(), version.since.clone()))
        .collect();
    state.pushed_forwards = adopted
        .iter()
        .filter_map(|version| {
            version
                .forward
                .clone()
                .map(|forward| (version.rule_id.clone(), forward))
        })
        .collect();
    state.possibly_live.clear();
    state.script_hash = Some(hash);
    state.server_active = state.enabled;
    Verified::Adopted
}

/// Whether the rules would compile to another script than the verified one.
async fn script_is_stale(pool: &SqlitePool, account_id: &str) -> Result<bool, ServiceError> {
    let state = ServerRulesRepository::new(pool)
        .get_or_empty(account_id)
        .await?;
    if !state.enabled || state.consent_script.is_some() {
        return Ok(false);
    }
    let rules = load_rules(pool).await?;
    let folders = load_folders(pool, account_id).await?;
    let addresses: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM accounts WHERE id = ?1
         UNION ALL SELECT email FROM account_aliases WHERE account_id = ?1",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    let vacation = load_vacation(pool, account_id, &addresses).await?;
    let compiled = compile_script_for(
        &rules,
        &vacation,
        &Target {
            extensions: &state.extensions,
            account_id: Some(account_id),
            folders: &folders,
        },
    );
    Ok(state.script_hash.as_deref() != Some(script_hash(&compiled.content).as_str()))
}

/// How often a sync reads the server's script back ([`verify_if_due`]).
/// Mail that arrives after someone switched Satchel's script off is left to
/// the server for at most this long.
pub const VERIFY_EVERY: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// A read-back that takes longer is abandoned; the sync goes on.
const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// [`verify`], at most every [`VERIFY_EVERY`] per account. Called when a
/// mailbox sync starts, before new mail meets the rules.
pub async fn verify_if_due(pool: &SqlitePool, account_id: &str) {
    static LAST: std::sync::Mutex<Option<std::collections::HashMap<String, std::time::Instant>>> =
        std::sync::Mutex::new(None);
    {
        let Ok(mut last) = LAST.lock() else {
            return;
        };
        let map = last.get_or_insert_with(Default::default);
        if map
            .get(account_id)
            .is_some_and(|at| at.elapsed() < VERIFY_EVERY)
        {
            return;
        }
        map.insert(account_id.to_string(), std::time::Instant::now());
    }
    match tokio::time::timeout(VERIFY_TIMEOUT, verify(pool, account_id)).await {
        Ok(Ok(outcome)) => tracing::debug!(%account_id, ?outcome, "server rules read back"),
        Ok(Err(error)) => tracing::info!(%error, %account_id, "server rules check failed"),
        Err(_) => tracing::info!(%account_id, "server rules check timed out"),
    }
}

async fn push_where(pool: &SqlitePool, wanted: impl Fn(&ServerRulesState) -> bool) {
    let states = match ServerRulesRepository::new(pool).list().await {
        Ok(states) => states,
        Err(error) => {
            tracing::warn!(%error, "server rules: could not list accounts");
            return;
        }
    };
    for state in states.into_iter().filter(|state| wanted(state)) {
        if let Err(error) = push_account(pool, &state.account_id, ActivationChoice::Ask, None).await
        {
            tracing::warn!(%error, account_id = %state.account_id, "server rules push failed");
        }
    }
}

/// Re-push every account that has server rules on, after rules changed.
/// Never takes over a script: a push that finds a foreign one stops and asks.
pub async fn push_enabled_accounts(pool: &SqlitePool) {
    push_where(pool, |state| state.enabled).await;
}

/// Whether an account's server may run something other than what Satchel
/// has now: its script was never verified active, or the last push failed
/// (an unreachable server keeps the old script, e.g. a vacation reply that was
/// switched off since). A clean push clears `last_error`. Accounts waiting
/// for the person's choice stay waiting; a reply the server cannot express
/// is not retried, since a retry would fail the same way.
pub fn needs_retry(state: &ServerRulesState) -> bool {
    let failed = state
        .last_error
        .as_deref()
        .is_some_and(|key| key != VACATION_UNSUPPORTED);
    state.enabled && state.consent_script.is_none() && (!state.server_active || failed)
}

const VACATION_UNSUPPORTED: &str = "backend.errors.serverRules.vacationUnsupported";

/// Retry accounts whose last push did not land; see [`needs_retry`].
pub async fn push_pending_accounts(pool: &SqlitePool) {
    push_where(pool, needs_retry).await;
}

/// Where a rule runs, for the badge next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePlacement {
    pub rule_id: String,
    /// Accounts whose server also runs exactly this version of the rule.
    pub server_account_ids: Vec<String>,
    /// Why it stays local on an account with server rules on, if it does.
    pub local_reason: Option<LocalReason>,
    /// Compilable, but the server does not have this version (yet).
    pub pending: bool,
    /// Per action, in the rule's order. Empty when no account the rule
    /// applies to has server rules on.
    pub actions: Vec<ActionPlacement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionPlacement {
    /// Runs on the server of every account the rule applies to.
    Server,
    /// Could run there, but this version is not verified on every one yet.
    Pending,
    /// Runs in Satchel only: this action has no Sieve form on one of the
    /// accounts, or another action keeps the whole rule local.
    Local(LocalReason),
}

static NO_FOLDERS: BTreeMap<String, String> = BTreeMap::new();

fn folders_of<'a>(
    folders: &'a BTreeMap<String, BTreeMap<String, String>>,
    account_id: &str,
) -> &'a BTreeMap<String, String> {
    folders.get(account_id).unwrap_or(&NO_FOLDERS)
}

/// Compile `rule` for one account's server.
fn compile_on(
    rule: &RuleSpec,
    state: &ServerRulesState,
    folders: &BTreeMap<String, BTreeMap<String, String>>,
) -> RuleCompilation {
    super::compiler::compile_rule_for(
        rule,
        &Target {
            extensions: &state.extensions,
            account_id: Some(&state.account_id),
            folders: folders_of(folders, &state.account_id),
        },
    )
}

/// `folders` is [`load_folders`] per account id.
pub fn placements(
    rules: &[RuleSpec],
    states: &[ServerRulesState],
    folders: &BTreeMap<String, BTreeMap<String, String>>,
) -> Vec<RulePlacement> {
    let enabled: Vec<&ServerRulesState> = states.iter().filter(|state| state.enabled).collect();
    rules
        .iter()
        .map(|rule| {
            let in_scope: Vec<&ServerRulesState> = enabled
                .iter()
                .copied()
                .filter(|state| rule.applies_to(&state.account_id))
                .collect();
            let server_account_ids: Vec<String> = in_scope
                .iter()
                .filter(|state| state.server_active)
                .filter(|state| {
                    state.pushed_rules.get(&rule.id)
                        == Some(&rule.fingerprint_for(folders_of(folders, &state.account_id)))
                })
                .map(|state| state.account_id.clone())
                .collect();
            let local_reason =
                in_scope
                    .iter()
                    .find_map(|state| match compile_on(rule, state, folders) {
                        RuleCompilation::Local(reason) => Some(reason),
                        RuleCompilation::Server { .. } => None,
                    });
            let everywhere = !in_scope.is_empty() && server_account_ids.len() == in_scope.len();
            let pending = local_reason.is_none() && !in_scope.is_empty() && !everywhere;
            let actions = if in_scope.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str::<Vec<String>>(&rule.actions)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|action| {
                        let alone = RuleSpec {
                            actions: serde_json::json!([action]).to_string(),
                            ..rule.clone()
                        };
                        let own = in_scope.iter().find_map(|state| {
                            match compile_on(&alone, state, folders) {
                                RuleCompilation::Local(reason) => Some(reason),
                                RuleCompilation::Server { .. } => None,
                            }
                        });
                        match (own, &local_reason) {
                            (Some(reason), _) => ActionPlacement::Local(reason),
                            (None, Some(reason)) => ActionPlacement::Local(reason.clone()),
                            (None, None) if everywhere => ActionPlacement::Server,
                            (None, None) => ActionPlacement::Pending,
                        }
                    })
                    .collect()
            };
            RulePlacement {
                rule_id: rule.id.clone(),
                server_account_ids,
                local_reason,
                pending,
                actions,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(host: &str) -> SieveEndpoint {
        SieveEndpoint {
            host: host.into(),
            port: DEFAULT_PORT,
            tls: SieveTls::Starttls,
        }
    }

    #[test]
    fn sites_compare_registrable_domains() {
        assert!(same_site("sieve.example.com", "imap.example.com"));
        assert!(same_site("example.com", "mail.example.com."));
        assert!(same_site("sieve.example.co.uk", "imap.example.co.uk"));
        assert!(!same_site("sieve.other.co.uk", "imap.example.co.uk"));
        assert!(!same_site("sieve.example.net", "imap.example.com"));
        assert!(same_site("sieve.gmx.de", "imap.gmx.de"));
        assert!(same_site("sieve.example.org.uk", "imap.example.org.uk"));
        assert!(!same_site("sieve.alice.me.uk", "imap.bob.me.uk"));
        assert!(!same_site("sieve.a.ltd.uk", "imap.b.ltd.uk"));
        assert!(!same_site("192.0.2.10", "192.0.2.10"));
        assert!(!same_site("[2001:db8::1]", "imap.example.com"));
    }

    #[tokio::test]
    async fn a_forced_turn_off_clears_the_record_without_the_server() {
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
        let mut state = ServerRulesState::empty("acc");
        // A server that no longer exists; the forced path must not look it up.
        state.detected = Some(endpoint("gone.invalid"));
        state.enabled = true;
        state.activation_mode = Some("direct".into());
        state.previous_active_script = Some("roundcube".into());
        state.last_error = Some("backend.errors.serverRules.unreachable".into());
        repo.save(&state).await.unwrap();

        let after = disable(&pool, "acc", true).await.unwrap();
        assert!(!after.enabled && !after.server_active);
        assert!(after.activation_mode.is_none() && after.previous_active_script.is_none());
        assert!(after.last_error.is_none());
        assert!(!needs_retry(&after), "retries stop");
        assert_eq!(
            after.detected,
            Some(endpoint("gone.invalid")),
            "detection stays"
        );
    }

    #[test]
    fn a_rule_keeps_its_live_time_while_the_server_runs_the_same_version() {
        let mut previous = ServerRulesState::empty("acc");
        previous.server_active = true;
        previous.pushed_rules = BTreeMap::from([
            ("same".to_string(), "v1".to_string()),
            ("edited".to_string(), "v1".to_string()),
        ]);
        previous.pushed_rules_since = BTreeMap::from([
            ("same".to_string(), "2026-01-01T00:00:00+00:00".to_string()),
            (
                "edited".to_string(),
                "2026-01-01T00:00:00+00:00".to_string(),
            ),
        ]);
        let pushed = BTreeMap::from([
            ("same".to_string(), "v1".to_string()),
            ("edited".to_string(), "v2".to_string()),
            ("new".to_string(), "v1".to_string()),
        ]);
        let since = rules_since(&previous, &pushed, "2026-06-01T00:00:00+00:00");
        assert_eq!(since["same"], "2026-01-01T00:00:00+00:00");
        assert_eq!(since["edited"], "2026-06-01T00:00:00+00:00");
        assert_eq!(since["new"], "2026-06-01T00:00:00+00:00");
        // A failed push in between does not restart an unchanged rule.
        previous.server_active = false;
        let since = rules_since(&previous, &pushed, "2026-06-01T00:00:00+00:00");
        assert_eq!(since["same"], "2026-01-01T00:00:00+00:00");
        // A version an unverified push may have written keeps its start too.
        previous
            .possibly_live
            .push(crate::db::server_rules::LiveVersion {
                forward: None,
                rule_id: "edited".into(),
                fingerprint: "v2".into(),
                since: "2026-03-01T00:00:00+00:00".into(),
                script: "h".into(),
            });
        let since = rules_since(&previous, &pushed, "2026-06-01T00:00:00+00:00");
        assert_eq!(since["edited"], "2026-03-01T00:00:00+00:00");
    }

    fn forwarding(rule_id: &str, to: &str) -> RuleSpec {
        RuleSpec {
            id: rule_id.into(),
            name: "Books".into(),
            trigger_type: "on_receive".into(),
            trigger_config: r#"{"from_contains":"billing@"}"#.into(),
            actions: serde_json::json!([format!("forward:{to}")]).to_string(),
            account_id: None,
        }
    }

    fn compiled_for(rules: &[RuleSpec]) -> CompiledScript {
        let extensions: Vec<String> = ["imap4flags", "copy"].map(String::from).to_vec();
        super::super::compiler::compile_script(rules, &VacationSpec::default(), &extensions)
    }

    /// A verified push of v1, then an edit to v2 whose push is in flight or
    /// failed after it may have written.
    fn after_v1_then_v2_in_flight() -> (ServerRulesState, RuleSpec, RuleSpec) {
        let v1 = forwarding("rule-1", "books@example.test");
        let v2 = forwarding("rule-1", "accounting@example.test");
        let mut state = enabled("acc", &["imap4flags", "copy"]);
        let first = compiled_for(std::slice::from_ref(&v1));
        let hash = mark_in_flight(&mut state, &first, "2026-01-01T00:00:00+00:00");
        let previous = state.clone();
        mark_verified(
            &mut state,
            &previous,
            &first,
            "2026-01-01T00:00:00+00:00",
            hash,
        );
        // The next push records itself, then breaks off (or is still running).
        state.server_active = false;
        let second = compiled_for(std::slice::from_ref(&v2));
        mark_in_flight(&mut state, &second, "2026-06-01T00:00:00+00:00");
        (state, v1, v2)
    }

    #[test]
    fn a_push_in_flight_or_failed_leaves_both_versions_live() {
        let (state, v1, v2) = after_v1_then_v2_in_flight();
        assert!(!state.server_active, "not verified");
        assert!(
            state.live_since(&v1.id, &v1.fingerprint()).is_some(),
            "the old script may still run"
        );
        assert!(
            state.live_since(&v2.id, &v2.fingerprint()).is_some(),
            "the new one may already run"
        );
        assert_eq!(
            state
                .live_since(&v1.id, &v1.fingerprint())
                .unwrap()
                .to_rfc3339(),
            "2026-01-01T00:00:00+00:00",
            "an unchanged version keeps its start"
        );
    }

    #[test]
    fn reading_the_script_back_settles_which_version_runs() {
        let (mut state, v1, v2) = after_v1_then_v2_in_flight();
        let old = compiled_for(std::slice::from_ref(&v1)).content;
        let new = compiled_for(std::slice::from_ref(&v2)).content;

        let mut kept_old = state.clone();
        assert_eq!(read_back(&mut kept_old, Some(&old)), Verified::Current);
        assert!(kept_old.live_since(&v1.id, &v1.fingerprint()).is_some());
        assert!(kept_old.live_since(&v2.id, &v2.fingerprint()).is_none());

        let mut took_new = state.clone();
        assert_eq!(read_back(&mut took_new, Some(&new)), Verified::Adopted);
        assert!(took_new.live_since(&v1.id, &v1.fingerprint()).is_none());
        assert!(took_new.live_since(&v2.id, &v2.fingerprint()).is_some());
        assert!(took_new.server_active);

        let mut edited = state.clone();
        assert_eq!(
            read_back(&mut edited, Some("# Satchel-managed script. Edits here are replaced on the next push.\r\nkeep;\r\n")),
            Verified::Unknown
        );
        assert!(edited.live_since(&v1.id, &v1.fingerprint()).is_some());
        assert!(edited.live_since(&v2.id, &v2.fingerprint()).is_some());

        // Switched off on the server (webmail activated another script):
        // Satchel takes the forwards over.
        assert_eq!(read_back(&mut state, None), Verified::NotActive);
        assert!(state.live_since(&v1.id, &v1.fingerprint()).is_none());
        assert!(state.live_since(&v2.id, &v2.fingerprint()).is_none());
    }

    #[test]
    fn an_adopted_push_keeps_the_start_of_unchanged_rules_and_forwards() {
        let books = forwarding("rule-1", "books@example.test");
        let mut state = enabled("acc", &["imap4flags", "copy"]);
        let first = compiled_for(std::slice::from_ref(&books));
        let hash = mark_in_flight(&mut state, &first, "2026-01-01T00:00:00+00:00");
        let previous = state.clone();
        mark_verified(
            &mut state,
            &previous,
            &first,
            "2026-01-01T00:00:00+00:00",
            hash,
        );
        assert_eq!(
            state.pushed_forwards["rule-1"].since,
            "2026-01-01T00:00:00+00:00"
        );

        // The same rule plus a pin, and a new rule; the push breaks off and
        // the read-back finds its script active.
        let pinned = RuleSpec {
            actions: r#"["star","forward:books@example.test"]"#.into(),
            ..books.clone()
        };
        let other = forwarding("rule-2", "accounting@example.test");
        let second = compiled_for(&[pinned.clone(), other.clone()]);
        mark_in_flight(&mut state, &second, "2026-06-01T00:00:00+00:00");
        assert_eq!(
            read_back(&mut state, Some(&second.content)),
            Verified::Adopted
        );
        assert_eq!(
            state.pushed_forwards["rule-1"].since, "2026-01-01T00:00:00+00:00",
            "the forward did not change"
        );
        assert_eq!(
            state.pushed_forwards["rule-2"].since,
            "2026-06-01T00:00:00+00:00"
        );
        assert_eq!(
            state.pushed_rules_since["rule-2"],
            "2026-06-01T00:00:00+00:00"
        );

        // An unchanged rule through an adopted push keeps its start too.
        let third = compiled_for(&[pinned.clone(), other.clone()]);
        let mut again = state.clone();
        mark_in_flight(&mut again, &third, "2026-09-01T00:00:00+00:00");
        assert_eq!(
            read_back(&mut again, Some(&third.content)),
            Verified::Current
        );
        let mut renamed = state.clone();
        let changed = compiled_for(&[pinned, forwarding("rule-3", "x@example.test"), other]);
        mark_in_flight(&mut renamed, &changed, "2026-09-01T00:00:00+00:00");
        assert_eq!(
            read_back(&mut renamed, Some(&changed.content)),
            Verified::Adopted
        );
        assert_eq!(
            renamed.pushed_rules_since["rule-2"], "2026-06-01T00:00:00+00:00",
            "unchanged across an adopted push"
        );
    }

    fn mixed_rule(account: Option<&str>) -> RuleSpec {
        RuleSpec {
            id: "rule-1".into(),
            name: "Mixed".into(),
            trigger_type: "on_receive".into(),
            trigger_config: r#"{"from_contains":"x"}"#.into(),
            actions: r#"["star","move_to_folder:f-1","forward:a@example.test","add_tag:x"]"#.into(),
            account_id: account.map(String::from),
        }
    }

    fn enabled(account: &str, extensions: &[&str]) -> ServerRulesState {
        let mut state = ServerRulesState::empty(account);
        state.enabled = true;
        state.extensions = extensions.iter().map(|ext| ext.to_string()).collect();
        state
    }

    #[test]
    fn placements_name_each_action_that_stays_local() {
        let state = enabled("acc", &["imap4flags", "fileinto"]);
        let rule = mixed_rule(Some("acc"));
        let folders = BTreeMap::from([(
            "acc".to_string(),
            BTreeMap::from([("f-1".to_string(), "Lists".to_string())]),
        )]);
        let placement = &placements(
            std::slice::from_ref(&rule),
            std::slice::from_ref(&state),
            &folders,
        )[0];
        let copy = LocalReason::Extension("copy".into());
        assert_eq!(
            placement.actions,
            vec![
                // Compilable alone, but the rule as a whole stays local.
                ActionPlacement::Local(copy.clone()),
                ActionPlacement::Local(copy.clone()),
                ActionPlacement::Local(copy.clone()),
                ActionPlacement::Local(LocalReason::Action("add_tag".into())),
            ]
        );
        assert_eq!(placement.local_reason, Some(copy));
        // A rule for another account has no badge on this one.
        let other = mixed_rule(Some("other"));
        let placement = &placements(&[other], &[state], &folders)[0];
        assert!(placement.actions.is_empty() && placement.local_reason.is_none());
    }

    #[test]
    fn an_action_says_server_only_once_every_account_runs_this_version() {
        let rule = RuleSpec {
            actions: r#"["star"]"#.into(),
            ..mixed_rule(None)
        };
        let folders = BTreeMap::new();
        let mut first = enabled("a", &["imap4flags"]);
        let mut second = enabled("b", &["imap4flags"]);
        let place = |states: &[ServerRulesState]| {
            placements(std::slice::from_ref(&rule), states, &folders)[0].clone()
        };
        assert_eq!(
            place(&[first.clone(), second.clone()]).actions,
            vec![ActionPlacement::Pending]
        );
        first.server_active = true;
        first
            .pushed_rules
            .insert(rule.id.clone(), rule.fingerprint());
        let partly = place(&[first.clone(), second.clone()]);
        assert_eq!(partly.actions, vec![ActionPlacement::Pending], "one of two");
        assert!(partly.pending);
        second.server_active = true;
        second
            .pushed_rules
            .insert(rule.id.clone(), rule.fingerprint());
        let everywhere = place(&[first.clone(), second.clone()]);
        assert_eq!(everywhere.actions, vec![ActionPlacement::Server]);
        assert!(!everywhere.pending);
        // An unscoped rule is checked against every account it applies to.
        let poor = enabled("c", &["fileinto"]);
        assert_eq!(
            place(&[first, second, poor]).actions,
            vec![ActionPlacement::Local(LocalReason::Extension(
                "imap4flags".into()
            ))]
        );
    }

    #[test]
    fn retries_cover_unverified_and_failed_pushes() {
        let mut state = ServerRulesState::empty("acc");
        assert!(!needs_retry(&state), "off");
        state.enabled = true;
        assert!(needs_retry(&state), "never verified");
        state.server_active = true;
        assert!(!needs_retry(&state), "clean");
        // Unreachable on the last push: the server still runs the old script.
        state.last_error = Some("backend.errors.serverRules.unreachable".into());
        assert!(needs_retry(&state));
        state.last_error = Some(VACATION_UNSUPPORTED.into());
        assert!(!needs_retry(&state), "would fail the same way");
        state.last_error = Some("backend.errors.serverRules.unreachable".into());
        state.consent_script = Some("roundcube".into());
        assert!(!needs_retry(&state), "waits for the person");
    }

    #[test]
    fn autoconfig_on_another_domain_is_suggested_not_probed() {
        let state = ServerRulesState::empty("acc");
        let (probes, suggested) = candidates(
            &state,
            "imap.example.com",
            &[endpoint("sieve.example.com"), endpoint("sieve.hoster.net")],
        );
        let hosts: Vec<&str> = probes.iter().map(|(e, _)| e.host.as_str()).collect();
        assert_eq!(hosts, vec!["sieve.example.com", "imap.example.com"]);
        assert_eq!(suggested, Some(endpoint("sieve.hoster.net")));
    }

    #[test]
    fn an_override_is_probed_alone() {
        let mut state = ServerRulesState::empty("acc");
        state.host_override = Some("manual.example.org".into());
        let (probes, suggested) = candidates(&state, "imap.example.com", &[endpoint("x.y.net")]);
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].1, "override");
        assert!(suggested.is_none());
    }
}
