use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy)]
pub enum OAuthProvider {
    Gmail,
    Outlook,
}

#[derive(Debug, Clone)]
pub struct OAuthClientConfig {
    pub provider: OAuthProvider,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenPayload {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OAuthIdentity {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfoResponse {
    email: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphMeResponse {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    mail: Option<String>,
    #[serde(rename = "userPrincipalName")]
    user_principal_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StoredOAuthConfig {
    pub provider: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub redirect_uri: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub expires_at: Option<String>,
    /// When these credentials were last established. Not the token expiry:
    /// a poll uses it to tell a finished flow from one still under way.
    pub connected_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AccountOAuthSnapshot {
    oauth: Option<StoredOAuthConfig>,
}

impl OAuthProvider {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "gmail" => Ok(Self::Gmail),
            "outlook" => Ok(Self::Outlook),
            _ => Err(format!("unsupported oauth provider: {value}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gmail => "gmail",
            Self::Outlook => "outlook",
        }
    }

    /// A redirect URI someone set explicitly, which outranks anything derived.
    pub fn configured_redirect_uri(self) -> Option<String> {
        match self {
            Self::Gmail => optional_env("SATCHEL_GMAIL_REDIRECT_URI"),
            Self::Outlook => optional_env("SATCHEL_OUTLOOK_REDIRECT_URI"),
        }
    }

    pub fn load_config(self) -> Result<OAuthClientConfig, String> {
        match self {
            Self::Gmail => Ok(self.build_config(
                required_env("SATCHEL_GMAIL_CLIENT_ID")?,
                optional_env("SATCHEL_GMAIL_CLIENT_SECRET"),
                optional_env("SATCHEL_GMAIL_REDIRECT_URI").unwrap_or_else(default_redirect_uri),
            )),
            Self::Outlook => Ok(self.build_config(
                required_env("SATCHEL_OUTLOOK_CLIENT_ID")?,
                optional_env("SATCHEL_OUTLOOK_CLIENT_SECRET"),
                optional_env("SATCHEL_OUTLOOK_REDIRECT_URI").unwrap_or_else(default_redirect_uri),
            )),
        }
    }

    fn load_config_for_stored(
        self,
        stored: &StoredOAuthConfig,
    ) -> Result<OAuthClientConfig, String> {
        let client_id = stored
            .client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| match self {
                Self::Gmail => optional_env("SATCHEL_GMAIL_CLIENT_ID"),
                Self::Outlook => optional_env("SATCHEL_OUTLOOK_CLIENT_ID"),
            })
            .ok_or_else(|| match self {
                Self::Gmail => "missing SATCHEL_GMAIL_CLIENT_ID".to_string(),
                Self::Outlook => "missing SATCHEL_OUTLOOK_CLIENT_ID".to_string(),
            })?;

        let client_secret = stored
            .client_secret
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| match self {
                Self::Gmail => optional_env("SATCHEL_GMAIL_CLIENT_SECRET"),
                Self::Outlook => optional_env("SATCHEL_OUTLOOK_CLIENT_SECRET"),
            });

        let redirect_uri = stored
            .redirect_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| match self {
                Self::Gmail => optional_env("SATCHEL_GMAIL_REDIRECT_URI"),
                Self::Outlook => optional_env("SATCHEL_OUTLOOK_REDIRECT_URI"),
            })
            .unwrap_or_else(default_redirect_uri);

        Ok(self.build_config(client_id, client_secret, redirect_uri))
    }

    fn build_config(
        self,
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
    ) -> OAuthClientConfig {
        match self {
            Self::Gmail => OAuthClientConfig {
                provider: self,
                client_id,
                client_secret,
                redirect_uri,
                auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token_url: "https://oauth2.googleapis.com/token".into(),
                scopes: vec![
                    "openid",
                    "email",
                    "profile",
                    "https://www.googleapis.com/auth/contacts.readonly",
                    "https://www.googleapis.com/auth/contacts.other.readonly",
                    "https://www.googleapis.com/auth/gmail.modify",
                    "https://www.googleapis.com/auth/gmail.send",
                    // Read-only calendar sync; accounts connected before this
                    // scope existed must re-run the OAuth flow to grant it.
                    "https://www.googleapis.com/auth/calendar.readonly",
                ],
            },
            Self::Outlook => OAuthClientConfig {
                provider: self,
                client_id,
                client_secret,
                redirect_uri,
                auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
                token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
                scopes: vec![
                    "openid",
                    "email",
                    "profile",
                    "offline_access",
                    "https://graph.microsoft.com/Contacts.Read",
                    "https://graph.microsoft.com/Mail.ReadWrite",
                    "https://graph.microsoft.com/Mail.Send",
                ],
            },
        }
    }
}

fn required_env(key: &str) -> Result<String, String> {
    optional_env(key).ok_or_else(|| format!("missing {key}"))
}

fn optional_env(key: &str) -> Option<String> {
    crate::utils::environment::debug_env(key)
        .or_else(|| compiled_env(key).map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Public client configuration compiled by `build.rs`; release ignores runtime overrides.
///
/// `option_env!` takes a literal, so the keys are spelled out rather than
/// looked up. They match `BAKED_KEYS` in the build script.
fn compiled_env(key: &str) -> Option<&'static str> {
    match key {
        "SATCHEL_GMAIL_CLIENT_ID" => option_env!("SATCHEL_GMAIL_CLIENT_ID"),
        "SATCHEL_GMAIL_REDIRECT_URI" => option_env!("SATCHEL_GMAIL_REDIRECT_URI"),
        "SATCHEL_OUTLOOK_CLIENT_ID" => option_env!("SATCHEL_OUTLOOK_CLIENT_ID"),
        "SATCHEL_OUTLOOK_REDIRECT_URI" => option_env!("SATCHEL_OUTLOOK_REDIRECT_URI"),
        _ => None,
    }
}

fn default_redirect_uri() -> String {
    loopback_redirect_uri(DEFAULT_LOOPBACK_PORT)
}

/// Port used when nothing pins one, and by the paths where the redirect URI is
/// carried but never sent — the refresh grant does not include it.
pub const DEFAULT_LOOPBACK_PORT: u16 = 8765;

/// The loopback address the callback listener answers on.
///
/// RFC 8252 asks native clients to take an ephemeral port from the operating
/// system for each authorization request, and requires authorization servers to
/// accept any port for a loopback redirect. A fixed port is a door another app
/// on the machine can sit on first — PKCE keeps it from spending the code it
/// would catch, but the standard's answer is simply not to offer the door.
pub fn loopback_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/oauth/callback")
}

/// The custom scheme a phone gets its authorization code back on.
///
/// Mirrors `CALLBACK_SCHEME`/`CALLBACK_HOST`/`CALLBACK_PATH` in
/// `src/utils/mobileOAuth.ts` and the `plugins.deep-link` registration in
/// `tauri.conf.json`; all three have to name the same URI.
pub const MOBILE_REDIRECT_URI: &str = "de.tbuck.satchel://oauth/callback";

/// How a build receives the authorization code the provider sends back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthPlatform {
    /// Serves the callback itself, on a loopback socket it owns.
    Desktop,
    /// Has the operating system hand the callback over as a deep link.
    Mobile,
}

/// The address the provider is told to redirect to.
///
/// Mobile defaults to the registered custom scheme, which the system delivers
/// even after a process restart. An explicit loopback registration instead
/// needs the callback listener to remain alive while the browser is open.
///
/// An explicitly configured URI wins over both: whoever set it registered
/// exactly that address with their provider, and silently sending a different
/// one earns a `redirect_uri_mismatch`.
pub fn redirect_uri_for(platform: OAuthPlatform, configured: Option<&str>, port: u16) -> String {
    if let Some(configured) = configured.map(str::trim).filter(|uri| !uri.is_empty()) {
        return configured.to_string();
    }
    match platform {
        OAuthPlatform::Desktop => loopback_redirect_uri(port),
        OAuthPlatform::Mobile => MOBILE_REDIRECT_URI.to_string(),
    }
}

/// The port a pinned redirect URI insists on, if it pins a loopback one.
///
/// Someone who has registered a fixed port with their provider must keep
/// getting it; only an unconfigured flow is free to take what the OS offers.
pub fn pinned_loopback_port(uri: &str) -> Option<u16> {
    let url = reqwest::Url::parse(uri).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    match url.host_str()? {
        "127.0.0.1" | "localhost" | "[::1]" => url.port(),
        _ => None,
    }
}

pub fn generate_state() -> String {
    format!("st-{}", uuid::Uuid::new_v4())
}

pub fn generate_code_verifier() -> String {
    format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()).replace('-', "")
}

pub fn code_challenge_s256(code_verifier: &str) -> String {
    let digest = Sha256::digest(code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub fn build_authorization_url(
    config: &OAuthClientConfig,
    state: &str,
    code_challenge: &str,
) -> Result<String, String> {
    let mut url = Url::parse(&config.auth_url).map_err(|err| err.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("client_id", &config.client_id);
        query.append_pair("redirect_uri", &config.redirect_uri);
        query.append_pair("response_type", "code");
        query.append_pair("scope", &config.scopes.join(" "));
        query.append_pair("state", state);
        query.append_pair("code_challenge", code_challenge);
        query.append_pair("code_challenge_method", "S256");

        match config.provider {
            OAuthProvider::Gmail => {
                query.append_pair("access_type", "offline");
                query.append_pair("prompt", "consent");
            }
            OAuthProvider::Outlook => {
                query.append_pair("response_mode", "query");
            }
        }
    }
    Ok(url.to_string())
}

pub async fn exchange_code_for_token(
    config: &OAuthClientConfig,
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokenPayload, String> {
    let client = crate::utils::http::client();

    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", config.redirect_uri.clone()),
        ("client_id", config.client_id.clone()),
        ("code_verifier", code_verifier.to_string()),
    ];

    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret.clone()));
    }

