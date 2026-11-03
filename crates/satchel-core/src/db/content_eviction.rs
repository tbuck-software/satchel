//! The one rule for releasing cached message content the provider can serve again.
//!
//! Both the age window (`body_window`) and the mobile storage budget
//! (`storage`) select candidates through [`RECOVERABLE`] and clear them through
//! [`release`]. Header rows and metadata always stay.

use sqlx::SqliteConnection;

/// `FROM` clause binding `m`, `a` and `f`. The inner join drops rows whose
/// folder is missing or belongs to another account.
pub(crate) const FROM: &str = "messages m JOIN accounts a ON a.id = m.account_id
     JOIN folders f ON f.id = m.folder_id AND f.account_id = m.account_id";

/// Content the provider still holds under the identity saved on the row.
/// IMAP UIDs are scoped to folder and generation, so the saved generation must
/// match the folder's current one; anything else may be the only copy.
pub(crate) const RECOVERABLE: &str = "a.kind = 'email' AND m.is_draft = 0
     AND m.pending_folder_id IS NULL
     AND NULLIF(m.remote_id, '') IS NOT NULL AND m.remote_id NOT LIKE 'import-%'
     AND (a.provider IN ('gmail', 'outlook')
          OR (a.provider = 'imap' AND m.remote_uidvalidity > 0
              AND m.remote_uidvalidity = f.uidvalidity
              AND NULLIF(COALESCE(f.remote_id, f.full_path), '') IS NOT NULL))";

/// Rows that still hold something [`release`] would clear.
pub(crate) const HAS_PAYLOAD: &str = "(m.body_text IS NOT NULL OR m.body_html IS NOT NULL
     OR m.original_body_html IS NOT NULL OR m.original_source IS NOT NULL
     OR m.smime_encrypted IS NOT NULL OR m.smime_signed IS NOT NULL
     OR m.pgp_signed IS NOT NULL
     OR EXISTS (SELECT 1 FROM attachments att WHERE att.message_id = m.id
                AND (att.content_base64 IS NOT NULL OR att.cached_path IS NOT NULL)))";

