use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::providers::traits::{MailProvider, RemoteIdTranslation};

/// Translate legacy Outlook message ids before the first provider sync.
pub(crate) async fn migrate_outlook_remote_ids(
    pool: &SqlitePool,
    account_id: &str,
    provider: &dyn MailProvider,
) -> Result<(), String> {
    let source_ids = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT source_id FROM (
           SELECT remote_id AS source_id FROM messages
           WHERE account_id = ? AND NULLIF(TRIM(remote_id), '') IS NOT NULL
           UNION
           SELECT remote_draft_id AS source_id FROM messages
           WHERE account_id = ? AND NULLIF(TRIM(remote_draft_id), '') IS NOT NULL
         )",
    )
    .bind(account_id)
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let translations = if source_ids.is_empty() {
        Vec::new()
    } else {
        provider
            .translate_message_ids(&source_ids)
            .await
            .map_err(|error| error.to_string())?
    };
    let returned = translations
        .iter()
        .map(|translation| translation.source_id.as_str())
        .collect::<HashSet<_>>();
    if let Some(missing) = source_ids
        .iter()
        .find(|source| !returned.contains(source.as_str()))
    {
        return Err(format!(
            "outlook id translation omitted stored message id {missing}"
        ));
    }

    apply_outlook_id_translations(pool, account_id, &translations).await
}

/// Apply translated Outlook ids atomically to messages and attachments.
///
pub async fn apply_outlook_id_translations(
    pool: &SqlitePool,
    account_id: &str,
    translations: &[RemoteIdTranslation],
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;

    for translation in translations {
        let target_id = translation.target_id.trim();
        if target_id.is_empty() {
            return Err(format!(
                "outlook id translation returned an empty target for {}",
                translation.source_id
            ));
        }
        sqlx::query(
            "UPDATE attachments
             SET remote_uid = ?
             WHERE remote_uid = ?
               AND message_id IN (
                 SELECT id FROM messages
                 WHERE account_id = ?
                   AND (remote_id = ? OR remote_draft_id = ?)
               )",
        )
        .bind(target_id)
        .bind(&translation.source_id)
        .bind(account_id)
        .bind(&translation.source_id)
        .bind(&translation.source_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
        sqlx::query(
            "UPDATE messages
             SET remote_id = ?,
                 remote_draft_id = CASE
                   WHEN remote_draft_id = ? THEN ? ELSE remote_draft_id
                 END,
                 updated_at = ?
             WHERE account_id = ?
               AND (remote_id = ? OR remote_draft_id = ?)",
        )
        .bind(target_id)
        .bind(&translation.source_id)
        .bind(target_id)
        .bind(&now)
        .bind(account_id)
        .bind(&translation.source_id)
        .bind(&translation.source_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;
    }

    sqlx::query(
        "UPDATE accounts
         SET outlook_immutable_ids_migrated = 1, updated_at = ?
         WHERE id = ?",
    )
    .bind(&now)
    .bind(account_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}
