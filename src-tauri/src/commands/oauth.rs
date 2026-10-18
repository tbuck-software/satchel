use chrono::{DateTime, Utc};
use satchel_core::db::{
    accounts::{AccountRepository, OAuthAccountStatus},
    oauth::{
        NewOAuthFlowSession, OAuthFlowError, OAuthFlowProgress, OAuthFlowRepository,
        OAuthFlowStatus,
    },
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

use crate::{
    commands::error::{CommandError, CommandResult},
    utils::oauth,
    AppState, OAuthFlowResult,
};

/// The default callback transport. Explicit loopback registrations also need
/// a listener on mobile; other mobile flows use the registered deep link.
#[cfg(desktop)]
const CALLBACK_PLATFORM: oauth::OAuthPlatform = oauth::OAuthPlatform::Desktop;
#[cfg(mobile)]
const CALLBACK_PLATFORM: oauth::OAuthPlatform = oauth::OAuthPlatform::Mobile;

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/", optional_fields = nullable))]
pub struct StartOAuthInput {
    pub account_id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/"))]
pub struct StartOAuthResponse {
    pub authorization_url: String,
    pub state: String,
    pub expires_at: String,
    pub browser_opened: bool,
    pub browser_error: Option<String>,
}

#[derive(Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/", optional_fields = nullable))]
pub struct CompleteOAuthInput {
    pub state: String,
    pub code: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/", optional_fields = nullable))]
pub struct PollOAuthInput {
    pub state: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/", optional_fields = nullable))]
pub struct AbortOAuthInput {
    pub state: String,
}

/// A flow a restarted app could still finish.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/"))]
pub struct OpenOAuthFlow {
    pub state: String,
    pub account_id: String,
    pub provider: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/"))]
pub struct OAuthStatus {
    pub account_id: String,
    pub provider: String,
    pub connected: bool,
    pub has_refresh_token: bool,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/oauth/"))]
pub struct OAuthFlowPollResponse {
    pub status: String,
    pub result: Option<OAuthStatus>,
    pub error: Option<String>,
}

impl From<OAuthAccountStatus> for OAuthStatus {
    fn from(status: OAuthAccountStatus) -> Self {
        Self {
            account_id: status.account_id,
            provider: status.provider,
            connected: status.connected,
            has_refresh_token: status.has_refresh_token,
            expires_at: status.expires_at,
        }
    }
}

impl From<OAuthFlowStatus> for OpenOAuthFlow {
    fn from(status: OAuthFlowStatus) -> Self {
        Self {
            state: status.state,
            account_id: status.account_id,
            provider: status.provider,
            created_at: status.created_at.to_rfc3339(),
            expires_at: status.expires_at.to_rfc3339(),
        }
    }
}

#[derive(Default)]
struct OAuthCallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[tauri::command]
pub async fn start_oauth_flow(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: StartOAuthInput,
) -> CommandResult<StartOAuthResponse> {
    let provider = oauth::OAuthProvider::parse(&input.provider)
        .map_err(|error| oauth_configuration_error("unsupported OAuth provider", error))?;
    let mut provider_config = provider
        .load_config()
        .map_err(|error| oauth_configuration_error("could not load OAuth provider", error))?;
    if !AccountRepository::new(pool.inner())
        .exists(&input.account_id)
        .await
        .map_err(CommandError::from)?
    {
        return Err(CommandError::new("backend.errors.accountNotFound"));
    }

    let configured_redirect_uri = provider.configured_redirect_uri();

    // Bind before publishing the authorization URL so the browser cannot
    // arrive at an address with no listener behind it.
    let listener = bind_callback_listener(CALLBACK_PLATFORM, configured_redirect_uri.as_deref())
        .await
        .map_err(|error| oauth_runtime_error("could not bind OAuth callback listener", error))?;
    let callback_port = listener
        .as_ref()
        .map(TcpListener::local_addr)
        .transpose()
        .map_err(|error| oauth_runtime_error("could not read OAuth callback port", error))?
        .map_or(0, |address| address.port());

    provider_config.redirect_uri = oauth::redirect_uri_for(
        CALLBACK_PLATFORM,
        configured_redirect_uri.as_deref(),
        callback_port,
    );
    let redirect_uri = provider_config.redirect_uri.clone();

    let oauth_state = oauth::generate_state();
    let code_verifier = oauth::generate_code_verifier();
    let code_challenge = oauth::code_challenge_s256(&code_verifier);
    let authorization_url =
        oauth::build_authorization_url(&provider_config, &oauth_state, &code_challenge).map_err(
            |error| oauth_configuration_error("could not build OAuth authorization URL", error),
        )?;

    let created_at = chrono::Utc::now();
    let expires_at = created_at + chrono::Duration::minutes(15);
    OAuthFlowRepository::new(pool.inner())
        .create(NewOAuthFlowSession {
            state: oauth_state.clone(),
            account_id: input.account_id,
            provider: provider.as_str().to_string(),
            code_verifier,
            created_at,
            expires_at,
            redirect_uri,
        })
        .await
        .map_err(oauth_flow_command_error)?;
    state
        .oauth_results
        .lock()
        .await
        .insert(oauth_state.clone(), OAuthFlowResult::Pending);

    if let Some(listener) = listener {
        let app_state = state.inner().clone();
        let db = pool.inner().clone();
        let listener_state = oauth_state.clone();
        tauri::async_runtime::spawn(async move {
            listen_for_oauth_callback(listener, app_state, db, listener_state).await;
        });
    }

    let browser_result = webbrowser::open(&authorization_url);
    let (browser_opened, browser_error) = match browser_result {
        Ok(_) => (true, None),
        Err(error) => {
            tracing::warn!(%error, "could not open the OAuth authorization URL");
            (
                false,
                Some("backend.errors.oauthBrowserOpenFailed".to_string()),
            )
        }
    };

    Ok(StartOAuthResponse {
        authorization_url,
        state: oauth_state,
        expires_at: expires_at.to_rfc3339(),
        browser_opened,
        browser_error,
    })
}

#[tauri::command]
pub async fn complete_oauth_flow(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: CompleteOAuthInput,
) -> CommandResult<OAuthStatus> {
    let flow_state = input.state.clone();
    match complete_oauth_flow_inner(state.inner(), pool.inner(), input).await {
        Ok(status) => Ok(status),
        Err(err) => {
            fail_oauth_flow(state.inner(), pool.inner(), &flow_state, &err).await;
            Err(err)
        }
    }
}

/// Where a flow stands.
///
/// The in-memory map answers the common case in one lock; everything else is
/// reconstructed from the database, because a process death between the
/// authorization and the callback wipes the map but not the flow.
#[tauri::command]
pub async fn poll_oauth_flow(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: PollOAuthInput,
) -> CommandResult<OAuthFlowPollResponse> {
    let stored = state.oauth_results.lock().await.get(&input.state).cloned();
    match stored {
        Some(result) => Ok(poll_response_from_memory(result)),
        None => resolve_persisted_oauth_flow(pool.inner(), &input.state, chrono::Utc::now()).await,
    }
}

/// Flows a freshly started app could still finish, newest first.
#[tauri::command]
pub async fn list_open_oauth_flows(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<OpenOAuthFlow>> {
    let flows = OAuthFlowRepository::new(pool.inner().as_ref())
        .open_flows(chrono::Utc::now())
        .await
        .map_err(oauth_flow_command_error)?;
    Ok(flows.into_iter().map(Into::into).collect())
}

/// End a flow the provider refused or the user cancelled.
///
/// Without this a deep link carrying `error=access_denied` leaves the flow
/// looking pending: the caller would watch a spinner until it times out.
#[tauri::command]
pub async fn abort_oauth_flow(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: AbortOAuthInput,
) -> CommandResult<()> {
    fail_oauth_flow(
        state.inner(),
        pool.inner(),
        &input.state,
        &CommandError::new("backend.errors.oauthAuthorizationCancelled"),
    )
    .await;
    Ok(())
}

fn poll_response_from_memory(stored: OAuthFlowResult) -> OAuthFlowPollResponse {
    match stored {
        OAuthFlowResult::Pending => pending_poll_response(),
        OAuthFlowResult::Completed {
            account_id,
            provider,
            connected,
            has_refresh_token,
            expires_at,
        } => OAuthFlowPollResponse {
            status: "completed".to_string(),
            result: Some(OAuthStatus {
                account_id,
                provider,
                connected,
                has_refresh_token,
                expires_at,
            }),
            error: None,
        },
        OAuthFlowResult::Failed(error) => failed_poll_response(error),
    }
}

fn pending_poll_response() -> OAuthFlowPollResponse {
    OAuthFlowPollResponse {
        status: "pending".to_string(),
        result: None,
        error: None,
    }
}

fn failed_poll_response(error: String) -> OAuthFlowPollResponse {
    OAuthFlowPollResponse {
        status: "failed".to_string(),
        result: None,
        error: Some(error),
    }
}

/// Answer a poll for a flow this process has never seen.
async fn resolve_persisted_oauth_flow(
    pool: &SqlitePool,
    flow_state: &str,
    now: DateTime<Utc>,
) -> CommandResult<OAuthFlowPollResponse> {
    let status = OAuthFlowRepository::new(pool)
        .status(flow_state, now)
        .await
        .map_err(oauth_flow_command_error)?
        .ok_or_else(|| CommandError::new("backend.errors.oauthFlowNotFound"))?;

    match status.progress {
        OAuthFlowProgress::Authorizing => Ok(pending_poll_response()),
        OAuthFlowProgress::Failed(error) => Ok(failed_poll_response(error)),
        // The verifier was spent, so an exchange ran. Whether it landed is
        // written on the account, not on the flow.
        OAuthFlowProgress::Exchanging => {
            let connected = AccountRepository::new(pool)
                .oauth_status(&status.account_id)
                .await
                .ok()
                .filter(|account| oauth_connected_since(account, status.created_at));
            match connected {
                Some(account) => Ok(OAuthFlowPollResponse {
                    status: "completed".to_string(),
                    result: Some(account.into()),
                    error: None,
                }),
                // Still exchanging, or the process died mid-exchange. Only the
                // session deadline tells the two apart.
                None if now <= status.expires_at => Ok(pending_poll_response()),
                None => Ok(failed_poll_response(
                    CommandError::new("backend.errors.oauthFlowNotFound").encoded(),
                )),
            }
        }
    }
}

/// Whether the account carries credentials this flow could have established.
///
/// An account reconnecting itself already had credentials; only ones written
/// after the flow began prove that this flow finished.
fn oauth_connected_since(account: &OAuthAccountStatus, started_at: DateTime<Utc>) -> bool {
    account.connected
        && account
            .connected_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .is_some_and(|connected_at| connected_at.with_timezone(&Utc) >= started_at)
}

#[tauri::command]
pub async fn oauth_status(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: String,
) -> CommandResult<OAuthStatus> {
    AccountRepository::new(pool.inner().as_ref())
        .oauth_status(&account_id)
        .await
        .map(Into::into)
        .map_err(CommandError::from)
}

async fn complete_oauth_flow_inner(
    state: &AppState,
    pool: &SqlitePool,
    input: CompleteOAuthInput,
) -> CommandResult<OAuthStatus> {
    let flow_state = input.state.clone();
    let session = OAuthFlowRepository::new(pool)
        .consume(&flow_state, chrono::Utc::now())
        .await
        .map_err(oauth_flow_command_error)?;

    let provider = oauth::OAuthProvider::parse(&session.provider)
        .map_err(|error| oauth_configuration_error("invalid stored OAuth provider", error))?;
    let mut provider_config = provider
        .load_config()
        .map_err(|error| oauth_configuration_error("could not load OAuth provider", error))?;
    // The exchange must repeat the address the authorization used, character
    // for character, or the provider answers redirect_uri_mismatch. Since that
    // address names a per-flow port, re-deriving it here would be a coin flip.
    if let Some(redirect_uri) = session.redirect_uri.clone() {
        provider_config.redirect_uri = redirect_uri;
    }
    let token =
        oauth::exchange_code_for_token(&provider_config, &input.code, &session.code_verifier)
            .await
            .map_err(|error| oauth_provider_error("OAuth token exchange failed", error))?;
    let identity = oauth::fetch_identity(provider, &token.access_token)
        .await
        .map_err(|error| oauth_provider_error("OAuth identity lookup failed", error))?;
    let now = chrono::Utc::now().to_rfc3339();
    let target_account = AccountRepository::new(pool)
        .connect_oauth(
            &session.account_id,
            &provider_config,
            &token,
            &identity,
            &now,
        )
        .await
        .map_err(CommandError::from)?;

    let status = OAuthStatus {
        account_id: target_account.account_id,
        provider: target_account.provider,
        connected: true,
        has_refresh_token: target_account.has_refresh_token,
        expires_at: target_account.expires_at,
    };
    restart_account_poller(state, &status.account_id);
    mark_oauth_completed(state, &flow_state, &status).await;
    if let Err(error) = OAuthFlowRepository::new(pool).complete(&flow_state).await {
        tracing::warn!(error = ?error, "finished OAuth flow could not be cleared");
    }

    Ok(status)
}

fn restart_account_poller(state: &AppState, account_id: &str) {
    if let Some(idle) = state.idle_control.get() {
        idle.add_account(account_id);
    }
}

fn oauth_flow_command_error(error: OAuthFlowError) -> CommandError {
    match error {
        OAuthFlowError::NotFoundOrExpired => CommandError::new("backend.errors.oauthFlowNotFound"),
        OAuthFlowError::CredentialAccess { detail } => {
            tracing::error!(%detail, "OAuth verifier could not be protected or unlocked");
            CommandError::new("backend.errors.oauthCredentialAccessFailed")
        }
        OAuthFlowError::InvalidSession { detail } => {
            tracing::error!(%detail, "stored OAuth flow is invalid");
            CommandError::new("backend.errors.oauthFlowInvalid")
        }
        OAuthFlowError::Persistence(error) => {
            tracing::error!(error = ?error, "OAuth flow persistence failed");
            CommandError::new("backend.errors.oauthPersistenceFailed")
        }
    }
}

async fn bind_callback_listener(
    platform: oauth::OAuthPlatform,
    configured_uri: Option<&str>,
) -> std::io::Result<Option<TcpListener>> {
    let pinned = configured_uri.and_then(oauth::pinned_loopback_port);
    if platform == oauth::OAuthPlatform::Mobile && pinned.is_none() {
        return Ok(None);
    }
    let address = configured_uri
        .and_then(|uri| reqwest::Url::parse(uri).ok())
        .filter(|uri| uri.host_str() == Some("[::1]"))
        .map_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), |_| {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        });
    TcpListener::bind((address, pinned.unwrap_or(0)))
        .await
        .map(Some)
}

fn oauth_runtime_error(context: &'static str, error: impl std::fmt::Display) -> CommandError {
    tracing::error!(%error, %context, "OAuth callback listener failed");
    CommandError::new("backend.errors.oauthCallbackUnavailable")
}

fn oauth_configuration_error(context: &'static str, error: String) -> CommandError {
    tracing::error!(%error, %context, "OAuth configuration failed");
    CommandError::new("backend.errors.oauthConfigurationInvalid")
}

fn oauth_provider_error(context: &'static str, error: String) -> CommandError {
    tracing::error!(%error, %context, "OAuth provider request failed");
    CommandError::new("backend.errors.oauthProviderRequestFailed")
}

async fn listen_for_oauth_callback(
    listener: TcpListener,
    state: AppState,
    pool: SqlitePool,
    expected_state: String,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    match wait_for_callback(
        &listener,
        &expected_state,
        deadline,
        std::time::Duration::from_secs(3),
    )
    .await
    {
        Ok((mut stream, query)) => {
            let response = handle_callback_query(&state, &pool, &expected_state, query).await;
            let _ = timeout(std::time::Duration::from_secs(3), async {
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await
            })
            .await;
        }
        Err(error) => fail_oauth_flow(&state, &pool, &expected_state, &error).await,
    }
}

async fn wait_for_callback(
    listener: &TcpListener,
    expected_state: &str,
    deadline: tokio::time::Instant,
    read_timeout: std::time::Duration,
) -> Result<(tokio::net::TcpStream, OAuthCallbackQuery), CommandError> {
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(CommandError::new("backend.errors.oauthCallbackTimedOut"));
        }

        let (mut stream, _) = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .map_err(|_| CommandError::new("backend.errors.oauthCallbackTimedOut"))?
            .map_err(|_| CommandError::new("backend.errors.oauthCallbackUnavailable"))?;
        let request_deadline = deadline.min(tokio::time::Instant::now() + read_timeout);
        let query =
            tokio::time::timeout_at(request_deadline, read_callback_query(&mut stream)).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(CommandError::new("backend.errors.oauthCallbackTimedOut"));
        }
        if let Ok(Ok(query)) = query {
            if query.state.as_deref() == Some(expected_state) {
                return Ok((stream, query));
            }
        }
        // Stray requests and idle preconnects do not consume the OAuth flow.
        let _ = tokio::time::timeout_at(
            request_deadline,
            stream.write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            ),
        )
        .await;
    }
}

async fn handle_callback_query(
    state: &AppState,
    pool: &SqlitePool,
    expected_state: &str,
    query: OAuthCallbackQuery,
) -> String {
    let strings = crate::commands::ui_strings::current();
    if query.state.as_deref() != Some(expected_state) {
        return http_response(
            400,
            "Bad Request",
            &render_oauth_page(
                &strings.oauth_invalid_title,
                &strings.oauth_invalid_state,
                false,
            ),
        );
    }

    if let Some(error) = query.error {
        let detail = query
            .error_description
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(error);
        tracing::info!(%detail, "OAuth authorization was not completed");
        fail_oauth_flow(
            state,
            pool,
            expected_state,
            &CommandError::new("backend.errors.oauthAuthorizationCancelled"),
        )
        .await;
        return http_response(
            400,
            "Bad Request",
            &render_oauth_page(
                &strings.oauth_cancelled_title,
                &strings.oauth_cancelled_message,
                false,
            ),
        );
    }

    let Some(code) = query.code else {
        fail_oauth_flow(
            state,
            pool,
            expected_state,
            &CommandError::new("backend.errors.oauthCallbackInvalid"),
        )
        .await;
        return http_response(
            400,
            "Bad Request",
            &render_oauth_page(
                &strings.oauth_invalid_title,
                &strings.oauth_missing_code,
                false,
            ),
        );
    };

    match complete_oauth_flow_inner(
        state,
        pool,
        CompleteOAuthInput {
            state: expected_state.to_string(),
            code,
        },
    )
    .await
    {
        Ok(status) => http_response(
            200,
            "OK",
            &render_oauth_page(
                &strings.oauth_connected_title,
                &strings.oauth_connected_message.replace(
                    "{provider}",
                    provider_label(&status.provider, &strings.oauth_provider_fallback),
                ),
                true,
            ),
        ),
        Err(err) => {
            fail_oauth_flow(state, pool, expected_state, &err).await;
            http_response(
                500,
                "Internal Server Error",
                &render_oauth_page(
                    &strings.oauth_failed_title,
                    &strings.oauth_failed_message,
                    false,
                ),
            )
        }
    }
}

async fn read_callback_query(
    stream: &mut tokio::net::TcpStream,
) -> Result<OAuthCallbackQuery, String> {
    let request = read_http_request(stream).await?;
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| "empty oauth callback request".to_string())?;

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    if method != "GET" {
        return Err(format!("unexpected oauth callback method: {method}"));
    }

    let path = parts
        .next()
        .ok_or_else(|| "missing oauth callback path".to_string())?;
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{path}"))
        .map_err(|err| format!("invalid oauth callback url: {err}"))?;

    if url.path() != "/oauth/callback" {
        return Err(format!("unexpected oauth callback path: {}", url.path()));
    }

    let mut query = OAuthCallbackQuery::default();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "state" => query.state = Some(value.into_owned()),
            "code" => query.code = Some(value.into_owned()),
            "error" => query.error = Some(value.into_owned()),
            "error_description" => query.error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    Ok(query)
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Result<String, String> {
    let mut buffer = [0_u8; 2048];
    let mut request = Vec::with_capacity(4096);

    loop {
        let read = timeout(std::time::Duration::from_secs(3), stream.read(&mut buffer))
            .await
            .map_err(|_| "oauth callback read timed out".to_string())?
            .map_err(|err| format!("failed to read oauth callback: {err}"))?;
        if read == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..read]);
        if request.len() > 16384 {
            return Err("oauth callback headers too large".to_string());
        }
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    String::from_utf8(request).map_err(|_| "oauth callback was not valid utf-8".to_string())
}

