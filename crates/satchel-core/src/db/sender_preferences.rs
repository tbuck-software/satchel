//! Tauri-free sender categorization policy and preference persistence.

use sqlx::SqlitePool;
use thiserror::Error;

use crate::db::threads::{ThreadError, ThreadRepository};

pub const VALID_CATEGORIES: [&str; 4] = ["personal", "updates", "promotions", "social"];

/// The category field needs three states at the persistence seam. A missing
/// field leaves the effective preference untouched, a value sets it, and an
/// explicit null removes the preference from the account row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SenderPreferenceCategoryUpdate {
    #[default]
    Unchanged,
    Set(String),
    Clear,
}

#[derive(Debug, Clone)]
pub struct SenderPreferenceUpdate {
    pub account_id: String,
    pub sender_email: String,
    pub category: SenderPreferenceCategoryUpdate,
    pub is_priority: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderPreferenceRecord {
    pub sender_email: String,
    pub category: Option<String>,
    pub is_priority: bool,
}

#[derive(Debug, Error)]
pub enum SenderPreferenceError {
    #[error("sender preference account id must not be empty")]
    AccountIdEmpty,
    #[error("sender preference email is invalid")]
    SenderEmailInvalid,
    #[error("sender preference category is invalid")]
    CategoryInvalid,
    #[error("sender preference persistence failed")]
    Persistence(#[source] sqlx::Error),
    #[error("sender preference projection failed")]
    Projection(#[source] ThreadError),
}

impl From<sqlx::Error> for SenderPreferenceError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

impl From<ThreadError> for SenderPreferenceError {
    fn from(error: ThreadError) -> Self {
        Self::Projection(error)
    }
}

#[derive(Debug, sqlx::FromRow)]
struct EffectivePreferenceRow {
    category: Option<String>,
    is_priority: i64,
}

/// Owns the global effective sender preference and its existing-thread
/// projection. Account rows remain separate, but reads and projections span
/// all email accounts by design.
pub struct SenderPreferenceRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SenderPreferenceRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Return the effective preference across all account rows.
    pub async fn get(
        &self,
        sender_email: &str,
    ) -> Result<SenderPreferenceRecord, SenderPreferenceError> {
        Self::get_with_executor(self.pool, sender_email).await
    }

    pub async fn get_with_executor<'e, E>(
        executor: E,
        sender_email: &str,
    ) -> Result<SenderPreferenceRecord, SenderPreferenceError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let sender_email = normalize_sender_email_for_lookup(sender_email);
        if sender_email.is_empty() {
            return Err(SenderPreferenceError::SenderEmailInvalid);
        }
        if !is_valid_sender_email(&sender_email) {
            return Ok(SenderPreferenceRecord {
                sender_email,
                category: None,
                is_priority: false,
            });
        }
        let effective = effective_preference(executor, &sender_email).await?;
        Ok(SenderPreferenceRecord {
            sender_email,
            category: effective.category,
            is_priority: effective.is_priority > 0,
        })
    }

    /// Persist one account row and project the resulting effective preference
    /// atomically to existing threads. The projection is deliberately in the
    /// same `BEGIN IMMEDIATE` transaction as the upsert, so a failed thread
    /// update cannot leave the sender rule ahead of its local mail.
    pub async fn set(&self, update: SenderPreferenceUpdate) -> Result<(), SenderPreferenceError> {
        let account_id = update.account_id.trim();
        if account_id.is_empty() {
            return Err(SenderPreferenceError::AccountIdEmpty);
        }
        let sender_email = normalize_sender_email_for_set(&update.sender_email)?;
        let category = normalize_category_update(update.category)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let account_preference = sqlx::query_as::<_, EffectivePreferenceRow>(
            "SELECT category, is_priority
             FROM sender_preferences
             WHERE account_id = ? AND sender_email = ?",
        )
        .bind(account_id)
        .bind(&sender_email)
        .fetch_optional(&mut *transaction)
        .await?;
        let project_category = !matches!(category, SenderPreferenceCategoryUpdate::Unchanged);
        let next_category = match &category {
            SenderPreferenceCategoryUpdate::Unchanged => account_preference
                .as_ref()
                .and_then(|row| row.category.clone()),
            SenderPreferenceCategoryUpdate::Set(category) => Some(category.clone()),
            SenderPreferenceCategoryUpdate::Clear => None,
        };
        let next_priority = update
            .is_priority
            .map(i64::from)
            .unwrap_or_else(|| account_preference.as_ref().map_or(0, |row| row.is_priority));

        sqlx::query(
            "INSERT INTO sender_preferences
             (account_id, sender_email, category, is_priority, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, sender_email) DO UPDATE SET
               category = excluded.category,
               is_priority = excluded.is_priority,
               updated_at = excluded.updated_at",
        )
        .bind(account_id)
        .bind(&sender_email)
        .bind(next_category.as_deref())
        .bind(next_priority)
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;

        let effective_after = effective_preference(&mut *transaction, &sender_email).await?;
        let project_priority = update.is_priority;
        ThreadRepository::new(self.pool)
            .project_sender_preference_in_transaction(
                &mut transaction,
                &sender_email,
                project_category.then_some(effective_after.category.as_deref()),
                project_priority.map(|_| effective_after.is_priority > 0),
            )
            .await?;

        transaction.commit().await?;
        Ok(())
    }
}

