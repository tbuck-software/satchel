use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::collections::HashSet;
use tokio::time::{timeout, Duration};

use crate::commands::error::{CommandError, CommandResult};
use crate::providers::{
    dav::{self, DavConfig, GroupwareSyncSummary},
    imap::ImapProvider,
    traits::MailProvider,
};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(12);
/// Short on purpose: the answer only preselects a sign-in method while typing.
const MX_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/onboarding/", optional_fields = nullable))]
pub struct DiscoverAccountInput {
    pub email: String,
    pub password: String,
    /// "imap" (default) or "pop3".
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub imap: Option<ServerOverride>,
    #[serde(default)]
    pub pop3: Option<ServerOverride>,
    #[serde(default)]
    pub smtp: Option<ServerOverride>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/onboarding/", optional_fields = nullable))]
pub struct ServerOverride {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/onboarding/")
)]
pub struct DiscoveredService {
    pub kind: String,
    pub status: String,
    pub label: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub collections: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/onboarding/")
)]
pub struct AccountDiscoveryDto {
    pub ready: bool,
    pub username: String,
    pub config: serde_json::Value,
    pub services: Vec<DiscoveredService>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/onboarding/")
)]
pub struct DavAutoconfigureDto {
    pub caldav_server: Option<String>,
    pub carddav_server: Option<String>,
    pub summary: GroupwareSyncSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MailCandidate {
    host: String,
    port: u16,
}

#[derive(Debug, Clone, Default)]
struct Autoconfig {
    imap: Vec<MailCandidate>,
    smtp: Vec<MailCandidate>,
    prefer_initial_last_username: bool,
}

#[derive(Debug, Clone, Copy)]
struct ProviderPreset {
    name: &'static str,
    domains: &'static [&'static str],
    imap_host: &'static str,
    imap_port: u16,
    smtp_host: &'static str,
    smtp_port: u16,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "WEB.DE",
        domains: &["web.de"],
        imap_host: "imap.web.de",
        imap_port: 993,
        smtp_host: "smtp.web.de",
        smtp_port: 587,
    },
    ProviderPreset {
        name: "GMX",
        domains: &["gmx.de", "gmx.net", "gmx.com", "gmx.at", "gmx.ch"],
        imap_host: "imap.gmx.net",
        imap_port: 993,
        smtp_host: "mail.gmx.net",
        smtp_port: 587,
    },
    ProviderPreset {
        name: "T-Online",
        domains: &["t-online.de", "magenta.de"],
        imap_host: "secureimap.t-online.de",
        imap_port: 993,
        smtp_host: "securesmtp.t-online.de",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "Posteo",
        domains: &["posteo.de", "posteo.net", "posteo.org"],
        imap_host: "posteo.de",
        imap_port: 993,
        smtp_host: "posteo.de",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "mailbox",
        domains: &["mailbox.org"],
        imap_host: "imap.mailbox.org",
        imap_port: 993,
        smtp_host: "smtp.mailbox.org",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "mail.de",
        domains: &["mail.de"],
        imap_host: "imap.mail.de",
        imap_port: 993,
        smtp_host: "smtp.mail.de",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "freenet Mail",
        domains: &["freenet.de"],
        imap_host: "mx.freenet.de",
        imap_port: 993,
        smtp_host: "mx.freenet.de",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "iCloud Mail",
        domains: &["icloud.com", "me.com", "mac.com"],
        imap_host: "imap.mail.me.com",
        imap_port: 993,
        smtp_host: "smtp.mail.me.com",
        smtp_port: 587,
    },
    ProviderPreset {
        name: "Yahoo Mail",
        domains: &["yahoo.com", "yahoo.de", "ymail.com", "rocketmail.com"],
        imap_host: "imap.mail.yahoo.com",
        imap_port: 993,
        smtp_host: "smtp.mail.yahoo.com",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "Fastmail",
        domains: &["fastmail.com", "fastmail.fm"],
        imap_host: "imap.fastmail.com",
        imap_port: 993,
        smtp_host: "smtp.fastmail.com",
        smtp_port: 465,
    },
    ProviderPreset {
        name: "AOL Mail",
        domains: &["aol.com", "aol.de"],
        imap_host: "imap.aol.com",
        imap_port: 993,
        smtp_host: "smtp.aol.com",
        smtp_port: 465,
    },
];

const TRUSTED_HTTP_AUTOCONFIG_SUFFIXES: &[&str] = &[
    "dogado.net",
    "dogado.de",
    "ionos.de",
    "1und1.de",
    "strato.de",
    "hosteurope.de",
    "all-inkl.com",
    "kasserver.com",
    "hetzner.com",
    "mittwald.de",
];

#[derive(Debug, FromRow)]
struct StoredAccountConfig {
    id: String,
    email: String,
    config: String,
}

