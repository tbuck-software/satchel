//! Server-side rules over ManageSieve: status, detection, endpoint override,
//! enabling and disabling per IMAP account.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::server_rules::{ServerRulesRepository, ServerRulesState};
use satchel_core::providers::managesieve::SieveTls;
use satchel_core::server_rules::compiler::LocalReason;
use satchel_core::server_rules::service::{self, ServiceError};
use satchel_core::server_rules::ActivationChoice;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/")
)]
pub struct ServerRulesAccountDto {
    pub account_id: String,
    pub display_name: String,
    pub email: String,
    /// `unknown`, `available` or `unavailable`.
    pub availability: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `starttls` or `tls`.
    pub tls: Option<String>,
    /// Where the endpoint came from: `override`, `autoconfig` or `imap_host`.
    pub source: Option<String>,
    pub host_override: Option<String>,
    pub port_override: Option<u16>,
    pub tls_override: Option<String>,
    /// A server autoconfig names on another domain than the IMAP host; never
    /// logged into until the person accepts it as the override.
    pub suggested_host: Option<String>,
    pub suggested_port: Option<u16>,
    pub suggested_tls: Option<String>,
    pub implementation: Option<String>,
    pub extensions: Vec<String>,
    pub checked_at: Option<String>,
    pub enabled: bool,
    pub server_active: bool,
    /// `direct` or `wrapper`.
    pub activation_mode: Option<String>,
    pub previous_active_script: Option<String>,
    /// Another script is active and a push waits for the person's choice.
    pub consent_script: Option<String>,
    pub include_available: bool,
    pub vacation_available: bool,
    pub vacation_window_available: bool,
    pub pushed_vacation: bool,
    pub pushed_rule_count: u32,
    pub last_push_at: Option<String>,
    /// A message key.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/")
)]
pub struct LocalReasonDto {
    /// `trigger`, `action`, `condition`, `extension`, `folder` or `invalid`.
    pub kind: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/")
)]
pub struct RulePlacementDto {
    pub rule_id: String,
    pub server_account_ids: Vec<String>,
    pub local_reason: Option<LocalReasonDto>,
    /// Compilable, but the server does not run this version yet.
    pub pending: bool,
    /// Per action, in the rule's order. Empty when no account the rule
    /// applies to has server rules on.
    pub actions: Vec<ActionPlacementDto>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/")
)]
pub struct ActionPlacementDto {
    /// `server`: runs on the server of every account the rule applies to.
    /// `pending`: could, but this version is not verified there yet.
    /// `local`: runs in Satchel only, for `reason`.
    pub state: String,
    pub reason: Option<LocalReasonDto>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/")
)]
pub struct ServerRulesOverviewDto {
    pub accounts: Vec<ServerRulesAccountDto>,
    pub rules: Vec<RulePlacementDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/", optional_fields = nullable)
)]
pub struct ServerRulesEndpointInput {
    pub account_id: String,
    /// Empty or missing clears the override.
    pub host: Option<String>,
    pub port: Option<u16>,
    /// `starttls` or `tls`.
    pub tls: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/server_rules/", optional_fields = nullable)
)]
pub struct EnableServerRulesInput {
    pub account_id: String,
    /// `ask` never takes over another active script; `wrapper` and
    /// `replace` are the person's explicit answer to that question.
    pub choice: String,
    /// The active script the person saw when choosing. If another one is
    /// active by now, the push stops and asks again.
    #[serde(default)]
    pub expected_active: Option<String>,
}

fn account_dto(
    state: ServerRulesState,
    display_name: String,
    email: String,
) -> ServerRulesAccountDto {
    let has = |name: &str| state.extensions.iter().any(|ext| ext == name);
    let endpoint = state.override_endpoint().or_else(|| state.detected.clone());
    ServerRulesAccountDto {
        include_available: has("include"),
        vacation_available: has("vacation"),
        vacation_window_available: has("date") && has("relational"),
        account_id: state.account_id.clone(),
        display_name,
        email,
        availability: state.availability.clone(),
        host: endpoint.as_ref().map(|endpoint| endpoint.host.clone()),
        port: endpoint.as_ref().map(|endpoint| endpoint.port),
        tls: endpoint
            .as_ref()
            .map(|endpoint| endpoint.tls.as_str().to_string()),
        source: if state.host_override.is_some() {
            Some("override".to_string())
        } else {
            state.detected_source.clone()
        },
        host_override: state.host_override.clone(),
        port_override: state.port_override,
        tls_override: state.tls_override.map(|tls| tls.as_str().to_string()),
        suggested_host: state
            .suggested
            .as_ref()
            .map(|endpoint| endpoint.host.clone()),
        suggested_port: state.suggested.as_ref().map(|endpoint| endpoint.port),
        suggested_tls: state
            .suggested
            .as_ref()
            .map(|endpoint| endpoint.tls.as_str().to_string()),
        implementation: state.implementation.clone(),
        extensions: state.extensions.clone(),
        checked_at: state.checked_at.clone(),
        enabled: state.enabled,
        server_active: state.server_active,
        activation_mode: state.activation_mode.clone(),
        previous_active_script: state.previous_active_script.clone(),
        consent_script: state.consent_script.clone(),
        pushed_vacation: state.pushed_vacation,
        pushed_rule_count: u32::try_from(state.pushed_rules.len()).unwrap_or(u32::MAX),
        last_push_at: state.last_push_at.clone(),
        last_error: state.last_error.clone(),
    }
}