fn normalize_category_update(
    update: SenderPreferenceCategoryUpdate,
) -> Result<SenderPreferenceCategoryUpdate, SenderPreferenceError> {
    match update {
        SenderPreferenceCategoryUpdate::Unchanged => Ok(update),
        SenderPreferenceCategoryUpdate::Clear => Ok(update),
        SenderPreferenceCategoryUpdate::Set(value) => {
            let value = value.trim();
            if value.is_empty() {
                return Ok(SenderPreferenceCategoryUpdate::Clear);
            }
            if !VALID_CATEGORIES.contains(&value) {
                return Err(SenderPreferenceError::CategoryInvalid);
            }
            Ok(SenderPreferenceCategoryUpdate::Set(value.to_owned()))
        }
    }
}

pub fn normalize_category(category: Option<&str>) -> Result<Option<String>, SenderPreferenceError> {
    let category = category.map(str::trim).filter(|value| !value.is_empty());
    category
        .map(|value| {
            VALID_CATEGORIES
                .contains(&value)
                .then(|| value.to_owned())
                .ok_or(SenderPreferenceError::CategoryInvalid)
        })
        .transpose()
}

pub fn is_valid_category(category: Option<&str>) -> bool {
    normalize_category(category).is_ok()
}

fn normalize_sender_email_for_lookup(sender_email: &str) -> String {
    sender_email.trim().to_ascii_lowercase()
}

fn is_valid_sender_email(sender_email: &str) -> bool {
    sender_email.len() <= 320 && sender_email.contains('@')
}

fn normalize_sender_email_for_set(sender_email: &str) -> Result<String, SenderPreferenceError> {
    let sender_email = normalize_sender_email_for_lookup(sender_email);
    if !is_valid_sender_email(&sender_email) {
        return Err(SenderPreferenceError::SenderEmailInvalid);
    }
    Ok(sender_email)
}

async fn effective_preference<'e, E>(
    executor: E,
    sender_email: &str,
) -> Result<EffectivePreferenceRow, SenderPreferenceError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query_as::<_, EffectivePreferenceRow>(
        "SELECT
           (SELECT category
            FROM sender_preferences
            WHERE sender_email = ? AND category IS NOT NULL
            ORDER BY updated_at DESC, account_id ASC
            LIMIT 1) AS category,
           COALESCE((SELECT MAX(is_priority)
                     FROM sender_preferences
                     WHERE sender_email = ?), 0) AS is_priority",
    )
    .bind(sender_email)
    .bind(sender_email)
    .fetch_one(executor)
    .await
    .map_err(Into::into)
}

/// Gmail's explicit category is stronger than address and list-header
/// heuristics. `CATEGORY_FORUMS` joins notifications in Satchel's focused list.
pub fn provider_category(labels: &[String]) -> Option<&'static str> {
    labels
        .iter()
        .find_map(|label| match label.trim().to_ascii_uppercase().as_str() {
            "CATEGORY_UPDATES" | "CATEGORY_FORUMS" => Some("updates"),
            "CATEGORY_PROMOTIONS" => Some("promotions"),
            "CATEGORY_SOCIAL" => Some("social"),
            "CATEGORY_PERSONAL" => Some("personal"),
            _ => None,
        })
}