#[tauri::command]
pub async fn discover_account(input: DiscoverAccountInput) -> CommandResult<AccountDiscoveryDto> {
    let email = input.email.trim().to_ascii_lowercase();
    let (local_part, domain) = split_email(&email)?;
    if input.password.is_empty() {
        return Err(CommandError::new("backend.errors.accountPasswordRequired"));
    }

    let autoconfig = fetch_autoconfig(domain, &email).await;
    let usernames = username_candidates(
        input.username.as_deref(),
        &email,
        local_part,
        autoconfig.prefer_initial_last_username,
    );
    let preset = provider_preset(domain);
    let mut configured_imap = autoconfig.imap.clone();
    let mut configured_smtp = autoconfig.smtp.clone();
    if let Some(preset) = preset {
        configured_imap.push(MailCandidate {
            host: preset.imap_host.to_string(),
            port: preset.imap_port,
        });
        configured_smtp.push(MailCandidate {
            host: preset.smtp_host.to_string(),
            port: preset.smtp_port,
        });
    }
    // POP3 takes a separate path: it has no folder list to discover, and the
    // resulting config uses a `pop3` block the provider factory keys on.
    if input
        .protocol
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("pop3"))
    {
        return discover_pop3_account(&input, &email, domain, &usernames, &configured_smtp).await;
    }

    let imap_candidates = if let Some(server) = input.imap.as_ref() {
        vec![validated_override(server)?]
    } else {
        mail_candidates(
            domain,
            &configured_imap,
            "imap",
            &[993, 143],
            reverse_hostname(domain).await.as_deref(),
        )
    };

    let probe = discover_imap(&imap_candidates, &usernames, &input.password, &email).await;
    let (imap, username, folders) = probe.found.ok_or_else(|| match preset {
        Some(preset) => CommandError::new("backend.errors.imapPresetLoginRejected")
            .with_param("provider", preset.name),
        // The last error carries the reason — a certificate naming a different
        // host, a refused login, a timeout. Reporting only "nothing worked"
        // sends the user hunting for a wrong password they do not have.
        None => match probe.last_error {
            Some(reason) => CommandError::new("backend.errors.imapConnectionFailed")
                .with_param("reason", reason),
            None => CommandError::new("backend.errors.imapCredentialsRejected"),
        },
    })?;

    let smtp_candidates = if let Some(server) = input.smtp.as_ref() {
        vec![validated_override(server)?]
    } else {
        smtp_candidates(domain, &imap.host, &configured_smtp)
    };
    let smtp = discover_smtp(
        &smtp_candidates,
        &usernames,
        &username,
        &input.password,
        &email,
        &imap,
    )
    .await;

    let (caldav, carddav) = discover_dav(domain, &username, &input.password, None, None).await;
    let mut config = serde_json::json!({
        "imap": {
            "host": imap.host,
            "port": imap.port,
            "tls": true,
            "username": username,
            "password": input.password,
        }
    });
    let object = config.as_object_mut().expect("object literal");
    if let Some(smtp) = smtp.as_ref() {
        object.insert(
            "smtp".to_string(),
            serde_json::json!({
                "host": smtp.0.host,
                "port": smtp.0.port,
                "tls": true,
                "username": smtp.1,
                "password": input.password,
                "from_address": email,
            }),
        );
    }
    if let Some((server, _)) = caldav.as_ref() {
        object.insert(
            "caldav".to_string(),
            serde_json::to_value(DavConfig {
                server: server.clone(),
                username: username.clone(),
                password: input.password.clone(),
            })
            .map_err(|err| err.to_string())?,
        );
    }
    if let Some((server, _)) = carddav.as_ref() {
        object.insert(
            "carddav".to_string(),
            serde_json::to_value(DavConfig {
                server: server.clone(),
                username: username.clone(),
                password: input.password.clone(),
            })
            .map_err(|err| err.to_string())?,
        );
    }

    let mut warnings = Vec::new();
    if smtp.is_none() {
        warnings.push("The outgoing server could not be verified automatically.".to_string());
    }
    if caldav.is_none() && carddav.is_none() {
        warnings.push("No CalDAV or CardDAV service was advertised by this domain.".to_string());
    }
    if let Some(preset) = preset {
        warnings.push(format!(
            "Used the built-in {} provider profile.",
            preset.name
        ));
    }
    let smtp_service = smtp
        .as_ref()
        .map(|value| &value.0)
        .or_else(|| smtp_candidates.first());
    let services = vec![
        mail_service(
            "mail",
            "onboarding.services.mail",
            Some(&imap),
            true,
            Some(folders),
        ),
        mail_service(
            "smtp",
            "onboarding.services.smtp",
            smtp_service,
            smtp.is_some(),
            None,
        ),
        dav_service("caldav", "onboarding.services.calendar", caldav.as_ref()),
        dav_service("carddav", "onboarding.services.contacts", carddav.as_ref()),
    ];

    Ok(AccountDiscoveryDto {
        ready: smtp.is_some(),
        username,
        config,
        services,
        warnings,
    })
}