fn http_response(status: u16, reason: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn render_oauth_page(title: &str, message: &str, success: bool) -> String {
    let strings = crate::commands::ui_strings::current();
    let (tone, icon) = if success {
        (
            "success",
            "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"m5 12.5 4.25 4.25L19 7\"/></svg>",
        )
    } else {
        (
            "error",
            "<svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M12 7.5v5\"/><path d=\"M12 16.5h.01\"/><circle cx=\"12\" cy=\"12\" r=\"9\"/></svg>",
        )
    };
    format!(
        "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>:root{{color-scheme:dark;--bg:#1d1f23;--text:#e9eaec;--secondary:#a3a7ae;--tertiary:#6d7178;--success:#3fb950;--error:#f85149}}*{{box-sizing:border-box}}body{{margin:0;min-height:100vh;padding:32px;display:grid;place-items:center;background:var(--bg);color:var(--text);font-family:Inter,system-ui,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;-webkit-font-smoothing:antialiased}}main{{width:min(420px,100%);text-align:center}}.outcome{{width:40px;height:40px;margin:0 auto 20px}}.outcome svg{{display:block;width:100%;height:100%;fill:none;stroke:currentColor;stroke-width:1.75;stroke-linecap:round;stroke-linejoin:round}}.outcome.success{{color:var(--success)}}.outcome.error{{color:var(--error)}}h1{{margin:0;font-size:22px;line-height:1.25;font-weight:700;letter-spacing:-.02em}}p{{margin:0}}.message{{margin-top:10px;color:var(--secondary);font-size:14px;line-height:1.6}}.hint{{margin-top:24px;color:var(--tertiary);font-size:13px;line-height:1.5}}@media (prefers-color-scheme:light){{:root{{color-scheme:light;--bg:#f2f2f4;--text:#1c1e22;--secondary:#5b6069;--tertiary:#8f939b}}}}@media (max-width:480px){{body{{padding:24px}}}}</style></head><body><main><div class=\"outcome {tone}\">{icon}</div><h1>{}</h1><p class=\"message\">{}</p><p class=\"hint\">{}</p></main></body></html>",
        strings.lang,
        escape_html(title),
        escape_html(title),
        escape_html(message),
        escape_html(&strings.oauth_close_hint),
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn provider_label<'a>(provider: &'a str, fallback: &'a str) -> &'a str {
    match provider {
        "gmail" => "Gmail",
        "outlook" => "Outlook",
        _ => fallback,
    }
}

async fn mark_oauth_completed(state: &AppState, flow_state: &str, status: &OAuthStatus) {
    state.oauth_results.lock().await.insert(
        flow_state.to_string(),
        OAuthFlowResult::Completed {
            account_id: status.account_id.clone(),
            provider: status.provider.clone(),
            connected: status.connected,
            has_refresh_token: status.has_refresh_token,
            expires_at: status.expires_at.clone(),
        },
    );
}

/// Record a terminal failure where it survives a restart, then in memory.
///
/// The database is written first: a poll after a process death has nowhere
/// else to look, and the in-memory map is only the fast path in front of it.
async fn fail_oauth_flow(
    state: &AppState,
    pool: &SqlitePool,
    flow_state: &str,
    error: &CommandError,
) {
    if let Err(persistence) = OAuthFlowRepository::new(pool)
        .fail(flow_state, &error.encoded())
        .await
    {
        tracing::error!(error = ?persistence, "failed OAuth flow could not be recorded");
    }
    mark_oauth_failed(state, flow_state, error.encoded()).await;
}

async fn mark_oauth_failed(state: &AppState, flow_state: &str, message: String) {
    let mut results = state.oauth_results.lock().await;
    let should_replace = !matches!(
        results.get(flow_state),
        Some(OAuthFlowResult::Completed { .. })
    );
    if should_replace {
        results.insert(flow_state.to_string(), OAuthFlowResult::Failed(message));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn connected_oauth_account_restarts_its_poller() {
        let state = crate::AppState::default();
        let (idle, mut commands) = crate::sync::idle::IdleControl::test_channel();
        assert!(state.idle_control.set(idle).is_ok());
        super::restart_account_poller(&state, "new-oauth-account");
        assert!(matches!(commands.try_recv().unwrap(),
            crate::sync::idle::IdleCommand::AddAccount(id) if id == "new-oauth-account"));
    }

    #[tokio::test]
    async fn idle_and_wrong_state_connections_do_not_consume_the_callback() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            super::wait_for_callback(
                &listener,
                "expected",
                tokio::time::Instant::now() + std::time::Duration::from_secs(2),
                std::time::Duration::from_millis(50),
            )
            .await
        });
        let _idle = tokio::net::TcpStream::connect(address).await.unwrap();
        let mut wrong = tokio::net::TcpStream::connect(address).await.unwrap();
        wrong
            .write_all(b"GET /oauth/callback?state=wrong&code=evil HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut valid = tokio::net::TcpStream::connect(address).await.unwrap();
        valid
            .write_all(b"GET /oauth/callback?state=expected&code=good HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let (_, query) = task.await.unwrap().unwrap();
        assert_eq!(query.code.as_deref(), Some("good"));
    }

    #[tokio::test]
    async fn a_ready_connection_cannot_extend_an_expired_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _queued = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let result = super::wait_for_callback(
            &listener,
            "expected",
            tokio::time::Instant::now(),
            std::time::Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            result.err().unwrap().key,
            "backend.errors.oauthCallbackTimedOut"
        );
    }

    #[tokio::test]
    async fn callback_deadline_bounds_an_idle_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _idle = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let result = super::wait_for_callback(
            &listener,
            "expected",
            tokio::time::Instant::now() + std::time::Duration::from_millis(50),
            std::time::Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            result.err().unwrap().key,
            "backend.errors.oauthCallbackTimedOut"
        );
    }

    #[cfg(desktop)]
    use super::render_oauth_page;
    use super::{oauth_flow_command_error, resolve_persisted_oauth_flow, CommandError};
    use chrono::{DateTime, Duration, Utc};
    use satchel_core::db::oauth::OAuthFlowError;
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

    #[tokio::test]
    async fn mobile_configured_loopback_callback_accepts_the_browser_request() {
        use super::{bind_callback_listener, oauth, read_callback_query};
        use tokio::io::AsyncWriteExt;

        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let uri = format!("http://127.0.0.1:{port}/oauth/callback");
        let listener = bind_callback_listener(oauth::OAuthPlatform::Mobile, Some(&uri))
            .await
            .unwrap()
            .expect("the configured mobile callback must have a listener");
        let request = tokio::spawn(async move {
            let mut browser = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .expect("the browser callback must not be refused");
            browser.write_all(b"GET /oauth/callback?state=flow&code=fixture-code HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let query = read_callback_query(&mut stream).await.unwrap();
        assert_eq!(query.state.as_deref(), Some("flow"));
        assert_eq!(query.code.as_deref(), Some("fixture-code"));
        request.await.unwrap();
    }

    #[tokio::test]
    async fn mobile_deep_link_callback_does_not_bind_a_socket() {
        use super::{bind_callback_listener, oauth};

        for uri in [
            None,
            Some(oauth::MOBILE_REDIRECT_URI),
            Some("https://satchel.test/cb"),
        ] {
            assert!(bind_callback_listener(oauth::OAuthPlatform::Mobile, uri)
                .await
                .unwrap()
                .is_none());
        }
        let listener = bind_callback_listener(oauth::OAuthPlatform::Desktop, None)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(listener.local_addr().unwrap().port(), 0);
    }

    /// A database with the flow tables and one account, and no keychain in
    /// sight: `status` never decrypts, and account configs pass plaintext
    /// through, so these tests read exactly what a restarted app would.
    async fn pool_with_account() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("../crates/satchel-core/src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state, color,
              is_active, created_at, updated_at)
             VALUES ('account', 'Account', 'user@example.com', 'gmail', '{}', '', '{}',
                     '#000000', 1, ?, ?)",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .expect("account");
        pool
    }

    async fn insert_session(
        pool: &SqlitePool,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO oauth_flow_sessions
             (state, account_id, provider, encrypted_code_verifier, created_at, expires_at,
              redirect_uri)
             VALUES ('flow', 'account', 'gmail', 'enc:v1:x', ?, ?, 'http://127.0.0.1:1/x')",
        )
        .bind(created_at.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(pool)
        .await
        .expect("session");
    }

    async fn insert_outcome(
        pool: &SqlitePool,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        error_key: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO oauth_flow_outcomes
             (state, account_id, provider, created_at, expires_at, error_key)
             VALUES ('flow', 'account', 'gmail', ?, ?, ?)",
        )
        .bind(created_at.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .bind(error_key)
        .execute(pool)
        .await
        .expect("outcome");
    }

    async fn connect_account(pool: &SqlitePool, connected_at: DateTime<Utc>) {
        let config = serde_json::json!({
            "oauth": {
                "provider": "gmail",
                "refresh_token": "refresh",
                "expires_at": (connected_at + Duration::hours(1)).to_rfc3339(),
                "connected_at": connected_at.to_rfc3339(),
            }
        })
        .to_string();
        satchel_core::utils::crypto::seed_master_key_for_tests();
        let config = crate::commands::accounts::encrypt_account_config("account", &config).unwrap();
        sqlx::query("UPDATE accounts SET config = ? WHERE id = 'account'")
            .bind(config)
            .execute(pool)
            .await
            .expect("connect");
    }

    #[tokio::test]
    async fn a_flow_this_process_never_saw_is_still_pending_until_it_expires() {
        let pool = pool_with_account().await;
        let now = Utc::now();
        insert_session(&pool, now, now + Duration::minutes(15)).await;

        let response = resolve_persisted_oauth_flow(&pool, "flow", now)
            .await
            .expect("poll");
        assert_eq!(response.status, "pending");
    }

    #[tokio::test]
    async fn an_expired_or_unknown_flow_stays_a_not_found_error() {
        let pool = pool_with_account().await;
        let now = Utc::now();
        insert_session(
            &pool,
            now - Duration::minutes(30),
            now - Duration::minutes(15),
        )
        .await;

        let expired = resolve_persisted_oauth_flow(&pool, "flow", now)
            .await
            .expect_err("expired");
        assert_eq!(expired.key, "backend.errors.oauthFlowNotFound");
        let unknown = resolve_persisted_oauth_flow(&pool, "never-existed", now)
            .await
            .expect_err("unknown");
        assert_eq!(unknown.key, "backend.errors.oauthFlowNotFound");
    }

    #[tokio::test]
    async fn a_recorded_failure_reaches_a_poll_from_a_later_process() {
        let pool = pool_with_account().await;
        let now = Utc::now();
        let cancelled = CommandError::new("backend.errors.oauthAuthorizationCancelled");
        insert_outcome(
            &pool,
            now,
            now + Duration::minutes(15),
            Some(&cancelled.encoded()),
        )
        .await;

        let response = resolve_persisted_oauth_flow(&pool, "flow", now)
            .await
            .expect("poll");
        assert_eq!(response.status, "failed");
        assert_eq!(
            response.error.as_deref(),
            Some(cancelled.encoded().as_str())
        );
    }

    #[tokio::test]
    async fn a_spent_verifier_reports_completion_from_the_account_it_connected() {
        let pool = pool_with_account().await;
        let now = Utc::now();
        insert_outcome(&pool, now, now + Duration::minutes(15), None).await;

        // Exchange under way: the account has nothing yet.
        let pending = resolve_persisted_oauth_flow(&pool, "flow", now)
            .await
            .expect("poll");
        assert_eq!(pending.status, "pending");

        connect_account(&pool, now + Duration::seconds(2)).await;
        let completed = resolve_persisted_oauth_flow(&pool, "flow", now)
            .await
            .expect("poll");
        assert_eq!(completed.status, "completed");
        assert_eq!(
            completed.result.map(|status| status.account_id),
            Some("account".to_string())
        );
    }

    #[tokio::test]
    async fn credentials_older_than_the_flow_do_not_pass_as_its_result() {
        let pool = pool_with_account().await;
        let now = Utc::now();
        // A re-authorization: the account was already connected when it began.
        connect_account(&pool, now - Duration::days(3)).await;
        insert_outcome(&pool, now, now + Duration::minutes(15), None).await;

        let pending = resolve_persisted_oauth_flow(&pool, "flow", now)
            .await
            .expect("poll");
        assert_eq!(pending.status, "pending");

        // Once the deadline passes without new credentials it is over.
        let over = resolve_persisted_oauth_flow(&pool, "flow", now + Duration::minutes(16))
            .await
            .expect("poll");
        assert_eq!(over.status, "failed");
    }

    /// The redirect URI the flow announces, decided before anything is bound.
    ///
    /// Only the desktop arm can be observed here — `cargo test` never runs on
    /// an Android or iOS target — so this pins the constant this build picked;
    /// the mobile arm is covered in `satchel_core::utils::oauth`.
    #[cfg(desktop)]
    #[test]
    fn a_desktop_build_redirects_to_its_own_loopback_port_unless_one_is_configured() {
        use satchel_core::utils::oauth::{redirect_uri_for, OAuthPlatform};

        assert_eq!(super::CALLBACK_PLATFORM, OAuthPlatform::Desktop);
        assert_eq!(
            redirect_uri_for(super::CALLBACK_PLATFORM, None, 49152),
            "http://127.0.0.1:49152/oauth/callback"
        );
        assert_eq!(
            redirect_uri_for(
                super::CALLBACK_PLATFORM,
                Some("http://127.0.0.1:8765/oauth/callback"),
                49152
            ),
            "http://127.0.0.1:8765/oauth/callback"
        );
    }

    #[test]
    fn oauth_repository_errors_use_redacted_stable_keys() {
        assert_eq!(
            oauth_flow_command_error(OAuthFlowError::NotFoundOrExpired).key,
            "backend.errors.oauthFlowNotFound"
        );
        let secret = "secret verifier detail";
        let error = oauth_flow_command_error(OAuthFlowError::CredentialAccess {
            detail: secret.into(),
        });
        let serialized = serde_json::to_string(&error).unwrap();
        assert_eq!(error.key, "backend.errors.oauthCredentialAccessFailed");
        assert!(!serialized.contains(secret));
    }

    #[cfg(desktop)]
    #[test]
    fn oauth_page_uses_the_app_palette_without_legacy_card_chrome() {
        let strings = crate::commands::ui_strings::current();
        let html = render_oauth_page(
            "Satchel connected",
            "Gmail is now connected to Satchel.",
            true,
        );

        assert!(html.contains("--bg:#1d1f23"));
        assert!(html.contains("class=\"outcome success\""));
        assert!(html.contains(&format!("<html lang=\"{}\">", strings.lang)));
        assert!(html.contains(&strings.oauth_close_hint));
        assert!(!html.contains("zur App zurückkehren"));
        assert!(!html.contains("class=\"badge\""));
        assert!(!html.contains("box-shadow"));
        assert!(!html.contains("<main><div class=\"badge\""));
    }

    #[cfg(desktop)]
    #[test]
    fn oauth_page_omits_technical_error_details() {
        let html = render_oauth_page(
            "OAuth-Rückruf ungültig",
            "Die Rückleitung ist fehlgeschlagen.",
            false,
        );

        assert!(html.contains("class=\"outcome error\""));
        assert!(!html.contains("class=\"detail\""));
    }
}
