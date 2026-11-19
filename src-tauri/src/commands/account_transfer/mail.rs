use super::{failed, CommandResult};
use satchel_core::db::accounts::{AccountRepository, CreateEmailAccount};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mailbox {
    pub(super) email: String,
    name: String,
    label: Option<String>,
    pub(super) provider: String,
    config: Value,
}

const CONNECTION: &[&str] = &[
    "host",
    "port",
    "tls",
    "username",
    "password",
    "from",
    "from_address",
];
const FLAT_SMTP: &[&str] = &[
    "smtp_host",
    "smtp_port",
    "smtp_tls",
    "smtp_username",
    "smtp_password",
    "smtp_from",
    "smtp_from_address",
];

fn fields(value: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|&key| {
            let v = value.get(key)?;
            (v.is_string() || v.is_boolean() || v.is_number()).then(|| (key.to_owned(), v.clone()))
        })
        .collect()
}

// Apply the same allowlist before sending and before importing. Provider
// configs can also contain OAuth, DAV, or local certificate decisions.
fn portable(provider: &str, source: &Value) -> CommandResult<Value> {
    if matches!(provider, "gmail" | "outlook") {
        return Ok(json!({}));
    }
    if !matches!(provider, "imap" | "pop3") {
        return Err(failed());
    }
    let incoming = source.get(provider).unwrap_or(source);
    for key in ["host", "password"] {
        if incoming[key].as_str().is_none_or(str::is_empty) {
            return Err(failed());
        }
    }
    let mut config = fields(incoming, CONNECTION);
    config.extend(fields(incoming, FLAT_SMTP));
    let mut result = Map::new();
    result.insert(provider.to_owned(), Value::Object(config));
    if let Some(smtp) = source.get("smtp").or_else(|| incoming.get("smtp")) {
        result.insert("smtp".into(), Value::Object(fields(smtp, CONNECTION)));
    }
    Ok(Value::Object(result))
}

pub(super) async fn selected(
    pool: &sqlx::SqlitePool,
    ids: &[String],
) -> CommandResult<Vec<Mailbox>> {
    let repo = AccountRepository::new(pool);
    let accounts = repo.list_active().await?;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    if !(1..=64).contains(&ids.len()) {
        return Err(failed());
    }
    for id in ids {
        let a = accounts
            .iter()
            .find(|a| a.id == *id && a.kind == "email")
            .ok_or_else(failed)?;
        if !seen.insert(id) {
            return Err(failed());
        }
        let mut config = Value::Null;
        if matches!(a.provider.as_str(), "imap" | "pop3") {
            let stored = repo.load_provider_config(id).await?.ok_or_else(failed)?;
            config = serde_json::from_str(&stored.config).map_err(|_| failed())?;
        }
        result.push(Mailbox {
            email: a.email.clone(),
            name: a.display_name.clone(),
            label: a.label.clone(),
            provider: a.provider.clone(),
            config: portable(&a.provider, &config)?,
        });
    }
    Ok(result)
}

pub(super) fn decode(bytes: &[u8]) -> CommandResult<Vec<CreateEmailAccount>> {
    let list: Vec<Mailbox> = serde_json::from_slice(bytes).map_err(|_| failed())?;
    if !(1..=64).contains(&list.len()) {
        return Err(failed());
    }
    list.into_iter()
        .map(|a| {
            if a.email.len() > 320
                || !a.email.contains('@')
                || a.email.chars().any(char::is_control)
            {
                return Err(failed());
            }
            let config = portable(&a.provider, &a.config)?;
            Ok(CreateEmailAccount {
                email: a.email,
                display_name: a.name,
                label: a.label,
                provider: a.provider,
                config: Some(config),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_mail_credentials_cross_devices() {
        let input = json!({"imap":{"host":"mail.test","password":"incoming","trusted_certificate":"x","smtp_password":"outgoing"},"smtp":{"username":"sender","password":"smtp","certificate":"x"},"oauth":{"refresh_token":"private"},"dav":{"password":"private"}});
        assert_eq!(
            portable("imap", &input).unwrap(),
            json!({"imap":{"host":"mail.test","password":"incoming","smtp_password":"outgoing"},"smtp":{"username":"sender","password":"smtp"}})
        );
        assert_eq!(portable("gmail", &input).unwrap(), json!({}));
        assert!(portable("script", &input).is_err());
    }
}