#[tauri::command]
pub async fn autoconfigure_groupware(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
) -> CommandResult<DavAutoconfigureDto> {
    let account = sqlx::query_as::<_, StoredAccountConfig>(
        "SELECT id, email, config FROM accounts WHERE id = ? AND is_active = 1 LIMIT 1",
    )
    .bind(&account_id)
    .fetch_one(pool.inner())
    .await
    .map_err(|err| err.to_string())?;
    let value: serde_json::Value = serde_json::from_str(
        &crate::commands::accounts::decrypt_account_config(&account.id, &account.config)?,
    )
    .map_err(|_| CommandError::new("backend.errors.accountConfigurationInvalid"))?;
    let imap = value.get("imap").unwrap_or(&value);
    let username = imap
        .get("username")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&account.email)
        .to_string();
    let password = imap
        .get("password")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CommandError::new("backend.errors.accountPasswordUnavailable"))?
        .to_string();
    let (_, domain) = split_email(&account.email)?;
    let current_caldav = value
        .pointer("/caldav/server")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let current_carddav = value
        .pointer("/carddav/server")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let (caldav, carddav) = discover_dav(
        domain,
        &username,
        &password,
        current_caldav.as_deref(),
        current_carddav.as_deref(),
    )
    .await;
    if caldav.is_none() && carddav.is_none() {
        return Err(CommandError::new(
            "backend.errors.groupwareCredentialsRejected",
        ));
    }

    let dav_config = |found: &Option<(String, usize)>| {
        found.as_ref().map(|(server, _)| DavConfig {
            server: server.clone(),
            username: username.clone(),
            password: password.clone(),
        })
    };
    let config = crate::db::accounts::AccountRepository::new(pool.inner())
        .update_dav_config(
            &account.id,
            dav_config(&caldav),
            dav_config(&carddav),
            false,
        )
        .await?;
    let summary = dav::sync_groupware_account(pool.inner(), &account.id, &config).await?;

    Ok(DavAutoconfigureDto {
        caldav_server: caldav.map(|value| value.0),
        carddav_server: carddav.map(|value| value.0),
        summary,
    })
}

fn split_email(email: &str) -> CommandResult<(&str, &str)> {
    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| CommandError::new("backend.errors.emailInvalid"))?;
    if local.trim().is_empty() || domain.trim().is_empty() || domain.contains(char::is_whitespace) {
        return Err(CommandError::new("backend.errors.emailInvalid"));
    }
    Ok((local, domain))
}

