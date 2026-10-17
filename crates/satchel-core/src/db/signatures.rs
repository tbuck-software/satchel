//! Tauri-free persistence for mail signatures, assignments, assets, and suggestions.

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use thiserror::Error;

const SIGNATURE_COLUMNS: &str = "id, account_id, name, body_text, body_html, short_body_text,
     short_body_html, is_default, created_at, updated_at";
const SIGNATURE_SELECT: &str = "SELECT id, account_id, name, body_text, body_html,
    short_body_text, short_body_html, is_default, created_at, updated_at FROM signatures";
const ASSIGNMENT_COLUMNS: &str =
    "account_id, identity_email, signature_id, first_contact_mode, created_at, updated_at";
const ASSIGNMENT_SELECT: &str = "SELECT account_id, identity_email, signature_id,
    first_contact_mode, created_at, updated_at FROM signature_assignments";
const SUGGESTION_SELECT: &str = "SELECT id, account_id, identity_email, body_text,
    body_html, evidence_count, status, created_at, updated_at FROM signature_suggestions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignaturePreference {
    Auto,
    Long,
    Short,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureVariant {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstContactMode {
    AlwaysFull,
    FullThenShort,
    AlwaysShort,
}

impl FirstContactMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlwaysFull => "always_full",
            Self::FullThenShort => "full_then_short",
            Self::AlwaysShort => "always_short",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SignatureError> {
        match value {
            "always_full" => Ok(Self::AlwaysFull),
            "full_then_short" => Ok(Self::FullThenShort),
            "always_short" => Ok(Self::AlwaysShort),
            _ => Err(SignatureError::InvalidFirstContactMode),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureSuggestionStatus {
    Pending,
    Accepted,
    Dismissed,
}

impl SignatureSuggestionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, SignatureError> {
        match value {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "dismissed" => Ok(Self::Dismissed),
            _ => Err(SignatureError::InvalidSuggestionStatus),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureAssetWrite {
    pub filename: String,
    pub mime_type: String,
    pub content_id: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SignatureAssetRecord {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub content_id: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct StoredSignature {
    id: String,
    account_id: String,
    name: String,
    body_text: String,
    body_html: Option<String>,
    short_body_text: String,
    short_body_html: Option<String>,
    is_default: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureRecord {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub short_body_text: String,
    pub short_body_html: Option<String>,
    pub assets: Vec<SignatureAssetRecord>,
    pub is_default: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SignatureUpsertRequest {
    pub id: Option<String>,
    pub account_id: String,
    pub name: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub short_body_text: String,
    pub short_body_html: Option<String>,
    pub assets: Vec<SignatureAssetWrite>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SignatureAssignmentRecord {
    pub account_id: String,
    pub identity_email: String,
    pub signature_id: Option<String>,
    pub first_contact_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SignatureAssignmentUpsertRequest {
    pub account_id: String,
    pub identity_email: String,
    pub signature_id: Option<String>,
    pub first_contact_mode: FirstContactMode,
}

#[derive(Debug, Clone)]
pub struct SignatureResolveRequest {
    pub account_id: String,
    pub identity_email: String,
    pub signature_id: Option<String>,
    pub preference: SignaturePreference,
    pub sent_from_identity_in_thread: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSignatureRecord {
    pub signature_id: String,
    pub variant: SignatureVariant,
    pub body_text: String,
    pub body_html: Option<String>,
    /// Raw bytes. Encoding and provider attachment mapping belong to the app.
    pub assets: Vec<SignatureAssetRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SignatureSuggestionRecord {
    pub id: String,
    pub account_id: String,
    pub identity_email: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub evidence_count: i64,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SignatureSuggestionUpsertRequest {
    pub body_text: String,
    pub body_html: Option<String>,
    pub evidence_count: i64,
}

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error("signature name is empty")]
    NameEmpty,
    #[error("signature was not found for account")]
    NotFoundForAccount,
    #[error("signature belongs to another account")]
    OwnedByAnotherAccount,
    #[error("invalid first-contact signature mode")]
    InvalidFirstContactMode,
    #[error("invalid signature suggestion status")]
    InvalidSuggestionStatus,
    #[error("signature persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for SignatureError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns the complete durable signature aggregate. Encoding, sanitizing, identity
/// authorization, provider attachment mapping, and compose orchestration remain
/// at the app seam.
pub struct SignatureRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SignatureRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<SignatureRecord>, SignatureError> {
        let rows = match account_id {
            Some(account_id) => {
                sqlx::query_as::<_, StoredSignature>(&format!(
                    "{SIGNATURE_SELECT} WHERE account_id = ? ORDER BY is_default DESC, name ASC"
                ))
                .bind(account_id)
                .fetch_all(self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, StoredSignature>(&format!(
                    "{SIGNATURE_SELECT} ORDER BY account_id, is_default DESC, name ASC"
                ))
                .fetch_all(self.pool)
                .await?
            }
        };

        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let assets = load_assets(self.pool, &row.id).await?;
            records.push(to_record(row, assets));
        }
        Ok(records)
    }

    /// Replace a signature and all assets in one write lock. The ownership
    /// check happens before clearing defaults, so an ID collision cannot mutate
    /// the other account even when two writers race.
    pub async fn upsert(
        &self,
        input: SignatureUpsertRequest,
    ) -> Result<SignatureRecord, SignatureError> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err(SignatureError::NameEmpty);
        }
        let id = input
            .id
            .unwrap_or_else(|| format!("sig-{}", uuid::Uuid::new_v4()));
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;

        let owner = sqlx::query_scalar::<_, String>(
            "SELECT account_id FROM signatures WHERE id = ? LIMIT 1",
        )
        .bind(&id)
        .fetch_optional(&mut *transaction)
        .await?;
        if owner
            .as_deref()
            .is_some_and(|owner| owner != input.account_id)
        {
            return Err(SignatureError::OwnedByAnotherAccount);
        }

        if input.is_default {
            sqlx::query(
                "UPDATE signatures SET is_default = 0, updated_at = ? WHERE account_id = ?",
            )
            .bind(&now)
            .bind(&input.account_id)
            .execute(&mut *transaction)
            .await?;
        }

        let stored = sqlx::query_as::<_, StoredSignature>(&format!(
            "INSERT INTO signatures
             (id, account_id, name, body_text, body_html, short_body_text, short_body_html,
              is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                body_text = excluded.body_text,
                body_html = excluded.body_html,
                short_body_text = excluded.short_body_text,
                short_body_html = excluded.short_body_html,
                is_default = excluded.is_default,
                updated_at = excluded.updated_at
             WHERE signatures.account_id = excluded.account_id
             RETURNING {}",
            SIGNATURE_COLUMNS
        ))
        .bind(&id)
        .bind(&input.account_id)
        .bind(name)
        .bind(&input.body_text)
        .bind(input.body_html.as_deref())
        .bind(&input.short_body_text)
        .bind(input.short_body_html.as_deref())
        .bind(i64::from(input.is_default))
        .bind(&now)
        .bind(&now)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(SignatureError::OwnedByAnotherAccount)?;

        sqlx::query("DELETE FROM signature_assets WHERE signature_id = ?")
            .bind(&id)
            .execute(&mut *transaction)
            .await?;
        let mut assets = Vec::with_capacity(input.assets.len());
        for asset in input.assets {
            let asset_id = format!("sig-asset-{}", uuid::Uuid::new_v4());
            sqlx::query(
                "INSERT INTO signature_assets
                 (id, signature_id, filename, content_type, content_id, content, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&asset_id)
            .bind(&id)
            .bind(&asset.filename)
            .bind(&asset.mime_type)
            .bind(&asset.content_id)
            .bind(&asset.content)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
            assets.push(SignatureAssetRecord {
                id: asset_id,
                filename: asset.filename,
                mime_type: asset.mime_type,
                content_id: asset.content_id,
                content: asset.content,
            });
        }
        transaction.commit().await?;
        Ok(to_record(stored, assets))
    }

    pub async fn contains_owned(
        &self,
        signature_id: &str,
        account_id: &str,
    ) -> Result<bool, SignatureError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(1) FROM signatures WHERE id = ? AND account_id = ?")
                .bind(signature_id)
                .bind(account_id)
                .fetch_one(self.pool)
                .await?;
        Ok(count > 0)
    }

    /// Detach the signature from drafts and delete the aggregate atomically.
    /// Sent-message provenance remains untouched by the draft predicate.
    pub async fn delete(&self, id: &str) -> Result<(), SignatureError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE messages SET signature_id = NULL, signature_variant = NULL
             WHERE signature_id = ? AND is_draft = 1",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM signatures WHERE id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_assignments(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<SignatureAssignmentRecord>, SignatureError> {
        match account_id {
            Some(account_id) => sqlx::query_as::<_, SignatureAssignmentRecord>(&format!(
                "{ASSIGNMENT_SELECT} WHERE account_id = ? ORDER BY identity_email"
            ))
            .bind(account_id)
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
            None => sqlx::query_as::<_, SignatureAssignmentRecord>(&format!(
                "{ASSIGNMENT_SELECT} ORDER BY account_id, identity_email"
            ))
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
        }
    }

    pub async fn upsert_assignment(
        &self,
        input: SignatureAssignmentUpsertRequest,
    ) -> Result<SignatureAssignmentRecord, SignatureError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(signature_id) = input.signature_id.as_deref() {
            if !contains_owned_in_transaction(&mut transaction, signature_id, &input.account_id)
                .await?
            {
                return Err(SignatureError::NotFoundForAccount);
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        let record = sqlx::query_as::<_, SignatureAssignmentRecord>(&format!(
            "INSERT INTO signature_assignments
             (account_id, identity_email, signature_id, first_contact_mode, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, identity_email) DO UPDATE SET
                signature_id = excluded.signature_id,
                first_contact_mode = excluded.first_contact_mode,
                updated_at = excluded.updated_at
             RETURNING {}",
            ASSIGNMENT_COLUMNS
        ))
        .bind(&input.account_id)
        .bind(&input.identity_email)
        .bind(input.signature_id.as_deref())
        .bind(input.first_contact_mode.as_str())
        .bind(&now)
        .bind(&now)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(record)
    }

    /// Resolve explicit, assigned, or default signature choice in that order.
    /// The app supplies thread history because message reads have a separate owner.
    pub async fn resolve(
        &self,
        input: SignatureResolveRequest,
    ) -> Result<Option<ResolvedSignatureRecord>, SignatureError> {
        if input.preference == SignaturePreference::None {
            return Ok(None);
        }
        let assignment = sqlx::query_as::<_, (Option<String>, String)>(
            "SELECT signature_id, first_contact_mode FROM signature_assignments
             WHERE account_id = ? AND lower(identity_email) = ? LIMIT 1",
        )
        .bind(&input.account_id)
        .bind(&input.identity_email)
        .fetch_optional(self.pool)
        .await?;
        let signature_id = match input.signature_id {
            Some(id) => Some(id),
            None => assignment
                .as_ref()
                .and_then(|(id, _)| id.clone())
                .or(sqlx::query_scalar::<_, String>(
                    "SELECT id FROM signatures WHERE account_id = ?
                     ORDER BY is_default DESC, name ASC LIMIT 1",
                )
                .bind(&input.account_id)
                .fetch_optional(self.pool)
                .await?),
        };
        let Some(signature_id) = signature_id else {
            return Ok(None);
        };
        let signature = self.load_owned(&signature_id, &input.account_id).await?;
        let mode = FirstContactMode::parse(
            assignment
                .as_ref()
                .map(|(_, mode)| mode.as_str())
                .unwrap_or("always_full"),
        )?;
        let mut variant = match input.preference {
            SignaturePreference::Long => SignatureVariant::Long,
            SignaturePreference::Short => SignatureVariant::Short,
            SignaturePreference::None => return Ok(None),
            SignaturePreference::Auto => match mode {
                FirstContactMode::AlwaysFull => SignatureVariant::Long,
                FirstContactMode::AlwaysShort => SignatureVariant::Short,
                FirstContactMode::FullThenShort => {
                    if input.sent_from_identity_in_thread {
                        SignatureVariant::Short
                    } else {
                        SignatureVariant::Long
                    }
                }
            },
        };
        if variant == SignatureVariant::Short
            && signature.short_body_text.trim().is_empty()
            && signature
                .short_body_html
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            variant = SignatureVariant::Long;
        }
        let (body_text, body_html) = match variant {
            SignatureVariant::Long => (signature.body_text, signature.body_html),
            SignatureVariant::Short => (signature.short_body_text, signature.short_body_html),
        };
        Ok(Some(ResolvedSignatureRecord {
            signature_id,
            variant,
            body_text,
            body_html,
            assets: signature.assets,
        }))
    }

    pub async fn list_suggestions(
        &self,
        account_id: Option<&str>,
        identity_email: Option<&str>,
        status: SignatureSuggestionStatus,
    ) -> Result<Vec<SignatureSuggestionRecord>, SignatureError> {
        match (account_id, identity_email) {
            (Some(account_id), Some(identity_email)) => {
                sqlx::query_as::<_, SignatureSuggestionRecord>(&format!(
                    "{SUGGESTION_SELECT}
                     WHERE account_id = ? AND identity_email = ? AND status = ?
                     ORDER BY evidence_count DESC, updated_at DESC"
                ))
                .bind(account_id)
                .bind(identity_email)
                .bind(status.as_str())
                .fetch_all(self.pool)
                .await
                .map_err(Into::into)
            }
            (Some(account_id), None) => sqlx::query_as::<_, SignatureSuggestionRecord>(&format!(
                "{SUGGESTION_SELECT} WHERE account_id = ? AND status = ?
                     ORDER BY evidence_count DESC, updated_at DESC"
            ))
            .bind(account_id)
            .bind(status.as_str())
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
            (None, _) => sqlx::query_as::<_, SignatureSuggestionRecord>(&format!(
                "{SUGGESTION_SELECT} WHERE status = ?
                 ORDER BY evidence_count DESC, updated_at DESC"
            ))
            .bind(status.as_str())
            .fetch_all(self.pool)
            .await
            .map_err(Into::into),
        }
    }

    pub async fn upsert_suggestions(
        &self,
        account_id: &str,
        identity_email: &str,
        suggestions: Vec<SignatureSuggestionUpsertRequest>,
    ) -> Result<(), SignatureError> {
        if suggestions.is_empty() {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        for suggestion in suggestions {
            sqlx::query(
                "INSERT INTO signature_suggestions
                 (id, account_id, identity_email, body_text, body_html, evidence_count,
                  status, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?)
                 ON CONFLICT(account_id, identity_email, body_text) DO UPDATE SET
                    evidence_count = MAX(signature_suggestions.evidence_count, excluded.evidence_count),
                    body_html = COALESCE(excluded.body_html, signature_suggestions.body_html),
                    updated_at = excluded.updated_at",
            )
            .bind(format!("sig-suggestion-{}", uuid::Uuid::new_v4()))
            .bind(account_id)
            .bind(identity_email)
            .bind(suggestion.body_text)
            .bind(suggestion.body_html)
            .bind(suggestion.evidence_count)
            .bind(&now)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Unknown IDs are a no-op, preserving stale UI and Undo behavior.
    pub async fn set_suggestion_status(
        &self,
        id: &str,
        status: SignatureSuggestionStatus,
    ) -> Result<(), SignatureError> {
        sqlx::query("UPDATE signature_suggestions SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn load_owned(
        &self,
        signature_id: &str,
        account_id: &str,
    ) -> Result<SignatureRecord, SignatureError> {
        let row = sqlx::query_as::<_, StoredSignature>(&format!(
            "{SIGNATURE_SELECT} WHERE id = ? AND account_id = ? LIMIT 1"
        ))
        .bind(signature_id)
        .bind(account_id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(SignatureError::NotFoundForAccount)?;
        let assets = load_assets(self.pool, signature_id).await?;
        Ok(to_record(row, assets))
    }
}

async fn contains_owned_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    signature_id: &str,
    account_id: &str,
) -> Result<bool, SignatureError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(1) FROM signatures WHERE id = ? AND account_id = ?")
            .bind(signature_id)
            .bind(account_id)
            .fetch_one(&mut **transaction)
            .await?;
    Ok(count > 0)
}

fn to_record(row: StoredSignature, assets: Vec<SignatureAssetRecord>) -> SignatureRecord {
    SignatureRecord {
        id: row.id,
        account_id: row.account_id,
        name: row.name,
        body_text: row.body_text,
        body_html: row.body_html,
        short_body_text: row.short_body_text,
        short_body_html: row.short_body_html,
        assets,
        is_default: row.is_default,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn load_assets(
    pool: &SqlitePool,
    signature_id: &str,
) -> Result<Vec<SignatureAssetRecord>, SignatureError> {
    sqlx::query_as::<_, SignatureAssetRecord>(
        "SELECT id, filename, content_type AS mime_type, content_id, content
         FROM signature_assets WHERE signature_id = ? ORDER BY created_at, id",
    )
    .bind(signature_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect("sqlite::memory:?cache=shared")
            .await
            .expect("memory db");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    async fn account(pool: &SqlitePool, id: &str, email: &str) {
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state,
              color, is_active, created_at, updated_at)
             VALUES (?, 'Account', ?, 'imap', '{}', '', '{}', '#000', 1, 'now', 'now')",
        )
        .bind(id)
        .bind(email)
        .execute(pool)
        .await
        .expect("account");
        sqlx::query(
            "INSERT INTO folders
             (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES (?, ?, 'Inbox', 'Inbox', 'inbox', 'INBOX', 'now', 'now')",
        )
        .bind(format!("folder-{id}"))
        .bind(id)
        .execute(pool)
        .await
        .expect("folder");
    }

    fn request(account_id: &str, name: &str, is_default: bool) -> SignatureUpsertRequest {
        SignatureUpsertRequest {
            id: None,
            account_id: account_id.into(),
            name: name.into(),
            body_text: format!("{name} long"),
            body_html: Some(format!("<p>{name} long</p>")),
            short_body_text: format!("{name} short"),
            short_body_html: Some(format!("<p>{name} short</p>")),
            assets: vec![SignatureAssetWrite {
                filename: "logo.png".into(),
                mime_type: "image/png".into(),
                content_id: "logo".into(),
                content: vec![1, 2, 3],
            }],
            is_default,
        }
    }

    #[tokio::test]
    async fn upsert_replaces_assets_and_moves_the_default_atomically() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let first = repository
            .upsert(request("a", "First", true))
            .await
            .unwrap();
        let second = repository
            .upsert(request("a", "Second", true))
            .await
            .unwrap();
        assert_eq!(second.assets[0].content, vec![1, 2, 3]);

        let rows = repository.list(Some("a")).await.unwrap();
        assert_eq!(rows[0].id, second.id);
        assert_eq!(rows.iter().filter(|row| row.is_default == 1).count(), 1);
        assert_eq!(
            rows.iter()
                .find(|row| row.id == first.id)
                .unwrap()
                .is_default,
            0
        );

        let mut update = request("a", "Second updated", true);
        update.id = Some(second.id.clone());
        update.assets[0].content = vec![9];
        let updated = repository.upsert(update).await.unwrap();
        assert_eq!(updated.assets.len(), 1);
        assert_eq!(updated.assets[0].content, vec![9]);
    }

    #[tokio::test]
    async fn cross_account_id_collision_cannot_clear_defaults_or_replace_assets() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        account(&pool, "b", "b@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let original = repository
            .upsert(request("a", "Original", true))
            .await
            .unwrap();
        let b_default = repository
            .upsert(request("b", "B default", true))
            .await
            .unwrap();

        let mut collision = request("b", "Collision", true);
        collision.id = Some(original.id.clone());
        assert!(matches!(
            repository.upsert(collision).await,
            Err(SignatureError::OwnedByAnotherAccount)
        ));
        let rows = repository.list(None).await.unwrap();
        assert_eq!(
            rows.iter().find(|row| row.id == original.id).unwrap().name,
            "Original"
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.id == b_default.id)
                .unwrap()
                .is_default,
            1
        );
    }

    #[tokio::test]
    async fn concurrent_cross_account_id_claim_has_one_owner() {
        use sqlx::sqlite::SqliteConnectOptions;
        use std::str::FromStr;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("signatures.db");
        let options = SqliteConnectOptions::from_str(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        account(&pool, "a", "a@example.test").await;
        account(&pool, "b", "b@example.test").await;
        let mut first = request("a", "A", true);
        first.id = Some("shared-id".into());
        let mut second = request("b", "B", true);
        second.id = Some("shared-id".into());
        let first_repository = SignatureRepository::new(&pool);
        let second_repository = SignatureRepository::new(&pool);
        let (first, second) = tokio::join!(
            first_repository.upsert(first),
            second_repository.upsert(second)
        );
        assert_ne!(first.is_ok(), second.is_ok());
        assert!(matches!(
            first.as_ref().err().or(second.as_ref().err()),
            Some(SignatureError::OwnedByAnotherAccount)
        ));
        let rows = SignatureRepository::new(&pool).list(None).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn asset_failure_rolls_back_replacement_and_default_change() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let first = repository
            .upsert(request("a", "First", true))
            .await
            .unwrap();
        let second = repository
            .upsert(request("a", "Second", false))
            .await
            .unwrap();
        let mut invalid = request("a", "Changed", true);
        invalid.id = Some(second.id.clone());
        invalid.assets.push(invalid.assets[0].clone());
        assert!(matches!(
            repository.upsert(invalid).await,
            Err(SignatureError::Persistence(_))
        ));
        let rows = repository.list(Some("a")).await.unwrap();
        assert_eq!(
            rows.iter()
                .find(|row| row.id == first.id)
                .unwrap()
                .is_default,
            1
        );
        let unchanged = rows.iter().find(|row| row.id == second.id).unwrap();
        assert_eq!(unchanged.name, "Second");
        assert_eq!(unchanged.assets.len(), 1);
    }

    #[tokio::test]
    async fn assignment_and_resolution_preserve_priority_modes_and_short_fallback() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let default = repository
            .upsert(request("a", "Default", true))
            .await
            .unwrap();
        let explicit = repository
            .upsert(request("a", "Explicit", false))
            .await
            .unwrap();
        repository
            .upsert_assignment(SignatureAssignmentUpsertRequest {
                account_id: "a".into(),
                identity_email: "alias@example.test".into(),
                signature_id: Some(default.id.clone()),
                first_contact_mode: FirstContactMode::FullThenShort,
            })
            .await
            .unwrap();

        let resolved = repository
            .resolve(SignatureResolveRequest {
                account_id: "a".into(),
                identity_email: "alias@example.test".into(),
                signature_id: Some(explicit.id.clone()),
                preference: SignaturePreference::Auto,
                sent_from_identity_in_thread: true,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.signature_id, explicit.id);
        assert_eq!(resolved.variant, SignatureVariant::Short);

        let mut without_short = request("a", "No short", false);
        without_short.short_body_text.clear();
        without_short.short_body_html = None;
        let without_short = repository.upsert(without_short).await.unwrap();
        let resolved = repository
            .resolve(SignatureResolveRequest {
                account_id: "a".into(),
                identity_email: "other@example.test".into(),
                signature_id: Some(without_short.id),
                preference: SignaturePreference::Short,
                sent_from_identity_in_thread: false,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.variant, SignatureVariant::Long);
    }

    #[tokio::test]
    async fn assignment_rejects_a_signature_from_another_account() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        account(&pool, "b", "b@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let signature = repository.upsert(request("a", "A", true)).await.unwrap();
        let result = repository
            .upsert_assignment(SignatureAssignmentUpsertRequest {
                account_id: "b".into(),
                identity_email: "b@example.test".into(),
                signature_id: Some(signature.id),
                first_contact_mode: FirstContactMode::AlwaysFull,
            })
            .await;
        assert!(matches!(result, Err(SignatureError::NotFoundForAccount)));
    }

    #[tokio::test]
    async fn assignment_race_cannot_attach_an_id_recreated_under_another_account() {
        use sqlx::sqlite::SqliteConnectOptions;
        use std::{str::FromStr, sync::Arc, time::Duration};
        use tokio::sync::Barrier;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("assignment-race.db");
        let options = SqliteConnectOptions::from_str(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        account(&pool, "a", "a@example.test").await;
        account(&pool, "b", "b@example.test").await;

        for iteration in 0..16 {
            sqlx::query("DELETE FROM signature_assignments")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("DELETE FROM signatures WHERE id = 'shared-id'")
                .execute(&pool)
                .await
                .unwrap();
            let mut signature = request("a", &format!("A {iteration}"), false);
            signature.id = Some("shared-id".into());
            SignatureRepository::new(&pool)
                .upsert(signature)
                .await
                .unwrap();

            let barrier = Arc::new(Barrier::new(2));
            let assignment_barrier = barrier.clone();
            let replacement_barrier = barrier.clone();
            let assignment_repository = SignatureRepository::new(&pool);
            let assignment = async {
                assignment_barrier.wait().await;
                assignment_repository
                    .upsert_assignment(SignatureAssignmentUpsertRequest {
                        account_id: "a".into(),
                        identity_email: "a@example.test".into(),
                        signature_id: Some("shared-id".into()),
                        first_contact_mode: FirstContactMode::AlwaysFull,
                    })
                    .await
            };
            let replacement = async {
                replacement_barrier.wait().await;
                let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
                sqlx::query("DELETE FROM signatures WHERE id = 'shared-id'")
                    .execute(&mut *transaction)
                    .await
                    .unwrap();
                sqlx::query(
                    "INSERT INTO signatures
                     (id, account_id, name, body_text, is_default, created_at, updated_at)
                     VALUES ('shared-id', 'b', 'B', 'B', 0, 'now', 'now')",
                )
                .execute(&mut *transaction)
                .await
                .unwrap();
                transaction.commit().await.unwrap();
            };

            let (assignment_result, ()) = tokio::join!(assignment, replacement);
            assert!(
                assignment_result.is_ok()
                    || matches!(assignment_result, Err(SignatureError::NotFoundForAccount))
            );
            let owner: String =
                sqlx::query_scalar("SELECT account_id FROM signatures WHERE id = 'shared-id'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(owner, "b");
            let assigned: Option<String> = sqlx::query_scalar(
                "SELECT signature_id FROM signature_assignments
                 WHERE account_id = 'a' AND identity_email = 'a@example.test'",
            )
            .fetch_optional(&pool)
            .await
            .unwrap()
            .flatten();
            assert_eq!(assigned, None);
        }
    }

    #[tokio::test]
    async fn suggestions_keep_max_evidence_and_status_updates_are_noops_for_unknown_ids() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        let repository = SignatureRepository::new(&pool);
        for evidence_count in [5, 3, 8] {
            repository
                .upsert_suggestions(
                    "a",
                    "a@example.test",
                    vec![SignatureSuggestionUpsertRequest {
                        body_text: "Regards".into(),
                        body_html: None,
                        evidence_count,
                    }],
                )
                .await
                .unwrap();
        }
        let rows = repository
            .list_suggestions(
                Some("a"),
                Some("a@example.test"),
                SignatureSuggestionStatus::Pending,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].evidence_count, 8);
        repository
            .set_suggestion_status(&rows[0].id, SignatureSuggestionStatus::Dismissed)
            .await
            .unwrap();
        repository
            .set_suggestion_status("missing", SignatureSuggestionStatus::Pending)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_detaches_drafts_but_preserves_sent_provenance() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let signature = repository
            .upsert(request("a", "Delete", true))
            .await
            .unwrap();
        for (id, is_draft, is_sent) in [("draft", 1, 0), ("sent", 0, 1)] {
            sqlx::query(
                "INSERT INTO messages
                 (id, account_id, folder_id, from_address, to_addresses, date, is_draft, is_sent,
                  signature_id, signature_variant, created_at, updated_at)
                 VALUES (?, 'a', 'folder-a', 'a@example.test', '[]', 'now', ?, ?, ?, 'long', 'now', 'now')",
            )
            .bind(id)
            .bind(is_draft)
            .bind(is_sent)
            .bind(&signature.id)
            .execute(&pool)
            .await
            .unwrap();
        }
        repository.delete(&signature.id).await.unwrap();
        let draft: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT signature_id, signature_variant FROM messages WHERE id = 'draft'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let sent: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT signature_id, signature_variant FROM messages WHERE id = 'sent'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(draft, (None, None));
        assert_eq!(sent, (Some(signature.id), Some("long".into())));
    }

    #[tokio::test]
    async fn failed_delete_rolls_back_draft_detach() {
        let pool = pool().await;
        account(&pool, "a", "a@example.test").await;
        let repository = SignatureRepository::new(&pool);
        let signature = repository
            .upsert(request("a", "Delete", true))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages
             (id, account_id, folder_id, from_address, to_addresses, date, is_draft, signature_id,
              signature_variant, created_at, updated_at)
             VALUES ('draft', 'a', 'folder-a', 'a@example.test', '[]', 'now', 1, ?, 'long', 'now', 'now')",
        )
        .bind(&signature.id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER fail_signature_delete BEFORE DELETE ON signatures
             BEGIN SELECT RAISE(ABORT, 'blocked'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(matches!(
            repository.delete(&signature.id).await,
            Err(SignatureError::Persistence(_))
        ));
        let value: Option<String> =
            sqlx::query_scalar("SELECT signature_id FROM messages WHERE id = 'draft'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(value.as_deref(), Some(signature.id.as_str()));
    }
}