    let response = client
        .post(&config.token_url)
        .form(&form)
        .send()
        .await
        .map_err(|err| format!("oauth token request failed: {err}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "oauth token exchange failed with status {}: {}",
            status, body
        ));
    }

    let token: OAuthTokenResponse = response
        .json()
        .await
        .map_err(|err| format!("oauth token parse failed: {err}"))?;

    let expires_at = token
        .expires_in
        .map(|seconds| (chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339());

    Ok(OAuthTokenPayload {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        token_type: token.token_type,
        scope: token.scope,
        expires_at,
    })
}

pub async fn fetch_identity(
    provider: OAuthProvider,
    access_token: &str,
) -> Result<OAuthIdentity, String> {
    let client = crate::utils::http::client();

    match provider {
        OAuthProvider::Gmail => {
            let response = client
                .get("https://openidconnect.googleapis.com/v1/userinfo")
                .bearer_auth(access_token)
                .send()
                .await
                .map_err(|err| format!("gmail userinfo request failed: {err}"))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(format!(
                    "gmail userinfo request failed with status {status}: {body}"
                ));
            }

            let payload: GoogleUserInfoResponse = response
                .json()
                .await
                .map_err(|err| format!("gmail userinfo parse failed: {err}"))?;

            let email = payload
                .email
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "gmail userinfo missing email".to_string())?;

            Ok(OAuthIdentity {
                email,
                display_name: payload.name.filter(|value| !value.trim().is_empty()),
            })
        }
        OAuthProvider::Outlook => {
            let response = client
                .get("https://graph.microsoft.com/v1.0/me?$select=displayName,mail,userPrincipalName")
                .bearer_auth(access_token)
                .send()
                .await
                .map_err(|err| format!("outlook profile request failed: {err}"))?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(format!(
                    "outlook profile request failed with status {status}: {body}"
                ));
            }

            let payload: GraphMeResponse = response
                .json()
                .await
                .map_err(|err| format!("outlook profile parse failed: {err}"))?;

            let email = payload
                .mail
                .or(payload.user_principal_name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "outlook profile missing email".to_string())?;

            Ok(OAuthIdentity {
                email,
                display_name: payload
                    .display_name
                    .filter(|value| !value.trim().is_empty()),
            })
        }
    }
}