fn username_candidates(
    explicit: Option<&str>,
    email: &str,
    local: &str,
    prefer_initial_last: bool,
) -> Vec<String> {
    let derived = prefer_initial_last
        .then(|| initial_last_username(local))
        .flatten()
        .unwrap_or_default();
    unique_strings(
        [explicit.unwrap_or(""), derived.as_str(), email, local]
            .into_iter()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
}

fn initial_last_username(local: &str) -> Option<String> {
    let (first, last) = local
        .split_once('.')
        .or_else(|| local.split_once('-'))
        .or_else(|| local.split_once('_'))?;
    let initial = first.chars().next()?;
    (!last.is_empty()).then(|| format!("{initial}{last}"))
}

fn provider_preset(domain: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS
        .iter()
        .find(|preset| preset.domains.contains(&domain))
}

fn validated_override(server: &ServerOverride) -> CommandResult<MailCandidate> {
    let host = server
        .host
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || server.port == 0 {
        return Err(CommandError::new(
            "backend.errors.serverHostAndPortRequired",
        ));
    }
    Ok(MailCandidate {
        host,
        port: server.port,
    })
}

/// The canonical name of the machine a domain points at.
///
/// Shared hosting is the normal case for a small domain: `example.de` resolves
/// to a box the provider certified under its *own* name, so `imap.example.de`
/// reaches a live server that then fails hostname verification against a
/// wildcard like `*.provider.net`. The reverse record is what that box calls
/// itself, and therefore the name its certificate actually covers.
///
/// Blocking resolver calls, so this runs on the blocking pool. Any failure is
/// simply one fewer candidate.
async fn reverse_hostname(domain: &str) -> Option<String> {
    let queried = domain.to_ascii_lowercase();
    let lookup = queried.clone();
    let resolved = tauri::async_runtime::spawn_blocking(move || {
        let addr = dns_lookup::lookup_host(&lookup).ok()?.next()?;
        let name = dns_lookup::lookup_addr(&addr).ok()?;
        Some(name.trim_end_matches('.').to_ascii_lowercase())
    })
    .await
    .ok()??;

    // A resolver that echoes the query back, or points at the domain itself,
    // adds nothing over the names already being tried.
    (resolved != queried && resolved.contains('.')).then_some(resolved)
}

fn mail_candidates(
    domain: &str,
    configured: &[MailCandidate],
    prefix: &str,
    ports: &[u16],
    reverse: Option<&str>,
) -> Vec<MailCandidate> {
    let mut values = configured.to_vec();
    let mut hosts = vec![
        format!("{prefix}.{domain}"),
        format!("mail.{domain}"),
        domain.to_string(),
    ];
    if let Some(reverse) = reverse {
        hosts.push(reverse.to_string());
    }
    for host in hosts {
        for port in ports {
            values.push(MailCandidate {
                host: host.clone(),
                port: *port,
            });
        }
    }
    unique_mail_candidates(values)
}

fn smtp_candidates(
    domain: &str,
    imap_host: &str,
    configured: &[MailCandidate],
) -> Vec<MailCandidate> {
    let derived = imap_host
        .strip_prefix("imap.")
        .map(|suffix| format!("smtp.{suffix}"));
    let mut values = configured.to_vec();
    let hosts = [
        derived.unwrap_or_else(|| format!("smtp.{domain}")),
        format!("smtp.{domain}"),
        format!("mail.{domain}"),
        imap_host.to_string(),
    ];
    for host in hosts {
        for port in [465, 587] {
            values.push(MailCandidate {
                host: host.clone(),
                port,
            });
        }
    }
    unique_mail_candidates(values)
}

/// Outcome of probing every IMAP candidate.
///
/// Carries the last error on failure. Discovery used to drop them all and
/// report "nothing accepted these credentials", which reads as a wrong password
/// even when the real answer was a certificate naming a different host — the
/// one piece of information that would have told the user what to enter.
struct ImapProbe {
    found: Option<(MailCandidate, String, usize)>,
    last_error: Option<String>,
}

async fn discover_imap(
    candidates: &[MailCandidate],
    usernames: &[String],
    password: &str,
    email: &str,
) -> ImapProbe {
    let mut last_error: Option<String> = None;
    for username in usernames {
        let mut attempts = FuturesUnordered::new();
        for candidate in candidates.iter().cloned() {
            let username = username.clone();
            let password = password.to_string();
            let email = email.to_string();
            attempts.push(async move {
                let config = account_config(&candidate, &candidate, &username, &password, &email);
                let provider = match ImapProvider::from_account_config(&config.to_string(), &email)
                {
                    Ok(provider) => provider,
                    Err(err) => return Err((candidate, err)),
                };
                match timeout(DISCOVERY_TIMEOUT, provider.sync_folders()).await {
                    Ok(Ok(folders)) => Ok((candidate, username, folders.len())),
                    Ok(Err(err)) => Err((candidate, err.to_string())),
                    Err(_) => Err((candidate, "timed out".to_string())),
                }
            });
        }
        while let Some(result) = attempts.next().await {
            match result {
                Ok(found) => {
                    return ImapProbe {
                        found: Some(found),
                        last_error: None,
                    }
                }
                Err((candidate, err)) => {
                    last_error = Some(format!("{}:{} — {err}", candidate.host, candidate.port));
                }
            }
        }
    }
    ImapProbe {
        found: None,
        last_error,
    }
}

async fn discover_smtp(
    candidates: &[MailCandidate],
    usernames: &[String],
    preferred_username: &str,
    password: &str,
    email: &str,
    imap: &MailCandidate,
) -> Option<(MailCandidate, String)> {
    let ordered_usernames = unique_strings(
        std::iter::once(preferred_username).chain(usernames.iter().map(String::as_str)),
    );
    for username in ordered_usernames {
        let mut attempts = FuturesUnordered::new();
        for candidate in candidates.iter().cloned() {
            let username = username.clone();
            let password = password.to_string();
            let email = email.to_string();
            let imap = imap.clone();
            attempts.push(async move {
                let config = account_config(&imap, &candidate, &username, &password, &email);
                let provider =
                    ImapProvider::from_account_config(&config.to_string(), &email).ok()?;
                timeout(DISCOVERY_TIMEOUT, provider.test_smtp_connection())
                    .await
                    .ok()?
                    .ok()?;
                Some((candidate, username))
            });
        }
        while let Some(result) = attempts.next().await {
            if result.is_some() {
                return result;
            }
        }
    }
    None
}

async fn discover_dav(
    domain: &str,
    username: &str,
    password: &str,
    current_caldav: Option<&str>,
    current_carddav: Option<&str>,
) -> (Option<(String, usize)>, Option<(String, usize)>) {
    let caldav = dav_candidates(domain, current_caldav, true);
    let carddav = dav_candidates(domain, current_carddav, false);
    tokio::join!(
        first_dav(caldav, username, password, true),
        first_dav(carddav, username, password, false)
    )
}

fn dav_candidates(domain: &str, current: Option<&str>, calendar: bool) -> Vec<String> {
    let kind = if calendar { "caldav" } else { "carddav" };
    unique_strings(
        current.into_iter().chain(
            [
                format!("https://{domain}/.well-known/{kind}"),
                format!("https://{kind}.{domain}"),
                format!("https://gw{kind}.{domain}"),
            ]
            .iter()
            .map(String::as_str),
        ),
    )
}

async fn first_dav(
    candidates: Vec<String>,
    username: &str,
    password: &str,
    calendar: bool,
) -> Option<(String, usize)> {
    let mut attempts = FuturesUnordered::new();
    for server in candidates {
        let username = username.to_string();
        let password = password.to_string();
        attempts.push(async move {
            timeout(
                DISCOVERY_TIMEOUT,
                dav::probe_dav_server(&server, &username, &password, calendar),
            )
            .await
            .ok()?
            .ok()
        });
    }
    while let Some(result) = attempts.next().await {
        if result.is_some() {
            return result;
        }
    }
    None
}

fn account_config(
    imap: &MailCandidate,
    smtp: &MailCandidate,
    username: &str,
    password: &str,
    email: &str,
) -> serde_json::Value {
    serde_json::json!({
        "imap": {
            "host": imap.host,
            "port": imap.port,
            "tls": true,
            "username": username,
            "password": password,
        },
        "smtp": {
            "host": smtp.host,
            "port": smtp.port,
            "tls": true,
            "username": username,
            "password": password,
            "from_address": email,
        }
    })
}

fn mail_service(
    kind: &str,
    label: &str,
    candidate: Option<&MailCandidate>,
    connected: bool,
    collections: Option<usize>,
) -> DiscoveredService {
    DiscoveredService {
        kind: kind.to_string(),
        status: if connected {
            "connected"
        } else {
            "needs_input"
        }
        .to_string(),
        label: label.to_string(),
        host: candidate.map(|value| value.host.clone()),
        port: candidate.map(|value| value.port),
        collections,
    }
}

fn dav_service(kind: &str, label: &str, value: Option<&(String, usize)>) -> DiscoveredService {
    DiscoveredService {
        kind: kind.to_string(),
        status: if value.is_some() {
            "connected"
        } else {
            "not_found"
        }
        .to_string(),
        label: label.to_string(),
        host: value.map(|(server, _)| server.clone()),
        port: None,
        collections: value.map(|(_, collections)| *collections),
    }
}

async fn fetch_autoconfig(domain: &str, email: &str) -> Autoconfig {
    let Ok(client) = satchel_core::utils::http::client_builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return Autoconfig::default();
    };
    let mut provider_url =
        match reqwest::Url::parse(&format!("https://autoconfig.{domain}/mail/config-v1.1.xml")) {
            Ok(url) => url,
            Err(_) => return Autoconfig::default(),
        };
    provider_url
        .query_pairs_mut()
        .append_pair("emailaddress", email);
    let mut well_known_url = match reqwest::Url::parse(&format!(
        "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"
    )) {
        Ok(url) => url,
        Err(_) => return Autoconfig::default(),
    };
    well_known_url
        .query_pairs_mut()
        .append_pair("emailaddress", email);
    let urls = [
        provider_url.to_string(),
        well_known_url.to_string(),
        format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
    ];
    let mut attempts = FuturesUnordered::new();
    for url in urls {
        let client = client.clone();
        attempts.push(async move {
            let response = client.get(url).send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let body = response.text().await.ok()?;
            let parsed = parse_autoconfig_xml(&body);
            (!parsed.imap.is_empty()).then_some(parsed)
        });
    }
    while let Some(result) = attempts.next().await {
        if let Some(config) = result {
            return config;
        }
    }

    // Some established hosting platforms still publish Thunderbird autoconfig only over HTTP.
    // Such responses are accepted solely when every advertised credential endpoint belongs to a
    // curated provider suffix. Credentials are subsequently sent only after normal TLS certificate
    // validation succeeds for that advertised endpoint.
    //
    // Deliberately without `emailaddress`: this request is in the clear, and
    // the address is the one thing in it worth reading. The servers this
    // fallback exists for answer with the domain's configuration either way.
    let Ok(legacy_url) =
        reqwest::Url::parse(&format!("http://autoconfig.{domain}/mail/config-v1.1.xml"))
    else {
        return Autoconfig::default();
    };
    if let Ok(response) = client.get(legacy_url).send().await {
        if response.status().is_success() {
            if let Ok(body) = response.text().await {
                let parsed = parse_autoconfig_xml(&body);
                if trusted_http_autoconfig(&parsed) {
                    return parsed;
                }
            }
        }
    }
    Autoconfig::default()
}