/// Build the small header context used by the automatic classifier. Keep this
/// shared so ingest and reclassification use the same newsletter signals.
pub fn classification_headers(
    list_unsubscribe: Option<&str>,
    list_unsubscribe_post: Option<&str>,
    folder_remote_id: Option<&str>,
) -> String {
    format!(
        "{}{}{}",
        list_unsubscribe
            .map(|value| format!("List-Unsubscribe: {value}\n"))
            .unwrap_or_default(),
        list_unsubscribe_post
            .map(|value| format!("List-Unsubscribe-Post: {value}\n"))
            .unwrap_or_default(),
        folder_remote_id.unwrap_or_default(),
    )
}

/// Heuristic categorization used when ingesting messages.
pub fn classify_message(
    raw_headers: Option<&str>,
    from_address: &str,
    subject: Option<&str>,
) -> &'static str {
    let from_address_lc = from_address.to_ascii_lowercase();
    let from_domain = from_address_lc
        .split_once('@')
        .map(|(_, d)| d.to_string())
        .unwrap_or_default();
    let subject_lc = subject.unwrap_or("").to_ascii_lowercase();
    let headers_lc = raw_headers.unwrap_or("").to_ascii_lowercase();

    let social_domains = [
        "facebook",
        "instagram",
        "twitter",
        "linkedin",
        "tiktok",
        "youtube",
        "reddit",
        "discord",
        "snapchat",
        "pinterest",
        "meetup",
    ];
    if social_domains.iter().any(|s| from_domain.contains(s))
        || from_domain == "x.com"
        || from_domain.ends_with(".x.com")
    {
        return "social";
    }

    let transactional_sender = [
        "gitlab@",
        "notifications@",
        "notification@",
        "alerts@",
        "alert@",
        "shipment@",
        "shipment-tracking@",
        "tracking@",
        "orders@",
        "order@",
        "billing@",
        "invoice@",
        "receipts@",
        "security@",
    ]
    .iter()
    .any(|prefix| from_address_lc.starts_with(prefix));
    let transactional_subject = [
        "shipped",
        "out for delivery",
        "delivered",
        "delivery update",
        "order confirmation",
        "payment received",
        "invoice",
        "receipt",
        "security alert",
        "verification code",
        "versandt",
        "zustellung",
        "geliefert",
        "bestellbestätigung",
        "zahlungsbestätigung",
        "rechnung",
        "sicherheitswarnung",
        "bestätigungscode",
    ]
    .iter()
    .any(|needle| subject_lc.contains(needle));
    if transactional_sender || transactional_subject {
        return "updates";
    }

    if headers_lc.contains("list-unsubscribe")
        || headers_lc.contains("precedence: bulk")
        || headers_lc.contains("category_promotions")
        || headers_lc.contains("promotions")
        || subject_lc.contains("sale")
        || subject_lc.contains("deal")
        || subject_lc.contains("offer")
        || subject_lc.contains("% off")
        || subject_lc.contains("rabatt")
        || subject_lc.contains("angebot")
        || subject_lc.contains("newsletter")
        || from_address_lc.starts_with("newsletter@")
        || from_address_lc.starts_with("promo@")
    {
        return "promotions";
    }

    if from_address_lc.starts_with("no-reply")
        || from_address_lc.starts_with("noreply")
        || from_address_lc.starts_with("notifications@")
        || from_address_lc.starts_with("notification@")
        || from_address_lc.starts_with("alerts@")
        || from_address_lc.starts_with("alert@")
        || from_address_lc.starts_with("info@")
        || from_address_lc.starts_with("billing@")
        || from_address_lc.starts_with("invoice@")
        || from_address_lc.starts_with("receipts@")
        || from_address_lc.starts_with("hello@")
        || headers_lc.contains("auto-submitted: auto-generated")
        || headers_lc.contains("category_updates")
        || headers_lc.contains("updates")
    {
        return "updates";
    }

    "personal"
}

/// Apply provider labels first and then the shared heuristic policy. The
/// policy intentionally uses subject only; snippets are display previews, not
/// classification input.
pub fn classify_message_with_provider_labels(
    provider_labels: &[String],
    raw_headers: Option<&str>,
    from_address: &str,
    subject: Option<&str>,
) -> &'static str {
    if let Some(category) = provider_category(provider_labels) {
        return category;
    }
    classify_message(raw_headers, from_address, subject)
}

