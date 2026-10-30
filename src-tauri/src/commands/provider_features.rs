use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::commands::error::CommandResult;
use crate::utils::oauth::{access_token_from_account_config, OAuthProvider};

#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/provider_features/")
)]
pub struct ProviderLabelDto {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    pub kind: Option<String>,
    pub visible: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/provider_features/", optional_fields = nullable))]
pub struct CreateProviderLabelInput {
    pub account_id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, FromRow)]
struct StoredAccountFeatureRow {
    id: String,
    provider: String,
    config: String,
}

struct AccountFeatureRow {
    id: String,
    provider: String,
    config: crate::commands::accounts::AccountConfig,
}

#[derive(Debug, Deserialize)]
struct GmailLabelsResponse {
    labels: Option<Vec<GmailLabel>>,
}

#[derive(Debug, Deserialize)]
struct GmailLabel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "labelListVisibility")]
    label_list_visibility: Option<String>,
    color: Option<GmailLabelColor>,
}

#[derive(Debug, Deserialize)]
struct GmailLabelColor {
    #[serde(rename = "backgroundColor")]
    background_color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OutlookListResponse<T> {
    value: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct OutlookCategory {
    id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: String,
    color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreatedLabelResponse {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[tauri::command]
pub async fn list_provider_labels(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<ProviderLabelDto>> {
    list_provider_labels_inner(pool.inner().as_ref(), account_id).await
}

async fn list_provider_labels_inner(
    pool: &SqlitePool,
    account_id: Option<String>,
) -> CommandResult<Vec<ProviderLabelDto>> {
    match account_id {
        Some(account_id) => sqlx::query_as::<_, ProviderLabelDto>(
            "SELECT id, account_id, provider, remote_id, name, color, kind, visible, updated_at
             FROM provider_labels WHERE account_id = ? ORDER BY name ASC",
        )
        .bind(account_id)
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string()),
        None => sqlx::query_as::<_, ProviderLabelDto>(
            "SELECT id, account_id, provider, remote_id, name, color, kind, visible, updated_at
             FROM provider_labels ORDER BY name ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string()),
    }
    .map_err(Into::into)
}

#[tauri::command]
pub async fn refresh_provider_labels(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
) -> CommandResult<Vec<ProviderLabelDto>> {
    let account = load_account(pool.inner(), &account_id).await?;
    match account.provider.as_str() {
        "gmail" => refresh_gmail_labels(pool.inner(), &account).await?,
        "outlook" => refresh_outlook_categories(pool.inner(), &account).await?,
        _ => refresh_imap_labels_from_folders(pool.inner(), &account).await?,
    }
    list_provider_labels_inner(pool.inner(), Some(account_id)).await
}

#[tauri::command]
pub async fn create_provider_label(
    pool: tauri::State<'_, SqlitePool>,
    input: CreateProviderLabelInput,
) -> CommandResult<ProviderLabelDto> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("label name is required".into());
    }
    let account = load_account(pool.inner(), &input.account_id).await?;
    match account.provider.as_str() {
        "gmail" => create_gmail_label(pool.inner(), &account, name, input.color.as_deref()).await?,
        "outlook" => {
            create_outlook_category(pool.inner(), &account, name, input.color.as_deref()).await?
        }
        _ => {
            upsert_provider_label(
                pool.inner(),
                &account.id,
                &account.provider,
                name,
                name,
                input.color.as_deref(),
                Some("folder"),
                true,
            )
            .await?
        }
    }
    sqlx::query_as::<_, ProviderLabelDto>(
        "SELECT id, account_id, provider, remote_id, name, color, kind, visible, updated_at
         FROM provider_labels WHERE account_id = ? AND name = ? ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(input.account_id)
    .bind(name)
    .fetch_one(pool.inner())
    .await
    .map_err(|err| err.to_string())
    .map_err(Into::into)
}

async fn load_account(pool: &SqlitePool, account_id: &str) -> Result<AccountFeatureRow, String> {
    let row = sqlx::query_as::<_, StoredAccountFeatureRow>(
        "SELECT id, provider, config FROM accounts WHERE id = ? LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(AccountFeatureRow {
        config: crate::utils::oauth::refresh_account_config(
            pool,
            &row.id,
            &row.provider,
            &row.config,
        )
        .await?,
        id: row.id,
        provider: row.provider,
    })
}

async fn refresh_gmail_labels(
    pool: &SqlitePool,
    account: &AccountFeatureRow,
) -> Result<(), String> {
    let token = access_token_from_account_config(OAuthProvider::Gmail, &account.config).await?;
    let response = satchel_core::utils::http::client()
        .get("https://gmail.googleapis.com/gmail/v1/users/me/labels")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| format!("gmail labels request failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "gmail labels request failed with status {status}: {body}"
        ));
    }
    let payload: GmailLabelsResponse = response
        .json()
        .await
        .map_err(|err| format!("gmail labels parse failed: {err}"))?;
    for label in payload.labels.unwrap_or_default() {
        let visible = label
            .label_list_visibility
            .as_deref()
            .map(|value| value != "labelHide")
            .unwrap_or(true);
        upsert_provider_label(
            pool,
            &account.id,
            "gmail",
            &label.id,
            &label.name,
            label.color.and_then(|c| c.background_color).as_deref(),
            label.kind.as_deref(),
            visible,
        )
        .await?;
    }
    Ok(())
}