fn trusted_http_autoconfig(config: &Autoconfig) -> bool {
    !config.imap.is_empty()
        && !config.smtp.is_empty()
        && config
            .imap
            .iter()
            .chain(config.smtp.iter())
            .all(|candidate| {
                TRUSTED_HTTP_AUTOCONFIG_SUFFIXES
                    .iter()
                    .any(|suffix| host_matches_suffix(&candidate.host, suffix))
            })
}

fn host_matches_suffix(host: &str, suffix: &str) -> bool {
    host == suffix
        || host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn parse_autoconfig_xml(xml: &str) -> Autoconfig {
    let mut parsed = Autoconfig {
        prefer_initial_last_username: xml.to_ascii_lowercase().contains("campusid"),
        ..Default::default()
    };
    for (tag, target) in [
        ("incomingServer", &mut parsed.imap),
        ("outgoingServer", &mut parsed.smtp),
    ] {
        for block in xml_blocks(xml, tag) {
            if tag == "incomingServer"
                && !block.opening.contains("type=\"imap\"")
                && !block.opening.contains("type='imap'")
            {
                continue;
            }
            let Some(host) = xml_tag_text(block.body, "hostname") else {
                continue;
            };
            let Some(port) =
                xml_tag_text(block.body, "port").and_then(|value| value.parse::<u16>().ok())
            else {
                continue;
            };
            let socket = xml_tag_text(block.body, "socketType")
                .unwrap_or_default()
                .to_ascii_uppercase();
            if socket != "SSL" && socket != "SSL/TLS" && socket != "STARTTLS" {
                continue;
            }
            target.push(MailCandidate {
                host: host.trim().trim_end_matches('.').to_ascii_lowercase(),
                port,
            });
        }
    }
    parsed.imap = unique_mail_candidates(parsed.imap);
    parsed.smtp = unique_mail_candidates(parsed.smtp);
    parsed
}

struct XmlBlock<'a> {
    opening: &'a str,
    body: &'a str,
}