#[cfg(test)]
mod tests {
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    async fn account(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref,
              is_active, created_at, updated_at)
             VALUES (?, 'Account', ?, 'imap', '{}', '', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(format!("{id}@example.test"))
        .execute(pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES (?, ?, 'Inbox', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
        )
        .bind(format!("folder-{id}"))
        .bind(id)
        .execute(pool)
        .await
        .expect("folder");
    }

    async fn thread(
        pool: &SqlitePool,
        account_id: &str,
        thread_id: &str,
        message_id: &str,
        sender: &str,
        date: &str,
        labels: &str,
        category: Option<&str>,
        source: &str,
    ) {
        sqlx::query(
            "INSERT INTO threads
             (id, account_id, subject, last_message_date, message_count, unread_count,
              priority, priority_source, category, category_source, created_at, updated_at)
             VALUES (?, ?, 'Subject', ?, 1, 1, 1, 'heuristic', ?, ?,
                     '2026-01-01', '2026-01-01')",
        )
        .bind(thread_id)
        .bind(account_id)
        .bind(date)
        .bind(category)
        .bind(source)
        .execute(pool)
        .await
        .expect("thread");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses,
              subject, date, provider_labels, category, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, '[]', 'Subject', ?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(message_id)
        .bind(account_id)
        .bind(format!("folder-{account_id}"))
        .bind(thread_id)
        .bind(sender)
        .bind(date)
        .bind(labels)
        .bind(category)
        .execute(pool)
        .await
        .expect("message");
    }

    #[test]
    fn self_hosted_code_notifications_are_updates_even_with_unsubscribe_links() {
        for headers in [
            None,
            Some("List-Unsubscribe: <https://code.example/unsubscribe>"),
        ] {
            assert_eq!(
                classify_message(
                    headers,
                    "gitlab@code.example",
                    Some("Project | Failed pipeline | d7ade544")
                ),
                "updates"
            );
        }
    }

    #[test]
    fn provider_categories_are_explicit_and_forums_are_notifications() {
        assert_eq!(
            provider_category(&["CATEGORY_UPDATES".into()]),
            Some("updates")
        );
        assert_eq!(
            provider_category(&["CATEGORY_FORUMS".into()]),
            Some("updates")
        );
        assert_eq!(provider_category(&["INBOX".into()]), None);
    }

    #[test]
    fn heuristic_classification_preserves_existing_policy() {
        assert_eq!(
            classify_message(None, "notifications@github.com", Some("Build failed")),
            "updates"
        );
        assert_eq!(
            classify_message(None, "marketing@stepke-kitas.de", Some("Formulare")),
            "personal"
        );
        assert_eq!(
            classify_message(
                Some("List-Unsubscribe: <https://example.com/unsubscribe>"),
                "shipment-tracking@amazon.de",
                Some("Unterwegs zur Zustellung"),
            ),
            "updates"
        );
        assert_eq!(
            classify_message_with_provider_labels(
                &[],
                None,
                "someone@example.test",
                Some("Status"),
            ),
            "personal"
        );
    }

    #[tokio::test]
    async fn resolves_global_category_deterministically_and_projects_cross_account() {
        let pool = pool().await;
        account(&pool, "account-a").await;
        account(&pool, "account-b").await;
        thread(
            &pool,
            "account-a",
            "thread-a",
            "message-a",
            "Sender@Example.test",
            "2026-01-02",
            "[]",
            Some("personal"),
            "heuristic",
        )
        .await;
        thread(
            &pool,
            "account-b",
            "thread-b",
            "message-b",
            "sender@example.test",
            "2026-01-02",
            "[]",
            Some("updates"),
            "heuristic",
        )
        .await;

        let repository = SenderPreferenceRepository::new(&pool);
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-a".into(),
                sender_email: " Sender@Example.test ".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: Some(true),
            })
            .await
            .expect("preference");