async fn refresh_outlook_categories(
    pool: &SqlitePool,
    account: &AccountFeatureRow,
) -> Result<(), String> {
    let token = access_token_from_account_config(OAuthProvider::Outlook, &account.config).await?;
    let response = satchel_core::utils::http::client()
        .get("https://graph.microsoft.com/v1.0/me/outlook/masterCategories")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| format!("outlook categories request failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "outlook categories request failed with status {status}: {body}"
        ));
    }
    let payload: OutlookListResponse<OutlookCategory> = response
        .json()
        .await
        .map_err(|err| format!("outlook categories parse failed: {err}"))?;
    for category in payload.value {
        let remote_id = category.id.as_deref().unwrap_or(&category.display_name);
        upsert_provider_label(
            pool,
            &account.id,
            "outlook",
            remote_id,
            &category.display_name,
            category.color.as_deref(),
            Some("category"),
            true,
        )
        .await?;
    }
    Ok(())
}

async fn refresh_imap_labels_from_folders(
    pool: &SqlitePool,
    account: &AccountFeatureRow,
) -> Result<(), String> {
    let rows = sqlx::query_as::<_, (Option<String>, String, String)>(
        "SELECT remote_id, name, folder_type FROM folders WHERE account_id = ? ORDER BY name ASC",
    )
    .bind(&account.id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;
    for (remote_id, name, kind) in rows {
        upsert_provider_label(
            pool,
            &account.id,
            &account.provider,
            remote_id.as_deref().unwrap_or(&name),
            &name,
            None,
            Some(&kind),
            true,
        )
        .await?;
    }
    Ok(())
}

async fn create_gmail_label(
    pool: &SqlitePool,
    account: &AccountFeatureRow,
    name: &str,
    color: Option<&str>,
) -> Result<(), String> {
    let token = access_token_from_account_config(OAuthProvider::Gmail, &account.config).await?;
    let mut body = serde_json::json!({
        "name": name,
        "labelListVisibility": "labelShow",
        "messageListVisibility": "show"
    });
    if let Some(color) = color.filter(|value| !value.trim().is_empty()) {
        body["color"] = serde_json::json!({"backgroundColor": color, "textColor": "#ffffff"});
    }
    let response = satchel_core::utils::http::client()
        .post("https://gmail.googleapis.com/gmail/v1/users/me/labels")
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|err| format!("gmail label create failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "gmail label create failed with status {status}: {body}"
        ));
    }
    let created: CreatedLabelResponse = response
        .json()
        .await
        .map_err(|err| format!("gmail label create parse failed: {err}"))?;
    let remote_id = created.id.unwrap_or_else(|| name.to_string());
    let label_name = created.name.unwrap_or_else(|| name.to_string());
    upsert_provider_label(
        pool,
        &account.id,
        "gmail",
        &remote_id,
        &label_name,
        color,
        Some("user"),
        true,
    )
    .await
}

async fn create_outlook_category(
    pool: &SqlitePool,
    account: &AccountFeatureRow,
    name: &str,
    color: Option<&str>,
) -> Result<(), String> {
    let token = access_token_from_account_config(OAuthProvider::Outlook, &account.config).await?;
    let body = serde_json::json!({
        "displayName": name,
        "color": color.unwrap_or("preset0")
    });
    let response = satchel_core::utils::http::client()
        .post("https://graph.microsoft.com/v1.0/me/outlook/masterCategories")
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|err| format!("outlook category create failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "outlook category create failed with status {status}: {body}"
        ));
    }
    let created: CreatedLabelResponse = response
        .json()
        .await
        .map_err(|err| format!("outlook category create parse failed: {err}"))?;
    let remote_id = created.id.unwrap_or_else(|| name.to_string());
    let label_name = created.display_name.unwrap_or_else(|| name.to_string());
    upsert_provider_label(
        pool,
        &account.id,
        "outlook",
        &remote_id,
        &label_name,
        color,
        Some("category"),
        true,
    )
    .await
}

async fn upsert_provider_label(
    pool: &SqlitePool,
    account_id: &str,
    provider: &str,
    remote_id: &str,
    name: &str,
    color: Option<&str>,
    kind: Option<&str>,
    visible: bool,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let id = satchel_core::sync::provider_identity::provider_local_id(
        "pl",
        provider,
        &[account_id, remote_id],
    );
    sqlx::query(
        "INSERT INTO provider_labels (id, account_id, provider, remote_id, name, color, kind, visible, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, remote_id) DO UPDATE SET
            name = excluded.name,
            color = excluded.color,
            kind = excluded.kind,
            visible = excluded.visible,
            updated_at = excluded.updated_at",
    )
    .bind(id)
    .bind(account_id)
    .bind(provider)
    .bind(remote_id)
    .bind(name)
    .bind(color)
    .bind(kind)
    .bind(if visible { 1 } else { 0 })
    .bind(now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::upsert_provider_label;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn provider_label_ids_preserve_legacy_rows_and_distinguish_remote_punctuation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES ('account', 'Person', 'person@example.test', 'gmail', '{}', '', '', '')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO provider_labels (id, account_id, provider, remote_id, name, visible, updated_at) VALUES ('pl-legacy', 'account', 'gmail', 'label.1', 'Old name', 1, '')").execute(&pool).await.unwrap();
        for (remote, name) in [("label.1", "Existing label"), ("label-1", "Another label")] {
            upsert_provider_label(&pool, "account", "gmail", remote, name, None, None, true)
                .await
                .unwrap();
        }
        let labels: Vec<(String, String, String)> =
            sqlx::query_as("SELECT id, remote_id, name FROM provider_labels ORDER BY remote_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(labels.len(), 2);
        assert_ne!(labels[0].0, labels[1].0);
        assert_eq!(
            labels[1],
            (
                "pl-legacy".to_string(),
                "label.1".to_string(),
                "Existing label".to_string()
            )
        );
    }
}
