//! Tauri-free persistence and matching policy for blocked sender rules.

use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedSenderPatternType {
    Address,
    Domain,
}

impl BlockedSenderPatternType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::Domain => "domain",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "address" => Some(Self::Address),
            "domain" => Some(Self::Domain),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct BlockedSenderRecord {
    pub id: String,
    pub account_id: Option<String>,
    pub pattern: String,
    pub pattern_type: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct BlockedSenderUpsertRequest {
    pub account_id: Option<String>,
    pub pattern: String,
}

#[derive(Debug, Error)]
pub enum BlockedSenderError {
    #[error("blocked sender pattern is empty")]
    PatternEmpty,
    #[error("blocked sender pattern matches one of the user's own addresses")]
    OwnAddress,
    #[error("blocked sender persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for BlockedSenderError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns durable blocked-sender rules and their matching policy. Mailbox
/// selection, local thread projection, provider moves, and notifications stay
/// in their respective aggregates.
pub struct BlockedSenderRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> BlockedSenderRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<BlockedSenderRecord>, BlockedSenderError> {
        sqlx::query_as::<_, BlockedSenderRecord>(
            "SELECT id, account_id, pattern, pattern_type, created_at
             FROM blocked_senders
             ORDER BY pattern ASC, COALESCE(account_id, '') ASC, created_at ASC, id ASC",
        )
        .fetch_all(self.pool)
        .await
        .map_err(Into::into)
    }

    /// Insert a rule once, then return the canonical row. Repeating the same
    /// normalized pattern in the same account or global scope is a no-op.
    pub async fn upsert(
        &self,
        input: BlockedSenderUpsertRequest,
    ) -> Result<BlockedSenderRecord, BlockedSenderError> {
        let (pattern, pattern_type) = normalize_pattern(&input.pattern)?;
        let account_id = input.account_id;
        let id = format!("blk-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();

        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        // A rule against an own address or domain would file the user's own
        // replies and sent copies as spam.
        let own_addresses: Vec<String> = sqlx::query_scalar(
            "SELECT email FROM accounts UNION SELECT email FROM account_aliases",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if own_addresses
            .iter()
            .any(|email| matches_sender(&pattern, pattern_type.as_str(), email))
        {
            return Err(BlockedSenderError::OwnAddress);
        }
        sqlx::query(
            "INSERT INTO blocked_senders (id, account_id, pattern, pattern_type, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(COALESCE(account_id, ''), pattern) DO NOTHING",
        )
        .bind(&id)
        .bind(account_id.as_deref())
        .bind(&pattern)
        .bind(pattern_type.as_str())
        .bind(&now)
        .execute(&mut *transaction)
        .await?;

        let record = sqlx::query_as::<_, BlockedSenderRecord>(
            "SELECT id, account_id, pattern, pattern_type, created_at
             FROM blocked_senders
             WHERE pattern = ? AND COALESCE(account_id, '') = COALESCE(?, '')
             LIMIT 1",
        )
        .bind(&pattern)
        .bind(account_id.as_deref())
        .fetch_one(&mut *transaction)
        .await
        .map_err(BlockedSenderError::from)?;
        transaction.commit().await?;
        Ok(record)
    }

    /// Deleting an unknown rule is intentionally a no-op for stale UI actions.
    pub async fn delete(&self, id: &str) -> Result<(), BlockedSenderError> {
        sqlx::query("DELETE FROM blocked_senders WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Check exact address or exact domain matches in account and global scope.
    pub async fn is_blocked(
        &self,
        account_id: &str,
        from: &str,
    ) -> Result<bool, BlockedSenderError> {
        let rows = sqlx::query_as::<_, BlockedSenderRecord>(
            "SELECT id, account_id, pattern, pattern_type, created_at
             FROM blocked_senders
             WHERE account_id IS NULL OR account_id = ?",
        )
        .bind(account_id)
        .fetch_all(self.pool)
        .await?;

        Ok(rows
            .iter()
            .any(|rule| matches_sender(&rule.pattern, &rule.pattern_type, from)))
    }
}

/// Normalize a user-provided rule and preserve the historic address/domain
/// classification heuristic used by the settings command.
pub fn normalize_pattern(
    pattern: &str,
) -> Result<(String, BlockedSenderPatternType), BlockedSenderError> {
    let pattern = pattern.trim().to_lowercase();
    if pattern.is_empty() {
        return Err(BlockedSenderError::PatternEmpty);
    }
    let pattern_type = classify_pattern(&pattern);
    Ok((pattern, pattern_type))
}

pub fn classify_pattern(pattern: &str) -> BlockedSenderPatternType {
    if pattern.starts_with('*')
        || pattern.starts_with('@')
        || (!pattern.contains('@') && pattern.contains('.'))
    {
        BlockedSenderPatternType::Domain
    } else {
        BlockedSenderPatternType::Address
    }
}

/// Apply the same strict matcher used by ingest to a stored rule. Domain
/// patterns match the domain after `@`, never a suffix of another domain.
pub fn matches_sender(pattern: &str, pattern_type: &str, from: &str) -> bool {
    let normalized_pattern = pattern.trim().to_lowercase();
    let normalized_from = from.trim().to_lowercase();
    let Some(pattern_type) = BlockedSenderPatternType::parse(pattern_type) else {
        return false;
    };
    match pattern_type {
        BlockedSenderPatternType::Address => normalized_pattern == normalized_from,
        BlockedSenderPatternType::Domain => {
            let Some((_, domain)) = normalized_from.split_once('@') else {
                return false;
            };
            let bare = normalized_pattern
                .trim_start_matches('*')
                .trim_start_matches('@');
            domain == bare
        }
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    async fn insert_account(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state,
              color, is_active, created_at, updated_at)
             VALUES (?, 'Account', ?, 'imap', '{}', '', '{}', '#000', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(format!("{id}@example.com"))
        .execute(pool)
        .await
        .expect("account");
    }

    #[test]
    fn normalizes_and_classifies_patterns() {
        assert_eq!(
            normalize_pattern("  Alice@Example.COM ").unwrap(),
            (
                "alice@example.com".into(),
                BlockedSenderPatternType::Address
            )
        );
        assert_eq!(
            normalize_pattern("*@Spam.Example.com").unwrap(),
            (
                "*@spam.example.com".into(),
                BlockedSenderPatternType::Domain
            )
        );
        assert_eq!(
            normalize_pattern("@Spam.Example.com").unwrap(),
            ("@spam.example.com".into(), BlockedSenderPatternType::Domain)
        );
        assert_eq!(
            normalize_pattern("spam.example.com").unwrap(),
            ("spam.example.com".into(), BlockedSenderPatternType::Domain)
        );
    }

    #[test]
    fn empty_pattern_is_rejected() {
        assert!(matches!(
            normalize_pattern(" \t"),
            Err(BlockedSenderError::PatternEmpty)
        ));
    }

    #[test]
    fn strict_matching_does_not_accept_domain_suffixes() {
        assert!(matches_sender(
            "*@spam.example.com",
            "domain",
            "Alice@Spam.Example.com"
        ));
        assert!(!matches_sender(
            "*@spam.example.com",
            "domain",
            "Alice@notspam.example.com"
        ));
        assert!(!matches_sender(
            "spam.example.com",
            "domain",
            "spam.example.com"
        ));
        assert!(matches_sender(
            "alice@example.com",
            "address",
            " Alice@Example.COM "
        ));
        assert!(!matches_sender(
            "alice@example.com",
            "address",
            "other@example.com"
        ));
        assert!(!matches_sender(
            "alice@example.com",
            "unknown",
            "alice@example.com"
        ));
    }

    #[tokio::test]
    async fn own_addresses_and_domains_cannot_be_blocked() {
        let pool = pool().await;
        insert_account(&pool, "acc-1").await;
        sqlx::query(
            "INSERT INTO account_aliases (id, account_id, email, created_at, updated_at)
             VALUES ('alias-1', 'acc-1', 'me@alias.example', '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .expect("alias");
        let repository = BlockedSenderRepository::new(&pool);
        for pattern in [
            "ACC-1@example.com",
            "*@example.com",
            "@example.com",
            "alias.example",
            "me@alias.example",
        ] {
            let error = repository
                .upsert(BlockedSenderUpsertRequest {
                    account_id: Some("acc-1".into()),
                    pattern: pattern.into(),
                })
                .await
                .expect_err(pattern);
            assert!(matches!(error, BlockedSenderError::OwnAddress), "{pattern}");
        }
        assert!(repository.list().await.unwrap().is_empty());
        repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: Some("acc-1".into()),
                pattern: "*@other.example.com".into(),
            })
            .await
            .expect("foreign domain");
    }

    #[tokio::test]
    async fn repository_lists_in_stable_pattern_order() {
        let pool = pool().await;
        let repository = BlockedSenderRepository::new(&pool);
        repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: None,
                pattern: "z@example.com".into(),
            })
            .await
            .unwrap();
        repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: None,
                pattern: "a@example.com".into(),
            })
            .await
            .unwrap();
        let rows = repository.list().await.unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.pattern.as_str())
                .collect::<Vec<_>>(),
            ["a@example.com", "z@example.com"]
        );
    }

    #[tokio::test]
    async fn duplicate_upsert_is_idempotent_per_scope() {
        let pool = pool().await;
        insert_account(&pool, "acc-a").await;
        let repository = BlockedSenderRepository::new(&pool);
        let first = repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: Some("acc-a".into()),
                pattern: " Alice@Example.COM ".into(),
            })
            .await
            .unwrap();
        let second = repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: Some("acc-a".into()),
                pattern: "alice@example.com".into(),
            })
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(repository.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn global_and_account_rules_have_separate_scope() {
        let pool = pool().await;
        insert_account(&pool, "acc-a").await;
        let repository = BlockedSenderRepository::new(&pool);
        repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: None,
                pattern: "global@example.com".into(),
            })
            .await
            .unwrap();
        repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: Some("acc-a".into()),
                pattern: "local@example.com".into(),
            })
            .await
            .unwrap();
        assert!(repository
            .is_blocked("acc-a", "global@example.com")
            .await
            .unwrap());
        assert!(repository
            .is_blocked("acc-b", "global@example.com")
            .await
            .unwrap());
        assert!(repository
            .is_blocked("acc-a", "local@example.com")
            .await
            .unwrap());
        assert!(!repository
            .is_blocked("acc-b", "local@example.com")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn delete_unknown_rule_is_a_no_op() {
        let pool = pool().await;
        let repository = BlockedSenderRepository::new(&pool);
        repository.delete("missing").await.unwrap();
    }

    #[tokio::test]
    async fn account_foreign_key_cascades_rules() {
        let pool = pool().await;
        insert_account(&pool, "acc-a").await;
        let repository = BlockedSenderRepository::new(&pool);
        repository
            .upsert(BlockedSenderUpsertRequest {
                account_id: Some("acc-a".into()),
                pattern: "cascade@example.com".into(),
            })
            .await
            .unwrap();
        sqlx::query("DELETE FROM accounts WHERE id = 'acc-a'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(repository.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn persistence_errors_are_typed() {
        let pool = pool().await;
        sqlx::query("DROP TABLE blocked_senders")
            .execute(&pool)
            .await
            .unwrap();
        let error = BlockedSenderRepository::new(&pool)
            .list()
            .await
            .expect_err("dropped table should fail");
        assert!(matches!(error, BlockedSenderError::Persistence(_)));
    }

    #[tokio::test]
    async fn repository_is_blocked_reports_persistence_errors() {
        let pool = pool().await;
        sqlx::query("DROP TABLE blocked_senders")
            .execute(&pool)
            .await
            .unwrap();
        let error = BlockedSenderRepository::new(&pool)
            .is_blocked("acc-a", "sender@example.com")
            .await
            .expect_err("dropped table should fail");
        assert!(matches!(error, BlockedSenderError::Persistence(_)));
    }
}