fn reason_dto(reason: LocalReason) -> LocalReasonDto {
    let (kind, detail) = match reason {
        LocalReason::Trigger => ("trigger", None),
        LocalReason::Action(detail) => ("action", Some(detail)),
        LocalReason::Condition(detail) => ("condition", Some(detail)),
        LocalReason::Extension(detail) => ("extension", Some(detail)),
        LocalReason::Folder => ("folder", None),
        LocalReason::Invalid => ("invalid", None),
    };
    LocalReasonDto {
        kind: kind.to_string(),
        detail,
    }
}

fn service_error(error: ServiceError) -> CommandError {
    tracing::warn!(%error, "server rules command failed");
    CommandError::new(error.message_key())
}

fn db_error(error: sqlx::Error) -> CommandError {
    service_error(ServiceError::Db(error))
}

pub async fn overview(pool: &SqlitePool) -> Result<ServerRulesOverviewDto, CommandError> {
    let imap_accounts: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, display_name, email FROM accounts WHERE provider = 'imap' ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await
    .map_err(db_error)?;
    let stored = ServerRulesRepository::new(pool)
        .list()
        .await
        .map_err(db_error)?;
    let states: Vec<ServerRulesState> = imap_accounts
        .iter()
        .map(|(id, _, _)| {
            stored
                .iter()
                .find(|state| &state.account_id == id)
                .cloned()
                .unwrap_or_else(|| ServerRulesState::empty(id))
        })
        .collect();
    let rules = service::load_rules(pool).await.map_err(db_error)?;
    let mut folders = std::collections::BTreeMap::new();
    for state in states.iter().filter(|state| state.enabled) {
        folders.insert(
            state.account_id.clone(),
            service::load_folders(pool, &state.account_id)
                .await
                .map_err(db_error)?,
        );
    }
    let placements = service::placements(&rules, &states, &folders)
        .into_iter()
        .map(|placement| RulePlacementDto {
            rule_id: placement.rule_id,
            server_account_ids: placement.server_account_ids,
            local_reason: placement.local_reason.map(reason_dto),
            pending: placement.pending,
            actions: placement
                .actions
                .into_iter()
                .map(|action| match action {
                    service::ActionPlacement::Server => ActionPlacementDto {
                        state: "server".into(),
                        reason: None,
                    },
                    service::ActionPlacement::Pending => ActionPlacementDto {
                        state: "pending".into(),
                        reason: None,
                    },
                    service::ActionPlacement::Local(reason) => ActionPlacementDto {
                        state: "local".into(),
                        reason: Some(reason_dto(reason)),
                    },
                })
                .collect(),
        })
        .collect();
    Ok(ServerRulesOverviewDto {
        accounts: states
            .into_iter()
            .zip(imap_accounts)
            .map(|(state, (_, display_name, email))| account_dto(state, display_name, email))
            .collect(),
        rules: placements,
    })
}