pub fn load_stored_oauth(config_json: &str) -> Result<StoredOAuthConfig, String> {
    let snapshot = serde_json::from_str::<AccountOAuthSnapshot>(config_json)
        .map_err(|err| format!("invalid oauth config JSON: {err}"))?;
    snapshot
        .oauth
        .ok_or_else(|| "missing oauth config for account".to_string())
}

pub async fn access_token_from_account_config(
    provider: OAuthProvider,
    config_json: &crate::accounts::AccountConfig,
) -> Result<String, String> {
    let stored = load_stored_oauth(config_json)?;
    let access_token = stored
        .access_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if let Some(token) = access_token {
        let not_expired = oauth_token_is_fresh(&stored);

        if not_expired {
            return Ok(token);
        }
    }

    let config = provider.load_config_for_stored(&stored)?;
    let refresh_token = stored
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing oauth refresh token".to_string())?;

    let refreshed = refresh_access_token(&config, refresh_token).await?;
    Ok(refreshed.access_token)
}

pub async fn ensure_fresh_account_config(
    provider: OAuthProvider,
    config_json: &crate::accounts::AccountConfig,
) -> Result<Option<String>, String> {
    // Returns *plaintext* JSON when it refreshes; callers re-encrypt before
    // storing (see the encrypt_account_config calls at each UPDATE).
    let stored = load_stored_oauth(config_json)?;
    if oauth_token_is_fresh(&stored) {
        return Ok(None);
    }

    let refresh_token = stored
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing oauth refresh token".to_string())?;
    let config = provider.load_config_for_stored(&stored)?;
    let refreshed = refresh_access_token(&config, refresh_token).await?;
    Ok(Some(merge_token_into_config(
        config_json,
        provider,
        &refreshed,
    )?))
}