fn xml_blocks<'a>(xml: &'a str, tag: &str) -> Vec<XmlBlock<'a>> {
    let mut blocks = Vec::new();
    let mut remainder = xml;
    let start = format!("<{tag}");
    let end = format!("</{tag}>");
    while let Some(open_at) = remainder.find(&start) {
        let after_open = &remainder[open_at..];
        let Some(open_end) = after_open.find('>') else {
            break;
        };
        let body_start = open_end + 1;
        let Some(close_at) = after_open[body_start..].find(&end) else {
            break;
        };
        blocks.push(XmlBlock {
            opening: &after_open[..=open_end],
            body: &after_open[body_start..body_start + close_at],
        });
        remainder = &after_open[body_start + close_at + end.len()..];
    }
    blocks
}

fn xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn unique_mail_candidates(values: Vec<MailCandidate>) -> Vec<MailCandidate> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.host.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn unique_strings<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

/// Implicit TLS only. Port 110 is plaintext — this client has no `STLS` — so
/// probing it would end in a login the user never agreed to send in the clear.
const POP3_DISCOVERY_PORTS: &[u16] = &[995];

/// The POP3 half of a discovered account config.
///
/// `tls` is constant, never derived from the port: the port is the one thing an
/// on-path attacker can steer, and a derived flag would store the downgrade.
fn discovered_pop3_config(
    candidate: &MailCandidate,
    username: &str,
    password: &str,
) -> serde_json::Value {
    serde_json::json!({
        "host": candidate.host,
        "port": candidate.port,
        "tls": true,
        "username": username,
        "password": password,
    })
}

/// Discover a POP3 account.
///
/// Deliberately simpler than the IMAP path: POP3 has no folders to enumerate
/// and no capabilities to probe, so a successful login plus STAT is the whole
/// test.
async fn discover_pop3_account(
    input: &DiscoverAccountInput,
    email: &str,
    domain: &str,
    usernames: &[String],
    configured_smtp: &[MailCandidate],
) -> CommandResult<AccountDiscoveryDto> {
    let candidates = if let Some(server) = input.pop3.as_ref() {
        vec![MailCandidate {
            host: server.host.clone(),
            port: server.port,
        }]
    } else {
        mail_candidates(
            domain,
            &[],
            "pop",
            POP3_DISCOVERY_PORTS,
            reverse_hostname(domain).await.as_deref(),
        )
    };

    let mut accepted: Option<(MailCandidate, String)> = None;
    'outer: for candidate in &candidates {
        for username in usernames {
            let config = serde_json::json!({
                "pop3": discovered_pop3_config(candidate, username, &input.password),
            })
            .to_string();
            let Ok(provider) =
                crate::providers::pop3::Pop3Provider::from_account_config(&config, username)
            else {
                continue;
            };
            if provider.verify_credentials().await.is_ok() {
                accepted = Some((candidate.clone(), username.clone()));
                break 'outer;
            }
        }
    }

    let (pop3, username) =
        accepted.ok_or_else(|| CommandError::new("backend.errors.pop3CredentialsRejected"))?;

    let smtp_candidates = smtp_candidates(domain, &pop3.host, configured_smtp);
    let smtp = discover_smtp(
        &smtp_candidates,
        usernames,
        &username,
        &input.password,
        email,
        &pop3,
    )
    .await;

    let mut pop3_config = discovered_pop3_config(&pop3, &username, &input.password);
    // Never delete the user's only copy without being asked.
    pop3_config["leave_on_server"] = serde_json::Value::Bool(true);
    let mut config = serde_json::json!({ "pop3": pop3_config });
    let object = config.as_object_mut().expect("object literal");
    if let Some(smtp) = smtp.as_ref() {
        object.insert(
            "smtp".to_string(),
            serde_json::json!({
                "host": smtp.0.host,
                "port": smtp.0.port,
                "tls": true,
                "username": smtp.1,
                "password": input.password,
                "from_address": email,
            }),
        );
    }

    let mut warnings = vec![
        "POP3 has no folders and no server-side flags: archiving, stars and read state stay on \
         this device."
            .to_string(),
    ];
    if smtp.is_none() {
        warnings.push("The outgoing server could not be verified automatically.".to_string());
    }

    Ok(AccountDiscoveryDto {
        ready: true,
        username,
        config,
        services: vec![DiscoveredService {
            kind: "mail".to_string(),
            status: "connected".to_string(),
            label: format!("POP3 {}:{}", pop3.host, pop3.port),
            host: Some(pop3.host),
            port: Some(pop3.port),
            collections: None,
        }],
        warnings,
    })
}

