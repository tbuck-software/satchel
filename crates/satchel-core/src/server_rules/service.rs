//! Detect, push and remove server rules for an account: credentials from the
//! account, rules and the vacation reply from the database, state back into
//! `managesieve_accounts`.
//!
//! The local engine runs every rule whatever happens here; see the module
//! documentation of [`super`] for why, and for what a non-idempotent action
//! would need first.

use sqlx::SqlitePool;

use super::compiler::{
    compile_script, LocalReason, RuleCompilation, RuleSpec, VacationCompilation, VacationSpec,
};
use super::{apply, plan, remove, ActivationChoice, Planned, PushError};
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
    let vacation = load_vacation(pool, account_id, &creds.addresses).await?;
    let compiled = compile_script(&rules, &vacation, &state.extensions);

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
            // Nothing was written; the rules run in Satchel as always.
            state.server_active = false;
            state.pushed_rules.clear();
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
    repo.save(&state).await?;

    match apply(&mut client, &planned, &compiled).await {
        Ok(warnings) => {
            let _ = client.logout().await;
            if let Some(warnings) = warnings {
                tracing::info!(%warnings, "managesieve accepted the script with warnings");
            }
            state.pushed_rules = compiled
                .server_rules()
                .map(|(id, fingerprint)| (id.to_string(), fingerprint.to_string()))
                .collect();
            state.server_active = true;
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

/// Record a push that failed after connecting. The connection is dropped,
/// not logged out: after a timeout or a broken reply it is out of step.
/// `activation_mode` and `previous_active_script` stay, so turning off still
/// cleans up whatever got written.
async fn fail(
    repo: &ServerRulesRepository<'_>,
    mut state: ServerRulesState,
    error: PushError,
) -> Result<ServerRulesState, ServiceError> {
    state.server_active = false;
    state.pushed_rules.clear();
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
    state.pushed_rules.clear();
    state.pushed_vacation = false;
    state.last_error = None;
    repo.save(&state).await?;
    Ok(state)
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
                .filter(|state| state.pushed_rules.get(&rule.id) == Some(&fingerprint))
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
