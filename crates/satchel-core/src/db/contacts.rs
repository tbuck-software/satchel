//! Tauri-free persistence and policy for local contacts.
//!
//! The app owns contact providers, image decoding and platform APIs.  This
//! module owns the durable contact record, normalization and avatar policy so
//! every caller observes the same conflict and ordering rules.

use std::fmt::Write as _;

use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

const CONTACT_LIST_LIMIT: i64 = 500;
const AVATAR_TARGET_LIMIT: i64 = 500;
const FALLBACK_AVATAR_TARGET_LIMIT: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ContactRecord {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_source: Option<String>,
    pub avatar_locked: i64,
    pub avatar_updated_at: Option<String>,
    pub frequency: i64,
    pub last_contacted: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ContactAvatarTarget {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContactUpsertRequest {
    pub id: Option<String>,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_source: Option<String>,
    pub avatar_locked: Option<bool>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContactAvatarOverrideRequest {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub locked: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarSource {
    Manual,
    LocalContact,
    ProviderContact,
    Bimi,
    Gravatar,
    Favicon,
}

impl AvatarSource {
    const ALL: [Self; 6] = [
        Self::Manual,
        Self::LocalContact,
        Self::ProviderContact,
        Self::Bimi,
        Self::Gravatar,
        Self::Favicon,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::LocalContact => "local_contact",
            Self::ProviderContact => "provider_contact",
            Self::Bimi => "bimi",
            Self::Gravatar => "gravatar",
            Self::Favicon => "favicon",
        }
    }

    pub const fn rank(self) -> i64 {
        match self {
            Self::Manual => 100,
            Self::LocalContact => 90,
            Self::ProviderContact => 80,
            Self::Bimi => 45,
            Self::Gravatar => 35,
            Self::Favicon => 20,
        }
    }
}

fn avatar_rank_sql(column: &str) -> String {
    let mut sql = format!("CASE COALESCE({column}, '')");
    for source in AvatarSource::ALL {
        write!(sql, " WHEN '{}' THEN {}", source.as_str(), source.rank())
            .expect("writing to a String cannot fail");
    }
    sql.push_str(" ELSE 0 END");
    sql
}

/// Alias retained for callers that want to make the contact-specific role
/// explicit at their seam.
pub type ContactAvatarSource = AvatarSource;

#[derive(Debug, Error)]
pub enum ContactError {
    #[error("contact email is invalid")]
    InvalidEmail,
    #[error("contact persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for ContactError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

const CONTACT_SELECT: &str = "SELECT id, email, display_name, avatar_url, avatar_source, avatar_locked, avatar_updated_at, frequency, last_contacted, notes, created_at, updated_at FROM contacts";

/// Owns local contact records and all contact-specific persistence policy.
pub struct ContactRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ContactRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<ContactRecord>, ContactError> {
        sqlx::query_as::<_, ContactRecord>(&format!(
            "{CONTACT_SELECT} ORDER BY frequency DESC, COALESCE(last_contacted, created_at) DESC, email ASC LIMIT {CONTACT_LIST_LIMIT}"
        ))
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn search(
        &self,
        query: &str,
        limit: Option<i64>,
    ) -> Result<Vec<ContactRecord>, ContactError> {
        let term = query.trim();
        if term.is_empty() {
            return Ok(Vec::new());
        }
        let limit = limit.unwrap_or(12).clamp(1, 50);
        let pattern = format!("%{term}%");
        sqlx::query_as::<_, ContactRecord>(&format!(
            "{CONTACT_SELECT} WHERE email LIKE ? OR COALESCE(display_name, '') LIKE ? ORDER BY frequency DESC, COALESCE(last_contacted, created_at) DESC, email ASC LIMIT ?"
        ))
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    /// Insert or merge a contact.  An existing email owns its original id;
    /// incoming optional fields never erase existing values, and every merge
    /// counts as a contact/touch just like the historic command did.
    pub async fn upsert(&self, input: ContactUpsertRequest) -> Result<ContactRecord, ContactError> {
        let email = normalize_email(&input.email).ok_or(ContactError::InvalidEmail)?;
        let now = chrono::Utc::now().to_rfc3339();
        let id = input
            .id
            .unwrap_or_else(|| format!("contact-{}", uuid::Uuid::new_v4()));
        let display_name = clean_optional(input.display_name);
        let avatar_url = clean_optional(input.avatar_url);
        let avatar_url_present = avatar_url.is_some();
        let avatar_source = input
            .avatar_source
            .unwrap_or_else(|| AvatarSource::Manual.as_str().to_owned());
        let avatar_locked = input.avatar_locked.unwrap_or(avatar_url_present);
        let avatar_updated_at = avatar_url_present.then_some(now.clone());
        let notes = clean_optional(input.notes);

        sqlx::query(
            "INSERT INTO contacts (id, email, display_name, avatar_url, avatar_source, avatar_locked, avatar_updated_at,
                                  frequency, last_contacted, notes, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                display_name = COALESCE(excluded.display_name, contacts.display_name),
                avatar_url = COALESCE(excluded.avatar_url, contacts.avatar_url),
                avatar_source = CASE
                    WHEN excluded.avatar_url IS NOT NULL THEN excluded.avatar_source
                    ELSE contacts.avatar_source
                END,
                avatar_locked = CASE
                    WHEN excluded.avatar_url IS NOT NULL THEN excluded.avatar_locked
                    ELSE contacts.avatar_locked
                END,
                avatar_updated_at = CASE
                    WHEN excluded.avatar_url IS NOT NULL THEN excluded.avatar_updated_at
                    ELSE contacts.avatar_updated_at
                END,
                notes = COALESCE(excluded.notes, contacts.notes),
                frequency = contacts.frequency + 1,
                last_contacted = excluded.last_contacted,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&email)
        .bind(display_name.as_deref())
        .bind(avatar_url.as_deref())
        .bind(&avatar_source)
        .bind(if avatar_locked { 1_i64 } else { 0_i64 })
        .bind(avatar_updated_at.as_deref())
        .bind(&now)
        .bind(notes.as_deref())
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;

        // Keep the historical separate read: it gives callers the row after
        // the conflict merge rather than the excluded INSERT values.
        self.get_by_email(&email).await
    }

    /// Apply the user-controlled avatar override.  Clearing an avatar writes
    /// NULL source and URL but still advances `avatar_updated_at`; unlike a
    /// profile upsert this operation does not increment contact frequency.
    pub async fn set_avatar(
        &self,
        input: ContactAvatarOverrideRequest,
    ) -> Result<ContactRecord, ContactError> {
        let email = normalize_email(&input.email).ok_or(ContactError::InvalidEmail)?;
        let avatar_url = clean_optional(input.avatar_url);
        let locked = input.locked.unwrap_or(true);
        let display_name = clean_optional(input.display_name);
        let now = chrono::Utc::now().to_rfc3339();
        let id = format!("contact-{}", uuid::Uuid::new_v4());

        sqlx::query(
            "INSERT INTO contacts (id, email, display_name, avatar_url, avatar_source, avatar_locked, avatar_updated_at,
                                  frequency, last_contacted, notes, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                display_name = COALESCE(excluded.display_name, contacts.display_name),
                avatar_url = excluded.avatar_url,
                avatar_source = excluded.avatar_source,
                avatar_locked = excluded.avatar_locked,
                avatar_updated_at = excluded.avatar_updated_at,
                updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(&email)
        .bind(display_name.as_deref())
        .bind(avatar_url.as_deref())
        .bind(avatar_url.is_some().then_some(AvatarSource::Manual.as_str()))
        .bind(if locked { 1_i64 } else { 0_i64 })
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;

        self.get_by_email(&email).await
    }

    pub async fn get_by_email(&self, email: &str) -> Result<ContactRecord, ContactError> {
        sqlx::query_as::<_, ContactRecord>(&format!("{CONTACT_SELECT} WHERE email = ?"))
            .bind(email)
            .fetch_one(self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn find_by_email(&self, email: &str) -> Result<Option<ContactRecord>, ContactError> {
        sqlx::query_as::<_, ContactRecord>(&format!("{CONTACT_SELECT} WHERE email = ?"))
            .bind(email)
            .fetch_optional(self.pool)
            .await
            .map_err(Into::into)
    }

    pub async fn delete(&self, id: &str) -> Result<(), ContactError> {
        sqlx::query("DELETE FROM contacts WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Best-effort caller operation: malformed addresses deliberately do
    /// nothing, while a database failure remains observable to the caller for
    /// structured diagnostics.
    pub async fn touch(&self, email: &str, display_name: Option<&str>) -> Result<(), ContactError> {
        let Some(email) = normalize_email(email) else {
            return Ok(());
        };
        let now = chrono::Utc::now().to_rfc3339();
        let id = format!("contact-{}", uuid::Uuid::new_v4());
        let display_name = display_name
            .map(|value| value.to_owned())
            .and_then(|value| clean_optional(Some(value)));

        sqlx::query(
            "INSERT INTO contacts (id, email, display_name, avatar_url, frequency, last_contacted, notes, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 1, ?, NULL, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                display_name = COALESCE(excluded.display_name, contacts.display_name),
                frequency = contacts.frequency + 1,
                last_contacted = excluded.last_contacted,
                updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(email)
        .bind(display_name.as_deref())
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Apply a provider/local/fallback avatar candidate according to source
    /// priority. Locked manual (or otherwise locked) avatars always win;
    /// `force` is checked after the lock and can replace unlocked sources.
    pub async fn upsert_avatar(
        &self,
        email: &str,
        display_name: Option<&str>,
        avatar_url: &str,
        source: AvatarSource,
        force: bool,
    ) -> Result<(), ContactError> {
        let Some(email) = normalize_email(email) else {
            return Ok(());
        };
        let now = chrono::Utc::now().to_rfc3339();
        let id = format!("contact-{}", uuid::Uuid::new_v4());
        let display_name = display_name
            .map(|value| value.to_owned())
            .and_then(|value| clean_optional(Some(value)));
        let stored_avatar_rank = avatar_rank_sql("contacts.avatar_source");
        let query = format!(
            "INSERT INTO contacts (id, email, display_name, avatar_url, avatar_source, avatar_locked, avatar_updated_at,
                                  frequency, last_contacted, notes, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, 1, ?, NULL, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                display_name = COALESCE(excluded.display_name, contacts.display_name),
                avatar_url = CASE
                    WHEN contacts.avatar_locked = 1 THEN contacts.avatar_url
                    WHEN ? = 1 THEN excluded.avatar_url
                    WHEN contacts.avatar_url IS NULL OR contacts.avatar_url = '' THEN excluded.avatar_url
                    WHEN ? >= {stored_avatar_rank} THEN excluded.avatar_url
                    ELSE contacts.avatar_url
                END,
                avatar_source = CASE
                    WHEN contacts.avatar_locked = 1 THEN contacts.avatar_source
                    WHEN ? = 1 THEN excluded.avatar_source
                    WHEN contacts.avatar_url IS NULL OR contacts.avatar_url = '' THEN excluded.avatar_source
                    WHEN ? >= {stored_avatar_rank} THEN excluded.avatar_source
                    ELSE contacts.avatar_source
                END,
                avatar_updated_at = CASE
                    WHEN contacts.avatar_locked = 1 THEN contacts.avatar_updated_at
                    WHEN ? = 1 THEN excluded.avatar_updated_at
                    WHEN contacts.avatar_url IS NULL OR contacts.avatar_url = '' THEN excluded.avatar_updated_at
                    WHEN ? >= {stored_avatar_rank} THEN excluded.avatar_updated_at
                    ELSE contacts.avatar_updated_at
                END,
                last_contacted = COALESCE(contacts.last_contacted, excluded.last_contacted),
                updated_at = excluded.updated_at"
        );

        sqlx::query(&query)
            .bind(id)
            .bind(email)
            .bind(display_name.as_deref())
            .bind(avatar_url)
            .bind(source.as_str())
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .bind(&now)
            .bind(if force { 1_i64 } else { 0_i64 })
            .bind(source.rank())
            .bind(if force { 1_i64 } else { 0_i64 })
            .bind(source.rank())
            .bind(if force { 1_i64 } else { 0_i64 })
            .bind(source.rank())
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn avatar_targets(
        &self,
        limit: i64,
    ) -> Result<Vec<ContactAvatarTarget>, ContactError> {
        let limit = limit.clamp(0, AVATAR_TARGET_LIMIT);
        sqlx::query_as::<_, ContactAvatarTarget>(
            "SELECT email, display_name FROM contacts
             WHERE email IS NOT NULL AND email != ''
               AND (avatar_url IS NULL OR avatar_url = ''
                    OR avatar_source IN ('provider_contact', 'bimi', 'gravatar', 'favicon'))
               AND avatar_locked = 0
             ORDER BY frequency DESC, COALESCE(last_contacted, created_at) DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn fallback_avatar_targets(
        &self,
        limit: i64,
    ) -> Result<Vec<ContactAvatarTarget>, ContactError> {
        let limit = limit.clamp(0, FALLBACK_AVATAR_TARGET_LIMIT);
        sqlx::query_as::<_, ContactAvatarTarget>(
            "SELECT email, display_name FROM contacts
             WHERE email IS NOT NULL AND email != ''
               AND avatar_locked = 0
               AND (avatar_url IS NULL OR avatar_url = '' OR avatar_source IN ('favicon', 'gravatar'))
             ORDER BY frequency DESC, COALESCE(last_contacted, created_at) DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    /// CardDAV's source-specific conflict policy: a stable ID derived from
    /// the normalized address, incoming display names replace old names, and
    /// a locally entered note always wins over a remote note.
    pub async fn upsert_carddav(
        &self,
        email: &str,
        display_name: Option<&str>,
        note: Option<&str>,
    ) -> Result<bool, ContactError> {
        let Some(email) = normalize_carddav_email(email) else {
            return Ok(false);
        };
        let now = chrono::Utc::now().to_rfc3339();
        let id = stable_id("contact-carddav", &email);
        sqlx::query(
            "INSERT INTO contacts
                (id, email, display_name, avatar_url, avatar_source, avatar_locked, avatar_updated_at,
                 frequency, last_contacted, notes, created_at, updated_at)
             VALUES (?, ?, ?, NULL, NULL, 0, NULL, 1, NULL, ?, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                display_name = COALESCE(excluded.display_name, contacts.display_name),
                notes = COALESCE(contacts.notes, excluded.notes),
                updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(&email)
        .bind(display_name)
        .bind(note)
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;
        Ok(true)
    }
}

/// Combine repeated addresses and write all contact touches under the ingest lock.
pub async fn touch_contacts_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    contacts: &[(&str, Option<&str>)],
) -> Result<(), ContactError> {
    let mut touches = std::collections::BTreeMap::<String, (Option<String>, i64)>::new();
    for (email, name) in contacts {
        let Some(email) = normalize_email(email) else {
            continue;
        };
        let entry = touches.entry(email).or_default();
        entry.1 += 1;
        if let Some(name) = name.and_then(|value| clean_optional(Some(value.to_owned()))) {
            entry.0 = Some(name);
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let touches = touches.into_iter().collect::<Vec<_>>();
    for chunk in touches.chunks(100) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO contacts (id, email, display_name, frequency, last_contacted, created_at, updated_at) "
        );
        query.push_values(chunk, |mut row, (email, (name, frequency))| {
            row.push_bind(format!("contact-{}", uuid::Uuid::new_v4()))
                .push_bind(email)
                .push_bind(name)
                .push_bind(frequency)
                .push_bind(&now)
                .push_bind(&now)
                .push_bind(&now);
        });
        query.push(" ON CONFLICT(email) DO UPDATE SET display_name = COALESCE(excluded.display_name, contacts.display_name), frequency = contacts.frequency + excluded.frequency, last_contacted = excluded.last_contacted, updated_at = excluded.updated_at");
        query.build().execute(&mut **tx).await?;
    }
    Ok(())
}

pub fn normalize_email(value: &str) -> Option<String> {
    let cleaned = value.trim().trim_matches(['<', '>']).to_ascii_lowercase();
    if cleaned.contains('@') {
        Some(cleaned)
    } else {
        None
    }
}

fn normalize_carddav_email(value: &str) -> Option<String> {
    let cleaned = value.trim().to_ascii_lowercase();
    cleaned.contains('@').then_some(cleaned)
}

pub fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn stable_id(prefix: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    let suffix = digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    fn input(email: &str) -> ContactUpsertRequest {
        ContactUpsertRequest {
            id: None,
            email: email.to_owned(),
            display_name: None,
            avatar_url: None,
            avatar_source: None,
            avatar_locked: None,
            notes: None,
        }
    }

    async fn insert_contact(
        pool: &SqlitePool,
        id: &str,
        email: &str,
        frequency: i64,
        created_at: &str,
        avatar_url: Option<&str>,
        avatar_source: Option<&str>,
        avatar_locked: i64,
    ) {
        sqlx::query(
            "INSERT INTO contacts
                (id, email, avatar_url, avatar_source, avatar_locked, frequency, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(email)
        .bind(avatar_url)
        .bind(avatar_source)
        .bind(avatar_locked)
        .bind(frequency)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert contact");
    }

    #[tokio::test]
    async fn normalizes_and_merges_contacts_while_preserving_created_id() {
        let pool = pool().await;
        let first = ContactRepository::new(&pool)
            .upsert(ContactUpsertRequest {
                id: Some("first".into()),
                email: " <Person@EXAMPLE.test> ".into(),
                display_name: Some(" First ".into()),
                ..input("ignored")
            })
            .await
            .expect("create");
        let second = ContactRepository::new(&pool)
            .upsert(ContactUpsertRequest {
                id: Some("second".into()),
                email: "person@example.test".into(),
                display_name: None,
                notes: Some("note".into()),
                ..input("ignored")
            })
            .await
            .expect("merge");
        assert_eq!(first.id, "first");
        assert_eq!(second.id, "first");
        assert_eq!(second.frequency, 2);
        assert_eq!(second.display_name.as_deref(), Some("First"));
        assert_eq!(second.notes.as_deref(), Some("note"));
    }

    #[tokio::test]
    async fn search_empty_and_limits_are_bounded() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        assert!(repo.search("  ", None).await.expect("empty").is_empty());
        for index in 0..60 {
            repo.touch(&format!("{index}@example.test"), None)
                .await
                .expect("touch");
        }
        assert_eq!(
            repo.search("@example", Some(100)).await.expect("max").len(),
            50
        );
        assert_eq!(
            repo.search("@example", Some(-4)).await.expect("min").len(),
            1
        );
    }

    #[tokio::test]
    async fn list_has_stable_order_and_a_hard_limit_of_500() {
        let pool = pool().await;
        for index in 0..501 {
            insert_contact(
                &pool,
                &format!("contact-{index}"),
                &format!("{index:03}@example.test"),
                if index == 500 { 100 } else { 0 },
                &format!("2024-01-01T00:{index:03}:00Z"),
                None,
                None,
                0,
            )
            .await;
        }
        let contacts = ContactRepository::new(&pool).list().await.expect("list");
        assert_eq!(contacts.len(), 500);
        assert_eq!(
            contacts.first().map(|contact| contact.email.as_str()),
            Some("500@example.test")
        );
        assert!(!contacts
            .iter()
            .any(|contact| contact.email == "000@example.test"));
    }

    #[tokio::test]
    async fn avatar_targets_filter_locked_and_existing_sources_and_cap_limits() {
        let pool = pool().await;
        for index in 0..550 {
            insert_contact(
                &pool,
                &format!("target-{index}"),
                &format!("target-{index}@example.test"),
                550 - index,
                &format!("2024-01-01T00:{index:03}:00Z"),
                None,
                None,
                0,
            )
            .await;
        }
        sqlx::query(
            "UPDATE contacts SET avatar_url = 'local', avatar_source = 'local_contact'
             WHERE email = 'target-0@example.test'",
        )
        .execute(&pool)
        .await
        .expect("local avatar");
        sqlx::query("UPDATE contacts SET avatar_locked = 1 WHERE email = 'target-1@example.test'")
            .execute(&pool)
            .await
            .expect("locked avatar");
        sqlx::query("UPDATE contacts SET avatar_url = 'small-icon', avatar_source = 'favicon' WHERE email = 'target-2@example.test'")
            .execute(&pool).await.expect("fallback avatar");

        let repo = ContactRepository::new(&pool);
        let targets = repo.avatar_targets(999).await.expect("targets");
        assert_eq!(targets.len(), 500);
        assert!(targets
            .iter()
            .all(|target| target.email != "target-0@example.test"));
        assert!(targets
            .iter()
            .all(|target| target.email != "target-1@example.test"));
        assert_eq!(
            targets.first().map(|target| target.email.as_str()),
            Some("target-2@example.test")
        );

        let fallback = repo.fallback_avatar_targets(999).await.expect("fallback");
        assert_eq!(fallback.len(), 300);
        assert!(fallback
            .iter()
            .all(|target| target.email != "target-0@example.test"));
        assert!(fallback
            .iter()
            .all(|target| target.email != "target-1@example.test"));
    }

    #[tokio::test]
    async fn avatar_override_clear_and_lock_do_not_touch_frequency() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        repo.touch("person@example.test", None)
            .await
            .expect("touch");
        let before = repo.get_by_email("person@example.test").await.expect("row");
        assert_eq!(before.frequency, 1);
        let set = repo
            .set_avatar(ContactAvatarOverrideRequest {
                email: "person@example.test".into(),
                display_name: None,
                avatar_url: Some("data:image/png;base64,AA==".into()),
                locked: Some(true),
            })
            .await
            .expect("avatar");
        assert_eq!(set.frequency, 1);
        assert_eq!(set.avatar_source.as_deref(), Some("manual"));
        assert_eq!(set.avatar_locked, 1);
        let cleared = repo
            .set_avatar(ContactAvatarOverrideRequest {
                email: "person@example.test".into(),
                display_name: None,
                avatar_url: None,
                locked: Some(false),
            })
            .await
            .expect("clear");
        assert_eq!(cleared.avatar_url, None);
        assert_eq!(cleared.avatar_source, None);
        assert_eq!(cleared.avatar_locked, 0);
        assert!(cleared.avatar_updated_at.is_some());
    }

    #[tokio::test]
    async fn profile_avatar_upsert_sets_manual_source_and_lock() {
        let pool = pool().await;
        let contact = ContactRepository::new(&pool)
            .upsert(ContactUpsertRequest {
                email: "person@example.test".into(),
                avatar_url: Some("profile-avatar".into()),
                ..input("ignored")
            })
            .await
            .expect("profile");
        assert_eq!(contact.avatar_url.as_deref(), Some("profile-avatar"));
        assert_eq!(contact.avatar_source.as_deref(), Some("manual"));
        assert_eq!(contact.avatar_locked, 1);
        assert!(contact.last_contacted.is_some());
    }

    #[tokio::test]
    async fn avatar_priority_and_lock_are_enforced() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        repo.upsert_avatar(
            "person@example.test",
            None,
            "favicon",
            AvatarSource::Favicon,
            false,
        )
        .await
        .expect("favicon");
        repo.upsert_avatar(
            "person@example.test",
            None,
            "provider",
            AvatarSource::ProviderContact,
            false,
        )
        .await
        .expect("provider");
        let provider = repo.get_by_email("person@example.test").await.expect("row");
        assert_eq!(provider.avatar_url.as_deref(), Some("provider"));
        repo.set_avatar(ContactAvatarOverrideRequest {
            email: "person@example.test".into(),
            display_name: None,
            avatar_url: Some("manual".into()),
            locked: Some(true),
        })
        .await
        .expect("manual");
        repo.upsert_avatar(
            "person@example.test",
            None,
            "forced",
            AvatarSource::Favicon,
            true,
        )
        .await
        .expect("forced");
        let locked = repo.get_by_email("person@example.test").await.expect("row");
        assert_eq!(locked.avatar_url.as_deref(), Some("manual"));
    }

    #[tokio::test]
    async fn avatar_priority_allows_equal_replacement_and_force_only_when_unlocked() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        repo.upsert_avatar(
            "person@example.test",
            None,
            "provider",
            AvatarSource::ProviderContact,
            false,
        )
        .await
        .expect("provider");
        repo.upsert_avatar(
            "person@example.test",
            None,
            "low",
            AvatarSource::Gravatar,
            false,
        )
        .await
        .expect("low");
        assert_eq!(
            repo.get_by_email("person@example.test")
                .await
                .expect("row")
                .avatar_url
                .as_deref(),
            Some("provider")
        );
        repo.upsert_avatar(
            "person@example.test",
            None,
            "equal",
            AvatarSource::ProviderContact,
            false,
        )
        .await
        .expect("equal");
        assert_eq!(
            repo.get_by_email("person@example.test")
                .await
                .expect("row")
                .avatar_url
                .as_deref(),
            Some("equal")
        );
        repo.upsert_avatar(
            "person@example.test",
            None,
            "forced",
            AvatarSource::Favicon,
            true,
        )
        .await
        .expect("force");
        assert_eq!(
            repo.get_by_email("person@example.test")
                .await
                .expect("row")
                .avatar_url
                .as_deref(),
            Some("forced")
        );
        repo.set_avatar(ContactAvatarOverrideRequest {
            email: "person@example.test".into(),
            display_name: None,
            avatar_url: Some("locked".into()),
            locked: Some(true),
        })
        .await
        .expect("lock");
        repo.upsert_avatar(
            "person@example.test",
            None,
            "forced-again",
            AvatarSource::Favicon,
            true,
        )
        .await
        .expect("force locked");
        assert_eq!(
            repo.get_by_email("person@example.test")
                .await
                .expect("row")
                .avatar_url
                .as_deref(),
            Some("locked")
        );
    }

    #[tokio::test]
    async fn invalid_profile_is_typed_and_invalid_touch_is_a_no_op() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        let error = repo
            .upsert(input("not-an-email"))
            .await
            .expect_err("invalid email");
        assert!(matches!(error, ContactError::InvalidEmail));
        repo.touch("not-an-email", Some("Ignored"))
            .await
            .expect("no-op");
        assert!(repo.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn delete_is_idempotent_and_removes_existing_contacts() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        repo.delete("missing").await.expect("missing delete");
        repo.touch("person@example.test", None)
            .await
            .expect("touch");
        let id = repo
            .get_by_email("person@example.test")
            .await
            .expect("contact")
            .id;
        repo.delete(&id).await.expect("delete");
        assert!(repo
            .find_by_email("person@example.test")
            .await
            .expect("find")
            .is_none());
    }

    #[tokio::test]
    async fn carddav_keeps_local_notes_and_uses_stable_id() {
        let pool = pool().await;
        let repo = ContactRepository::new(&pool);
        repo.upsert(ContactUpsertRequest {
            id: Some("local".into()),
            email: "person@example.test".into(),
            display_name: Some("Local".into()),
            notes: Some("local note".into()),
            avatar_url: Some("local-avatar".into()),
            ..input("ignored")
        })
        .await
        .expect("local");
        let before = repo
            .get_by_email("person@example.test")
            .await
            .expect("before");
        repo.upsert_carddav("PERSON@EXAMPLE.TEST", Some(" Remote "), Some("remote note"))
            .await
            .expect("carddav");
        let row = repo.get_by_email("person@example.test").await.expect("row");
        assert_eq!(row.id, "local");
        assert_eq!(row.display_name.as_deref(), Some(" Remote "));
        assert_eq!(row.notes.as_deref(), Some("local note"));
        assert_eq!(row.avatar_url.as_deref(), Some("local-avatar"));
        assert_eq!(row.frequency, before.frequency);
        assert_eq!(row.last_contacted, before.last_contacted);

        repo.upsert_carddav("new@example.test", Some(" New "), Some(" note "))
            .await
            .expect("new carddav");
        let new = repo
            .get_by_email("new@example.test")
            .await
            .expect("new row");
        assert_eq!(new.id, stable_id("contact-carddav", "new@example.test"));
        assert_eq!(new.display_name.as_deref(), Some(" New "));
        assert_eq!(new.notes.as_deref(), Some(" note "));
    }

    #[tokio::test]
    async fn missing_schema_is_a_typed_persistence_error() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        let error = ContactRepository::new(&pool)
            .list()
            .await
            .expect_err("missing schema");
        assert!(matches!(error, ContactError::Persistence(_)));
    }

    #[tokio::test]
    async fn concurrent_file_backed_touches_keep_all_frequency_increments() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("contacts.db");
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .expect("file db");
        crate::db::init(&pool).await.expect("migrations");
        let pool = Arc::new(pool);
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let pool = Arc::clone(&pool);
            tasks.push(tokio::spawn(async move {
                ContactRepository::new(&pool)
                    .touch("parallel@example.test", None)
                    .await
                    .expect("touch");
            }));
        }
        for task in tasks {
            task.await.expect("touch task");
        }
        let contact = ContactRepository::new(&pool)
            .get_by_email("parallel@example.test")
            .await
            .expect("contact");
        assert_eq!(contact.frequency, 20);
    }
}