/// The OAuth provider that receives mail for `domain`, from its MX records.
///
/// A company domain on Google Workspace or Microsoft 365 looks like any other
/// domain by name, and the IMAP password form fails there: both want OAuth.
/// `None` covers everything else, including a failed or slow lookup; the
/// person can still pick the method by hand.
#[tauri::command]
pub async fn detect_mail_provider(domain: String) -> CommandResult<Option<String>> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if !plausible_mail_domain(&domain) {
        return Ok(None);
    }
    match timeout(MX_LOOKUP_TIMEOUT, mx_hosts(&domain)).await {
        Ok(Ok(hosts)) => Ok(provider_for_mx_hosts(&hosts).map(str::to_string)),
        Ok(Err(error)) => {
            tracing::debug!(%domain, %error, "MX lookup failed");
            Ok(None)
        }
        Err(_) => {
            tracing::debug!(%domain, "MX lookup timed out");
            Ok(None)
        }
    }
}

fn plausible_mail_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain
            .split('.')
            .all(|label| !label.is_empty() && label.len() <= 63)
        && domain
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '.')
}

/// MX exchanges, most preferred first.
async fn mx_hosts(domain: &str) -> Result<Vec<String>, String> {
    use hickory_resolver::proto::rr::RData;

    let resolver = hickory_resolver::TokioResolver::builder_tokio()
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    // The trailing dot keeps the search list out of it.
    let lookup = resolver
        .mx_lookup(format!("{domain}."))
        .await
        .map_err(|error| error.to_string())?;
    let mut records = lookup
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::MX(mx) => Some((mx.preference, mx.exchange.to_ascii())),
            _ => None,
        })
        .collect::<Vec<_>>();
    records.sort();
    Ok(records.into_iter().map(|(_, host)| host).collect())
}