pub fn merge_token_into_config(
    config_json: &str,
    provider: OAuthProvider,
    token: &OAuthTokenPayload,
) -> Result<String, String> {
    let mut config = serde_json::from_str::<Value>(config_json)
        .unwrap_or_else(|_| Value::Object(Default::default()));
    if !config.is_object() {
        config = Value::Object(Default::default());
    }

    let stored = load_stored_oauth(config_json).ok();
    let provider_config = match stored.as_ref() {
        Some(stored) => provider.load_config_for_stored(stored)?,
        None => provider.load_config()?,
    };
    let refresh_token = token
        .refresh_token
        .clone()
        .or_else(|| stored.and_then(|oauth| oauth.refresh_token));
    // No `client_secret`: it would ride along into every backup and platform
    // export. The provider config still carries one where a deployment set it.
    config["oauth"] = serde_json::json!({
        "provider": provider.as_str(),
        "client_id": provider_config.client_id,
        "redirect_uri": provider_config.redirect_uri,
        "access_token": token.access_token,
        "refresh_token": refresh_token,
        "token_type": token.token_type,
        "scope": token.scope,
        "expires_at": token.expires_at,
        "connected_at": chrono::Utc::now().to_rfc3339()
    });

    Ok(config.to_string())
}

async fn refresh_access_token(
    config: &OAuthClientConfig,
    refresh_token: &str,
) -> Result<OAuthTokenPayload, String> {
    let client = crate::utils::http::client();
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("client_id", config.client_id.clone()),
    ];

    if let Some(secret) = &config.client_secret {
        form.push(("client_secret", secret.clone()));
    }

    let response = client
        .post(&config.token_url)
        .form(&form)
        .send()
        .await
        .map_err(|err| format!("oauth refresh request failed: {err}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "oauth refresh failed with status {}: {}",
            status, body
        ));
    }

    let token: OAuthTokenResponse = response
        .json()
        .await
        .map_err(|err| format!("oauth refresh parse failed: {err}"))?;

    let expires_at = token
        .expires_in
        .map(|seconds| (chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339());

    Ok(OAuthTokenPayload {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .or_else(|| Some(refresh_token.to_string())),
        token_type: token.token_type,
        scope: token.scope,
        expires_at,
    })
}