        assert_eq!(
            repository.get("SENDER@example.test").await.unwrap(),
            SenderPreferenceRecord {
                sender_email: "sender@example.test".into(),
                category: Some("promotions".into()),
                is_priority: true,
            }
        );
        let projections: Vec<(String, String, i64, String)> = sqlx::query_as(
            "SELECT category, category_source, priority, priority_source
             FROM threads ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("projections");
        assert_eq!(
            projections,
            vec![
                (
                    "promotions".into(),
                    "manual_sender".into(),
                    2,
                    "manual_sender".into()
                ),
                (
                    "promotions".into(),
                    "manual_sender".into(),
                    2,
                    "manual_sender".into()
                ),
            ]
        );

        let timestamps: (String, String) = sqlx::query_as(
            "SELECT created_at, updated_at
             FROM sender_preferences
             WHERE account_id = 'account-a' AND sender_email = 'sender@example.test'",
        )
        .fetch_one(&pool)
        .await
        .expect("preference timestamps");
        assert_eq!(timestamps.0, timestamps.1);
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-a".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: None,
            })
            .await
            .expect("preference update");
        let updated_timestamps: (String, String) = sqlx::query_as(
            "SELECT created_at, updated_at
             FROM sender_preferences
             WHERE account_id = 'account-a' AND sender_email = 'sender@example.test'",
        )
        .fetch_one(&pool)
        .await
        .expect("updated preference timestamps");
        assert_eq!(updated_timestamps.0, timestamps.0);
        assert_ne!(updated_timestamps.1, timestamps.1);

        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-b".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Unchanged,
                is_priority: Some(true),
            })
            .await
            .expect("account-specific priority change");
        assert!(
            repository
                .get("sender@example.test")
                .await
                .unwrap()
                .is_priority
        );
    }

    #[tokio::test]
    async fn account_patches_preserve_target_fields_and_clear_without_a_ghost() {
        let pool = pool().await;
        account(&pool, "account-a").await;
        account(&pool, "account-b").await;
        thread(
            &pool,
            "account-b",
            "thread-b",
            "message-b",
            "sender@example.test",
            "2026-01-02",
            "[\"CATEGORY_UPDATES\"]",
            Some("updates"),
            "heuristic",
        )
        .await;
        let repository = SenderPreferenceRepository::new(&pool);
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-a".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: Some(true),
            })
            .await
            .expect("account A preference");

        // A priority-only patch in B must not materialize A's effective
        // category into B's account row.
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-b".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Unchanged,
                is_priority: Some(true),
            })
            .await
            .expect("account B priority patch");
        let account_b: (Option<String>, i64) = sqlx::query_as(
            "SELECT category, is_priority
             FROM sender_preferences
             WHERE account_id = 'account-b' AND sender_email = 'sender@example.test'",
        )
        .fetch_one(&pool)
        .await
        .expect("account B row");
        assert_eq!(account_b, (None, 1));
        assert_eq!(
            repository
                .get("sender@example.test")
                .await
                .unwrap()
                .category,
            Some("promotions".into())
        );

        // A category-only patch in B must preserve its existing priority too.
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-b".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("social".into()),
                is_priority: None,
            })
            .await
            .expect("account B category patch");
        let account_b: (Option<String>, i64) = sqlx::query_as(
            "SELECT category, is_priority
             FROM sender_preferences
             WHERE account_id = 'account-b' AND sender_email = 'sender@example.test'",
        )
        .fetch_one(&pool)
        .await
        .expect("account B category row");
        assert_eq!(account_b, (Some("social".into()), 1));

        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-b".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Clear,
                is_priority: None,
            })
            .await
            .expect("clear account B category");

        // Clearing A must remove the effective category; B's clear must not
        // leave behind a copied category that keeps it alive.
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account-a".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Clear,
                is_priority: None,
            })
            .await
            .expect("clear account A category");
        assert_eq!(
            repository
                .get("sender@example.test")
                .await
                .unwrap()
                .category,
            None
        );
        let projection: (Option<String>, String) =
            sqlx::query_as("SELECT category, category_source FROM threads WHERE id = 'thread-b'")
                .fetch_one(&pool)
                .await
                .expect("cleared projection");
        assert_eq!(projection, (Some("updates".into()), "provider".into()));
    }

    #[tokio::test]
    async fn clear_restores_latest_message_category_and_provider_priority_fallback() {
        let pool = pool().await;
        account(&pool, "account").await;
        thread(
            &pool,
            "account",
            "thread",
            "message",
            "sender@example.test",
            "2026-01-02",
            "[\"CATEGORY_UPDATES\",\"IMPORTANT\"]",
            Some("updates"),
            "heuristic",
        )
        .await;
        let repository = SenderPreferenceRepository::new(&pool);
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: Some(true),
            })
            .await
            .expect("set preference");
        // Ingest may have stored the effective sender category on the newest
        // message as well; clearing the preference must still reconstruct the
        // provider category instead of treating that value as the base.
        sqlx::query("UPDATE messages SET category = 'promotions' WHERE id = 'message'")
            .execute(&pool)
            .await
            .expect("sender category on message");
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Clear,
                is_priority: Some(false),
            })
            .await
            .expect("clear preference");

        assert_eq!(
            repository.get("sender@example.test").await.unwrap(),
            SenderPreferenceRecord {
                sender_email: "sender@example.test".into(),
                category: None,
                is_priority: false,
            }
        );
        let projection: (Option<String>, String, i64, String) = sqlx::query_as(
            "SELECT category, category_source, priority, priority_source
             FROM threads WHERE id = 'thread'",
        )
        .fetch_one(&pool)
        .await
        .expect("projection");
        assert_eq!(
            projection,
            (
                Some("updates".into()),
                "provider".into(),
                2,
                "provider".into()
            )
        );
    }

    #[tokio::test]
    async fn clear_reclassifies_multiple_threads_from_latest_message_data() {
        let pool = pool().await;
        account(&pool, "account").await;
        thread(
            &pool,
            "account",
            "thread-personal",
            "message-personal",
            "sender@example.test",
            "2026-01-01",
            "[]",
            Some("promotions"),
            "heuristic",
        )
        .await;
        sqlx::query(
            "UPDATE messages
             SET subject = 'Status', snippet = 'Sale invoice offer'
             WHERE id = 'message-personal'",
        )
        .execute(&pool)
        .await
        .expect("snippet-only message");
        thread(
            &pool,
            "account",
            "thread-latest",
            "message-latest-old",
            "sender@example.test",
            "2026-01-01",
            "[]",
            Some("personal"),
            "heuristic",
        )
        .await;
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses,
              subject, snippet, date, provider_labels, category, created_at, updated_at)
             VALUES ('message-latest-new', 'account', 'folder-account', 'thread-latest',
                     'sender@example.test', '[]', 'Invoice status', 'Your invoice is ready',
                     '2026-02-01', '[]', 'promotions', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("latest message");

        let repository = SenderPreferenceRepository::new(&pool);
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: None,
            })
            .await
            .expect("set preference");
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Clear,
                is_priority: None,
            })
            .await
            .expect("clear preference");

        let projections: Vec<(String, String)> = sqlx::query_as(
            "SELECT category, category_source
             FROM threads ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("projections");
        assert_eq!(
            projections,
            vec![
                ("updates".into(), "heuristic".into()),
                ("personal".into(), "heuristic".into()),
            ]
        );
    }

    #[tokio::test]
    async fn projection_uses_non_draft_date_and_id_order_for_sender_selection() {
        let pool = pool().await;
        account(&pool, "account").await;
        thread(
            &pool,
            "account",
            "thread",
            "message-base",
            "other@example.test",
            "2026-01-01",
            "[]",
            Some("personal"),
            "heuristic",
        )
        .await;
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses,
              subject, date, is_draft, provider_labels, category, created_at, updated_at)
             VALUES ('message-draft', 'account', 'folder-account', 'thread',
                     'sender@example.test', '[]', 'Draft', '2026-12-01', 1,
                     '[]', 'personal', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("draft message");
        let repository = SenderPreferenceRepository::new(&pool);
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: None,
            })
            .await
            .expect("preference");
        let category: (Option<String>, String) =
            sqlx::query_as("SELECT category, category_source FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("draft projection");
        assert_eq!(category, (Some("personal".into()), "heuristic".into()));

        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses,
              subject, date, is_draft, provider_labels, category, created_at, updated_at)
             VALUES ('message-a', 'account', 'folder-account', 'thread',
                     'other@example.test', '[]', 'Old tie', '2027-01-01', 0,
                     '[]', 'personal', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("first tie message");
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, thread_id, from_address, to_addresses,
              subject, date, is_draft, provider_labels, category, created_at, updated_at)
             VALUES ('message-z', 'account', 'folder-account', 'thread',
                     'sender@example.test', '[]', 'New tie', '2027-01-01', 0,
                     '[]', 'personal', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("second tie message");
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: None,
            })
            .await
            .expect("preference after tie");
        let category: (Option<String>, String) =
            sqlx::query_as("SELECT category, category_source FROM threads WHERE id = 'thread'")
                .fetch_one(&pool)
                .await
                .expect("tie projection");
        assert_eq!(
            category,
            (Some("promotions".into()), "manual_sender".into())
        );
    }

    #[tokio::test]
    async fn unchanged_category_only_updates_priority_and_manual_fields_are_protected() {
        let pool = pool().await;
        account(&pool, "account").await;
        thread(
            &pool,
            "account",
            "thread",
            "message",
            "sender@example.test",
            "2026-01-02",
            "[]",
            Some("personal"),
            "manual_thread",
        )
        .await;
        let repository = SenderPreferenceRepository::new(&pool);
        sqlx::query(
            "CREATE TRIGGER reject_unchanged_category_projection
             BEFORE UPDATE OF category ON threads
             BEGIN SELECT RAISE(ABORT, 'category must remain untouched'); END",
        )
        .execute(&pool)
        .await
        .expect("category guard");
        repository
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Unchanged,
                is_priority: Some(true),
            })
            .await
            .expect("preference");
        let projection: (Option<String>, String, i64, String) = sqlx::query_as(
            "SELECT category, category_source, priority, priority_source
             FROM threads WHERE id = 'thread'",
        )
        .fetch_one(&pool)
        .await
        .expect("projection");
        assert_eq!(
            projection,
            (
                Some("personal".into()),
                "manual_thread".into(),
                2,
                "manual_sender".into()
            )
        );
    }

    #[tokio::test]
    async fn failed_projection_rolls_back_preference_upsert() {
        let pool = pool().await;
        account(&pool, "account").await;
        thread(
            &pool,
            "account",
            "thread",
            "message",
            "sender@example.test",
            "2026-01-02",
            "[]",
            Some("personal"),
            "heuristic",
        )
        .await;
        sqlx::query(
            "CREATE TRIGGER fail_sender_projection BEFORE UPDATE OF category ON threads
             BEGIN SELECT RAISE(ABORT, 'sender projection failed'); END",
        )
        .execute(&pool)
        .await
        .expect("trigger");
        let result = SenderPreferenceRepository::new(&pool)
            .set(SenderPreferenceUpdate {
                account_id: "account".into(),
                sender_email: "sender@example.test".into(),
                category: SenderPreferenceCategoryUpdate::Set("promotions".into()),
                is_priority: None,
            })
            .await;
        assert!(matches!(result, Err(SenderPreferenceError::Projection(_))));
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sender_preferences WHERE sender_email = 'sender@example.test'",
        )
        .fetch_one(&pool)
        .await
        .expect("preference count");
        assert_eq!(count, 0);
    }

    #[test]
    fn validates_category_and_sender_email_with_current_normalization() {
        assert!(matches!(
            normalize_category(Some(" unknown ")),
            Err(SenderPreferenceError::CategoryInvalid)
        ));
        assert!(matches!(
            normalize_sender_email_for_set(" "),
            Err(SenderPreferenceError::SenderEmailInvalid)
        ));
        assert_eq!(normalize_category(Some("  ")).unwrap(), None);
    }

    #[tokio::test]
    async fn lookup_tolerates_malformed_sender_but_set_stays_strict() {
        let pool = pool().await;
        let repository = SenderPreferenceRepository::new(&pool);
        assert_eq!(
            repository.get(" Broken Sender ").await.unwrap(),
            SenderPreferenceRecord {
                sender_email: "broken sender".into(),
                category: None,
                is_priority: false,
            }
        );
        assert!(matches!(
            repository
                .set(SenderPreferenceUpdate {
                    account_id: "account".into(),
                    sender_email: " Broken Sender ".into(),
                    category: SenderPreferenceCategoryUpdate::Set("personal".into()),
                    is_priority: None,
                })
                .await,
            Err(SenderPreferenceError::SenderEmailInvalid)
        ));
    }
}