/// Clear every recoverable copy of the given messages on the caller's
/// transaction. The predicate is re-applied, so a row that changed identity
/// since selection is left alone. Released `cached_path` files are removed by
/// the maintenance orphan sweep.
pub(crate) async fn release(
    connection: &mut SqliteConnection,
    ids: &[String],
) -> Result<u64, sqlx::Error> {
    if ids.is_empty() {
        return Ok(0);
    }
    let ids =
        serde_json::to_string(ids).map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let eligible = format!(
        "SELECT m.id FROM {FROM} WHERE {RECOVERABLE} AND m.id IN (SELECT value FROM json_each(?))"
    );
    // Attachments first: the message update below must not change eligibility.
    sqlx::query(&format!(
        "UPDATE attachments SET cached_path = NULL, content_base64 = NULL
         WHERE (cached_path IS NOT NULL OR content_base64 IS NOT NULL)
           AND message_id IN ({eligible})"
    ))
    .bind(&ids)
    .execute(&mut *connection)
    .await?;
    let released = sqlx::query(&format!(
        "UPDATE messages SET body_text = NULL, body_html = NULL,
             original_body_html = NULL, original_source = NULL,
             smime_encrypted = NULL, smime_signed = NULL, pgp_signed = NULL, updated_at = ?
         WHERE id IN ({eligible})"
    ))
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&ids)
    .execute(&mut *connection)
    .await?;
    Ok(released.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{body_window, data_repairs, storage};
    use sqlx::SqlitePool;

    /// Accounts `imap`, `gmail`, `pop3` and `other-imap`; folder `<provider>`
    /// per account at IMAP generation 7.
    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        for (id, provider) in [
            ("imap", "imap"),
            ("gmail", "gmail"),
            ("pop3", "pop3"),
            ("other-imap", "imap"),
        ] {
            sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at) VALUES (?, '', 'a@example.test', ?, '{}', '', '', '')")
                .bind(id).bind(provider).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO folders (id, account_id, name, full_path, remote_id, folder_type, uidvalidity, created_at, updated_at) VALUES (?, ?, 'INBOX', 'INBOX', 'INBOX', 'inbox', 7, '', '')")
                .bind(id).bind(id).execute(&pool).await.unwrap();
        }
        // (id, account, folder, remote id, saved generation)
        for (id, account, folder, remote, generation) in [
            ("imap-verified", "imap", "imap", "1", Some(7)),
            ("gmail-remote", "gmail", "gmail", "g1", None),
            ("imap-stale", "imap", "imap", "2", Some(6)),
            ("imap-missing", "imap", "imap", "3", None),
            ("imap-invalid", "imap", "imap", "4", Some(0)),
            ("cross-account", "imap", "other-imap", "5", Some(7)),
            ("pending-move", "imap", "imap", "6", Some(7)),
            ("draft", "imap", "imap", "7", Some(7)),
            ("imported", "imap", "imap", "import-1", Some(7)),
            ("local", "imap", "imap", "", Some(7)),
            ("pop3", "pop3", "pop3", "8", None),
        ] {
            sqlx::query("INSERT INTO messages (id, account_id, folder_id, remote_id, remote_uidvalidity, date, from_address, to_addresses, subject, snippet, body_text, body_html, original_body_html, original_source, smime_encrypted, smime_signed, pgp_signed, created_at, updated_at) VALUES (?, ?, ?, ?, ?, '2010-01-01', 'from@example.test', '[]', 'Subject', 'Preview', 'Text', '<p>Text</p>', '<p onclick=x()>Text</p>', zeroblob(64), zeroblob(64), zeroblob(64), zeroblob(64), '', '')")
                .bind(id).bind(account).bind(folder).bind(remote).bind(generation)
                .execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO attachments (id, message_id, filename, content_type, size, cached_path, content_base64, created_at) VALUES (?, ?, 'file', 'text/plain', 3, '/cache/file', 'YWJj', '')")
                .bind(format!("att-{id}")).bind(id).execute(&pool).await.unwrap();
        }
        sqlx::query("UPDATE messages SET is_draft = 1 WHERE id = 'draft'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE messages SET pending_folder_id = 'imap' WHERE id = 'pending-move'")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    /// Ids whose content (message columns and attachment bytes) is fully released.
    async fn released(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT m.id FROM messages m WHERE m.body_text IS NULL AND m.body_html IS NULL
               AND m.original_body_html IS NULL AND m.original_source IS NULL
               AND m.smime_encrypted IS NULL AND m.smime_signed IS NULL AND m.pgp_signed IS NULL
               AND NOT EXISTS (SELECT 1 FROM attachments att WHERE att.message_id = m.id
                   AND (att.cached_path IS NOT NULL OR att.content_base64 IS NOT NULL))
             ORDER BY m.id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Ids that kept every copy.
    async fn intact(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT m.id FROM messages m JOIN attachments att ON att.message_id = m.id
             WHERE m.body_text IS NOT NULL AND m.original_source IS NOT NULL
               AND m.smime_encrypted IS NOT NULL AND m.pgp_signed IS NOT NULL
               AND att.content_base64 IS NOT NULL AND att.cached_path IS NOT NULL
             ORDER BY m.id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    const KEPT: [&str; 9] = [
        "cross-account",
        "draft",
        "imap-invalid",
        "imap-missing",
        "imap-stale",
        "imported",
        "local",
        "pending-move",
        "pop3",
    ];

    async fn assert_rule(pool: &SqlitePool) {
        assert_eq!(released(pool).await, ["gmail-remote", "imap-verified"]);
        assert_eq!(intact(pool).await, KEPT);
        let (headers, attachments): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM messages WHERE subject = 'Subject' AND snippet = 'Preview'),
                    (SELECT COUNT(*) FROM attachments)",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            (headers, attachments),
            (11, 11),
            "headers and attachment rows stay"
        );
    }

    #[tokio::test]
    async fn age_window_and_storage_budget_release_the_same_messages() {
        let window = pool().await;
        body_window::set_days(&window, 30, 0).await.unwrap();
        data_repairs::run_repair(&window, &body_window::BODY_WINDOW, 3, None, |_| {})
            .await
            .unwrap();
        assert_rule(&window).await;

        let budget = pool().await;
        while storage::evict_to_target(&budget, 0, 1).await.unwrap() > 0 {}
        assert_rule(&budget).await;
    }

    #[tokio::test]
    async fn a_generation_change_after_selection_keeps_the_content() {
        let pool = pool().await;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("UPDATE folders SET uidvalidity = 8 WHERE id = 'imap'")
            .execute(&mut *tx)
            .await
            .unwrap();
        let ids = ["imap-verified".to_string(), "gmail-remote".to_string()];
        assert_eq!(release(&mut tx, &ids).await.unwrap(), 1);
        tx.commit().await.unwrap();
        assert_eq!(released(&pool).await, ["gmail-remote"]);
        assert!(intact(&pool).await.contains(&"imap-verified".to_string()));
    }

    #[tokio::test]
    async fn an_aborted_pass_releases_nothing() {
        let pool = pool().await;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        assert_eq!(
            release(&mut tx, &["imap-verified".to_string()])
                .await
                .unwrap(),
            1
        );
        tx.rollback().await.unwrap();
        assert!(released(&pool).await.is_empty());
        assert_eq!(intact(&pool).await.len(), 11);
    }
}