fn parse_rfc3339(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

pub fn oauth_config_has_fresh_access_token(config_json: &str) -> bool {
    load_stored_oauth(config_json)
        .map(|stored| oauth_token_is_fresh(&stored))
        .unwrap_or(false)
}

fn oauth_token_is_fresh(stored: &StoredOAuthConfig) -> bool {
    stored
        .access_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        && stored
            .expires_at
            .as_deref()
            .and_then(parse_rfc3339)
            .map(|expires_at| expires_at > chrono::Utc::now() + chrono::Duration::seconds(90))
            .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_oauth_config_supplies_client_details_without_process_env() {
        let stored = StoredOAuthConfig {
            provider: Some("gmail".to_string()),
            client_id: Some("stored-client-id".to_string()),
            client_secret: Some("stored-secret".to_string()),
            redirect_uri: Some("http://127.0.0.1:8765/oauth/callback".to_string()),
            access_token: Some("access".to_string()),
            refresh_token: Some("refresh".to_string()),
            token_type: None,
            scope: None,
            expires_at: Some("2000-01-01T00:00:00Z".to_string()),
            connected_at: None,
        };

        let config = OAuthProvider::Gmail
            .load_config_for_stored(&stored)
            .expect("stored oauth config should be enough for refresh");

        assert_eq!(config.client_id, "stored-client-id");
        assert_eq!(config.client_secret.as_deref(), Some("stored-secret"));
        assert_eq!(config.redirect_uri, "http://127.0.0.1:8765/oauth/callback");
        assert_eq!(config.token_url, "https://oauth2.googleapis.com/token");
    }

    /// No client secret is compiled into the binary.
    ///
    /// `strings` on a release build used to find the `GOCSPX-` prefix, and the
    /// same secret was copied into every account row from there. The
    /// runtime configuration supplies any secret required by the registration.
    #[test]
    fn no_client_secret_is_baked_into_the_binary() {
        let build_rs = include_str!("../../build.rs");
        let baked_keys = build_rs
            .split_once("const BAKED_KEYS")
            .expect("build.rs declares BAKED_KEYS")
            .1
            .split_once("];")
            .expect("the list is terminated")
            .0;

        for key in [
            "SATCHEL_GMAIL_CLIENT_SECRET",
            "SATCHEL_OUTLOOK_CLIENT_SECRET",
        ] {
            assert!(
                !baked_keys.contains(&format!("\"{key}\"")),
                "{key} must not be compiled into the binary"
            );
            assert!(
                compiled_env(key).is_none(),
                "{key} still resolves from the compiled environment"
            );
        }
    }

    /// The secret must not ride along into backups and platform exports.
    #[test]
    fn merge_token_into_config_omits_the_client_secret() {
        let stored = serde_json::json!({
            "oauth": {
                "provider": "gmail",
                "client_id": "stored-client-id",
                "client_secret": "stored-secret",
                "redirect_uri": "http://127.0.0.1:8765/oauth/callback",
                "refresh_token": "refresh",
            }
        })
        .to_string();
        let token = OAuthTokenPayload {
            access_token: "fresh-access".into(),
            refresh_token: None,
            token_type: Some("Bearer".into()),
            scope: None,
            expires_at: None,
        };

        let merged = merge_token_into_config(&stored, OAuthProvider::Gmail, &token)
            .expect("merging a refreshed token");
        let value: Value = serde_json::from_str(&merged).expect("merged config is json");

        assert_eq!(value["oauth"]["client_id"], "stored-client-id");
        assert_eq!(value["oauth"]["access_token"], "fresh-access");
        assert!(
            value["oauth"].get("client_secret").is_none(),
            "the account row still carries a client secret: {merged}"
        );
    }

    /// The build script bakes a list of keys; `compiled_env` maps them back.
    ///
    /// Two lists that must agree, in two files, with no compiler check between
    /// them: adding a key to the build script and forgetting the match arm
    /// leaves the value compiled into the binary and unreachable — which on a
    /// phone looks exactly like the "missing SATCHEL_GMAIL_CLIENT_ID" this
    /// whole mechanism exists to prevent.
    #[test]
    fn every_baked_key_can_be_read_back() {
        let build_rs = include_str!("../../build.rs");
        let list = build_rs
            .split_once("const BAKED_KEYS")
            .expect("build.rs declares BAKED_KEYS")
            .1
            .split_once("];")
            .expect("the list is terminated")
            .0;

        let keys: Vec<&str> = list
            .split('"')
            .filter(|part| part.starts_with("SATCHEL_"))
            .collect();
        assert!(!keys.is_empty(), "expected the build script to bake keys");

        // Matches on the source, not on the runtime answer: a machine without a
        // `.env` bakes nothing, and the arm still has to be there.
        for key in keys {
            assert!(
                include_str!("oauth.rs").contains(&format!("\"{key}\" => option_env!")),
                "{key} is baked but compiled_env has no arm for it"
            );
        }
    }

    #[test]
    fn mobile_build_carries_explicit_oauth_redirect_uris() {
        let build_rs = include_str!("../../build.rs");
        let baked_keys = build_rs
            .split_once("const BAKED_KEYS")
            .expect("build.rs declares BAKED_KEYS")
            .1
            .split_once("];")
            .expect("the list is terminated")
            .0;

        for key in ["SATCHEL_GMAIL_REDIRECT_URI", "SATCHEL_OUTLOOK_REDIRECT_URI"] {
            assert!(
                baked_keys.contains(&format!("\"{key}\"")),
                "{key} must be available to mobile OAuth flows"
            );
        }
    }

    /// A pinned port must survive, because the person who pinned it registered
    /// exactly that address with their provider.
    #[test]
    fn a_pinned_loopback_port_is_recognised() {
        assert_eq!(
            super::pinned_loopback_port("http://127.0.0.1:8765/oauth/callback"),
            Some(8765)
        );
        assert_eq!(
            super::pinned_loopback_port("http://localhost:1234/oauth/callback"),
            Some(1234)
        );
    }

    /// Anything that is not a loopback address pins nothing, and the flow is
    /// then free to take whatever port the operating system offers.
    #[test]
    fn only_loopback_addresses_pin_a_port() {
        for uri in [
            "https://example.com:8765/oauth/callback",
            "de.tbuck.satchel://oauth/callback",
            "http://192.168.1.5:8765/oauth/callback",
            "not a url",
        ] {
            assert_eq!(super::pinned_loopback_port(uri), None, "{uri}");
        }
        // No port at all means nothing to pin.
        assert_eq!(
            super::pinned_loopback_port("http://127.0.0.1/oauth/callback"),
            None
        );
    }

    /// The three-way choice the flow makes before it builds the authorization
    /// URL. Both platform arms are checked from one test binary, because the
    /// desktop test run is the only run there is.
    #[test]
    fn the_redirect_uri_follows_the_platform_unless_one_is_configured() {
        assert_eq!(
            super::redirect_uri_for(OAuthPlatform::Desktop, None, 49152),
            "http://127.0.0.1:49152/oauth/callback"
        );
        assert_eq!(
            super::redirect_uri_for(OAuthPlatform::Mobile, None, 49152),
            "de.tbuck.satchel://oauth/callback"
        );
        for platform in [OAuthPlatform::Desktop, OAuthPlatform::Mobile] {
            assert_eq!(
                super::redirect_uri_for(platform, Some(" https://satchel.test/cb "), 49152),
                "https://satchel.test/cb",
                "{platform:?}"
            );
            // An empty environment variable configures nothing.
            assert_eq!(
                super::redirect_uri_for(platform, Some("  "), 49152),
                super::redirect_uri_for(platform, None, 49152),
                "{platform:?}"
            );
        }
    }

    /// The path has to stay put: the callback listener matches on it, and the
    /// provider compares the whole string when the code is exchanged.
    #[test]
    fn the_loopback_uri_keeps_its_callback_path() {
        assert_eq!(
            super::loopback_redirect_uri(49152),
            "http://127.0.0.1:49152/oauth/callback"
        );
    }
}