/// Google Workspace (`aspmx.l.google.com`, `smtp.google.com`, the old
/// `googlemail.com` hosts) and Microsoft 365 (`*.mail.protection.outlook.com`).
/// The most preferred exchange decides; a filter service in front hides both.
fn provider_for_mx_hosts(hosts: &[String]) -> Option<&'static str> {
    let host = hosts.first()?.trim_end_matches('.').to_ascii_lowercase();
    if host.ends_with(".google.com") || host.ends_with(".googlemail.com") {
        Some("gmail")
    } else if host.ends_with(".protection.outlook.com") {
        Some("outlook")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_secure_thunderbird_autoconfig() {
        let xml = r#"
          <clientConfig><emailProvider>
            <incomingServer type="imap"><hostname>imap.example.org</hostname><port>993</port><socketType>SSL</socketType></incomingServer>
            <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
            <outgoingServer type="smtp"><hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
          </emailProvider></clientConfig>"#;
        let parsed = parse_autoconfig_xml(xml);
        assert_eq!(
            parsed.imap,
            vec![MailCandidate {
                host: "imap.example.org".into(),
                port: 993
            }]
        );
        assert_eq!(
            parsed.smtp,
            vec![MailCandidate {
                host: "smtp.example.org".into(),
                port: 587
            }]
        );
    }

    #[test]
    fn ignores_insecure_autoconfig_entries() {
        let xml = r#"<incomingServer type="imap"><hostname>imap.example.org</hostname><port>143</port><socketType>plain</socketType></incomingServer>"#;
        assert!(parse_autoconfig_xml(xml).imap.is_empty());
    }

    #[test]
    fn prefers_explicit_then_email_then_local_username() {
        assert_eq!(
            username_candidates(Some("employee"), "person@example.org", "person", false),
            vec!["employee", "person@example.org", "person"]
        );
        assert_eq!(
            username_candidates(
                Some("person@example.org"),
                "person@example.org",
                "person",
                false,
            ),
            vec!["person@example.org", "person"]
        );
    }

    #[test]
    fn derives_campus_id_when_autoconfig_requests_it() {
        let xml = r#"
          <clientConfig>
            <emailProvider>
              <userinput label="CampusID" key="%USERNAME%">CampusID</userinput>
              <incomingServer type="imap">
                <hostname>gwimap.college.example</hostname><port>993</port><socketType>SSL</socketType>
              </incomingServer>
              <outgoingServer type="smtp">
                <hostname>smtp.intranet.college.example</hostname><port>587</port><socketType>STARTTLS</socketType>
              </outgoingServer>
            </emailProvider>
          </clientConfig>"#;
        let parsed = parse_autoconfig_xml(xml);
        assert!(parsed.prefer_initial_last_username);
        assert_eq!(
            username_candidates(
                None,
                "alex.morgan@college.example",
                "alex.morgan",
                parsed.prefer_initial_last_username,
            ),
            vec!["amorgan", "alex.morgan@college.example", "alex.morgan"]
        );
        assert_eq!(parsed.smtp[0].host, "smtp.intranet.college.example");
        assert_eq!(parsed.smtp[0].port, 587);
    }

    #[test]
    fn includes_groupwise_dav_conventions() {
        let candidates = dav_candidates("example.org", None, true);
        assert!(candidates.contains(&"https://gwcaldav.example.org".to_string()));
        assert!(candidates.contains(&"https://example.org/.well-known/caldav".to_string()));
    }

    /// Shared hosting is the case automatic discovery kept failing on: the
    /// customer domain points at a provider machine certified under its own
    /// name, so every guessed `imap.<domain>` reaches a live server and then
    /// fails hostname verification.
    #[test]
    fn offers_the_reverse_hostname_alongside_the_guessed_ones() {
        let candidates = mail_candidates(
            "example.de",
            &[],
            "imap",
            &[993],
            Some("fixture-host.provider.net"),
        );
        let hosts: Vec<&str> = candidates.iter().map(|c| c.host.as_str()).collect();

        assert!(hosts.contains(&"imap.example.de"));
        assert!(hosts.contains(&"fixture-host.provider.net"));
        // The guessed names stay ahead of it: a domain that runs its own mail
        // server should not be dragged onto the hoster's shared box.
        assert!(
            hosts.iter().position(|h| *h == "imap.example.de")
                < hosts.iter().position(|h| *h == "fixture-host.provider.net")
        );
    }

    #[test]
    fn omits_the_reverse_hostname_when_there_is_none() {
        let candidates = mail_candidates("example.de", &[], "imap", &[993], None);
        assert!(candidates.iter().all(|c| c.host.ends_with("example.de")));
    }

    /// A discovery that falls back to 110 hands the password to whoever
    /// blocked 995.
    #[test]
    fn pop3_discovery_offers_no_plaintext_port_and_stores_no_downgrade() {
        assert_eq!(POP3_DISCOVERY_PORTS, &[995]);
        let candidates = mail_candidates("example.de", &[], "pop", POP3_DISCOVERY_PORTS, None);
        assert!(candidates.iter().all(|candidate| candidate.port == 995));

        let config = discovered_pop3_config(
            &MailCandidate {
                host: "pop.example.de".into(),
                port: 110,
            },
            "me@example.de",
            "secret",
        );
        assert_eq!(config["tls"], serde_json::json!(true));
    }

    #[test]
    fn includes_popular_provider_presets() {
        let web = provider_preset("web.de").expect("WEB.DE preset");
        assert_eq!(web.imap_host, "imap.web.de");
        assert_eq!(web.smtp_host, "smtp.web.de");

        let mailbox = provider_preset("mailbox.org").expect("mailbox preset");
        assert_eq!(mailbox.imap_port, 993);
        assert_eq!(mailbox.smtp_port, 465);
    }

    #[test]
    fn accepts_only_curated_hosts_from_plain_http_autoconfig() {
        let dogado = Autoconfig {
            imap: vec![MailCandidate {
                host: "fixture-mail.dogado.net".into(),
                port: 993,
            }],
            smtp: vec![MailCandidate {
                host: "fixture-mail.dogado.net".into(),
                port: 465,
            }],
            ..Default::default()
        };
        assert!(trusted_http_autoconfig(&dogado));

        let attacker = Autoconfig {
            imap: vec![MailCandidate {
                host: "dogado.net.attacker.example".into(),
                port: 993,
            }],
            smtp: vec![MailCandidate {
                host: "smtp.attacker.example".into(),
                port: 465,
            }],
            ..Default::default()
        };
        assert!(!trusted_http_autoconfig(&attacker));
    }

    #[test]
    fn mx_hosts_name_the_oauth_provider_behind_a_company_domain() {
        let hosts = |list: &[&str]| list.iter().map(|host| host.to_string()).collect::<Vec<_>>();
        assert_eq!(
            provider_for_mx_hosts(&hosts(&["aspmx.l.google.com.", "alt1.aspmx.l.google.com."])),
            Some("gmail")
        );
        assert_eq!(
            provider_for_mx_hosts(&hosts(&["smtp.google.com."])),
            Some("gmail")
        );
        assert_eq!(
            provider_for_mx_hosts(&hosts(&["ASPMX2.GOOGLEMAIL.COM"])),
            Some("gmail")
        );
        assert_eq!(
            provider_for_mx_hosts(&hosts(&["contoso-com.mail.protection.outlook.com."])),
            Some("outlook")
        );
        assert_eq!(provider_for_mx_hosts(&hosts(&["mx01.ionos.de."])), None);
        // A lookalike host is not Google.
        assert_eq!(provider_for_mx_hosts(&hosts(&["mx.notgoogle.com."])), None);
        // The preferred exchange decides, not a fallback further down.
        assert_eq!(
            provider_for_mx_hosts(&hosts(&["mx1.filter.example.", "aspmx.l.google.com."])),
            None
        );
        assert_eq!(provider_for_mx_hosts(&[]), None);
    }

    #[test]
    fn only_domain_shaped_input_reaches_the_resolver() {
        assert!(plausible_mail_domain("acme.example"));
        assert!(plausible_mail_domain("müller.example"));
        assert!(!plausible_mail_domain("localhost"));
        assert!(!plausible_mail_domain("acme..example"));
        assert!(!plausible_mail_domain("acme.example/x"));
        assert!(!plausible_mail_domain(""));
    }
}