#[tauri::command]
pub async fn get_server_rules_overview(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<ServerRulesOverviewDto> {
    overview(pool.inner().as_ref()).await
}

async fn account_email(pool: &SqlitePool, account_id: &str) -> Result<String, CommandError> {
    sqlx::query_scalar::<_, String>("SELECT email FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| CommandError::new("backend.errors.accountNotFound"))
}

/// Probe the account's servers for ManageSieve: the override if set, else
/// what the domain's autoconfig advertises, else the IMAP host on 4190.
#[tauri::command]
pub async fn detect_server_rules(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
) -> CommandResult<ServerRulesOverviewDto> {
    let pool = pool.inner();
    let state = ServerRulesRepository::new(pool)
        .get_or_empty(&account_id)
        .await
        .map_err(db_error)?;
    let autoconfig = if state.host_override.is_some() {
        Vec::new()
    } else {
        let email = account_email(pool, &account_id).await?;
        crate::commands::onboarding::managesieve_autoconfig(&email).await
    };
    service::detect(pool, &account_id, &autoconfig)
        .await
        .map_err(service_error)?;
    overview(pool).await
}

#[tauri::command]
pub async fn set_server_rules_endpoint(
    pool: tauri::State<'_, SqlitePool>,
    input: ServerRulesEndpointInput,
) -> CommandResult<ServerRulesOverviewDto> {
    let host = input
        .host
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty());
    if let Some(host) = host.as_deref() {
        let plausible = host.len() <= 253
            && host
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'));
        if !plausible {
            return Err(CommandError::new("backend.errors.serverRules.invalidHost"));
        }
    }
    if input.port == Some(0) {
        return Err(CommandError::new("backend.errors.serverRules.invalidPort"));
    }
    let tls = match input.tls.as_deref() {
        None | Some("") => None,
        Some(value) => Some(
            SieveTls::parse(value)
                .ok_or_else(|| CommandError::new("backend.errors.serverRules.invalidTls"))?,
        ),
    };
    service::set_override(pool.inner(), &input.account_id, host, input.port, tls)
        .await
        .map_err(service_error)?;
    overview(pool.inner()).await
}

/// Turn server rules on (or push again). A push that finds another active
/// script stops and reports it in `consent_script`, unless `choice` already
/// answers the question.
#[tauri::command]
pub async fn enable_server_rules(
    pool: tauri::State<'_, SqlitePool>,
    input: EnableServerRulesInput,
) -> CommandResult<ServerRulesOverviewDto> {
    let choice = match input.choice.as_str() {
        "ask" => ActivationChoice::Ask,
        "wrapper" => ActivationChoice::Wrapper,
        "replace" => ActivationChoice::Replace,
        _ => {
            return Err(CommandError::new(
                "backend.errors.serverRules.invalidChoice",
            ))
        }
    };
    service::push_account(
        pool.inner(),
        &input.account_id,
        choice,
        input.expected_active.as_deref(),
    )
    .await
    .map_err(service_error)?;
    overview(pool.inner()).await
}

/// Turn server rules off. With `force`, the server is not contacted (it is
/// gone, or refuses the account): the record is cleared and retries stop,
/// while Satchel's scripts stay on that server. The UI asks first.
#[tauri::command]
pub async fn disable_server_rules(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
    force: Option<bool>,
) -> CommandResult<ServerRulesOverviewDto> {
    service::disable(pool.inner(), &account_id, force.unwrap_or(false))
        .await
        .map_err(service_error)?;
    overview(pool.inner()).await
}

/// Retries pushes that did not land: an account with server rules on whose
/// script is not verified active (server down, the app quit mid-push). Shortly
/// after start, then every half hour. Accounts waiting for the person's choice
/// stay waiting.
pub fn spawn_retry_worker(pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
        loop {
            service::push_pending_accounts(&pool).await;
            tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
        }
    });
}

/// The vacation reply of an IMAP account rides in its Satchel script. With
/// server rules off there is nowhere to put it; the settings page says so.
pub async fn push_vacation(
    pool: &SqlitePool,
    account_id: &str,
    wanted: bool,
) -> Result<(), CommandError> {
    let state = ServerRulesRepository::new(pool)
        .get(account_id)
        .await
        .map_err(db_error)?;
    if !state.as_ref().is_some_and(|state| state.enabled) {
        return Ok(());
    }
    let state = service::push_account(pool, account_id, ActivationChoice::Ask, None)
        .await
        .map_err(service_error)?;
    if state.consent_script.is_some() {
        return Err(CommandError::new(
            "backend.errors.serverRules.consentNeeded",
        ));
    }
    if wanted && !state.pushed_vacation {
        return Err(CommandError::new(
            "backend.errors.serverRules.vacationUnsupported",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_dto_reports_override_and_extension_flags() {
        let mut state = ServerRulesState::empty("acc");
        state.host_override = Some("sieve.example.com".into());
        state.extensions = vec!["include".into(), "vacation".into(), "date".into()];
        state.pushed_rules.insert("rule-1".into(), "f".into());
        let dto = account_dto(state, "Work".into(), "me@example.com".into());
        assert_eq!(dto.host.as_deref(), Some("sieve.example.com"));
        assert_eq!(dto.port, Some(4190));
        assert_eq!(dto.tls.as_deref(), Some("starttls"));
        assert_eq!(dto.source.as_deref(), Some("override"));
        assert!(dto.include_available && dto.vacation_available);
        assert!(!dto.vacation_window_available, "relational missing");
        assert_eq!(dto.pushed_rule_count, 1);
    }

    #[test]
    fn errors_are_message_keys() {
        let error = service_error(ServiceError::NotConfigured);
        assert_eq!(error.key, "backend.errors.serverRules.notConfigured");
    }
}
