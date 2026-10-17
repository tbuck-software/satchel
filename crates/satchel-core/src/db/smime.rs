//! Tauri-free persistence for S/MIME certificate records and key material.

use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmimeCertificateMetadataRecord {
    pub id: String,
    pub label: String,
    pub cert_type: String,
    pub fingerprint: String,
    pub subject_email: Option<String>,
    pub subject_cn: Option<String>,
    pub issuer_cn: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub has_passphrase: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// How a certificate entered the store. Manual imports are a decision the user
/// made; automatic ones are whatever a signed message happened to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmimeTrustOrigin {
    Manual,
    Auto,
}

impl SmimeTrustOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SmimeCertificateInsert {
    pub label: Option<String>,
    pub trust_origin: SmimeTrustOrigin,
    pub fingerprint: String,
    pub subject_email: Option<String>,
    pub subject_cn: Option<String>,
    pub issuer_cn: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub pem_certificate: String,
    pub pem_private_key_encrypted: Option<String>,
    pub has_passphrase: bool,
}

/// Private certificate material deliberately has no `Debug` implementation.
/// Logging it could disclose a certificate or an encrypted private key.
pub struct SmimePrivateCertificateMaterial {
    pub id: String,
    pub pem_certificate: String,
    pub pem_private_key_encrypted: Option<String>,
    pub has_passphrase: bool,
}

/// Recipient material deliberately has no `Debug` implementation because it
/// carries the complete PEM certificate.
pub struct SmimeRecipientCertificate {
    pub subject_email: String,
    pub pem_certificate: String,
}

#[derive(Debug, Error)]
pub enum SmimeRepositoryError {
    #[error("S/MIME certificate not found")]
    NotFound,
    #[error("missing S/MIME recipient certificates")]
    MissingRecipients { recipients: Vec<String> },
    #[error("ambiguous S/MIME recipient certificates")]
    AmbiguousRecipients { recipients: Vec<String> },
    #[error("S/MIME certificate persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for SmimeRepositoryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

#[derive(FromRow)]
struct StoredSmimeCertificateMetadata {
    id: String,
    label: String,
    cert_type: String,
    fingerprint: String,
    subject_email: Option<String>,
    subject_cn: Option<String>,
    issuer_cn: Option<String>,
    not_before: Option<String>,
    not_after: Option<String>,
    has_passphrase: i64,
    created_at: String,
    updated_at: String,
}

impl From<StoredSmimeCertificateMetadata> for SmimeCertificateMetadataRecord {
    fn from(row: StoredSmimeCertificateMetadata) -> Self {
        Self {
            id: row.id,
            label: row.label,
            cert_type: row.cert_type,
            fingerprint: row.fingerprint,
            subject_email: row.subject_email,
            subject_cn: row.subject_cn,
            issuer_cn: row.issuer_cn,
            not_before: row.not_before,
            not_after: row.not_after,
            has_passphrase: row.has_passphrase > 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(FromRow)]
struct StoredSmimePrivateCertificateMaterial {
    id: String,
    pem_certificate: String,
    pem_private_key_encrypted: Option<String>,
    has_passphrase: i64,
}

impl From<StoredSmimePrivateCertificateMaterial> for SmimePrivateCertificateMaterial {
    fn from(row: StoredSmimePrivateCertificateMaterial) -> Self {
        Self {
            id: row.id,
            pem_certificate: row.pem_certificate,
            pem_private_key_encrypted: row.pem_private_key_encrypted,
            has_passphrase: row.has_passphrase > 0,
        }
    }
}

#[derive(FromRow)]
struct StoredSmimeRecipientCertificate {
    subject_email: Option<String>,
    pem_certificate: String,
    fingerprint: String,
    /// Selection ranks computed in SQL so ordering and tie detection cannot
    /// drift apart.
    cert_rank: i64,
    origin_rank: i64,
}

const METADATA_SELECT: &str =
    "SELECT id, label, cert_type, fingerprint, subject_email, subject_cn, issuer_cn,
            not_before, not_after, has_passphrase, created_at, updated_at
     FROM smime_certificates";

const PRIVATE_MATERIAL_SELECT: &str =
    "SELECT id, pem_certificate, pem_private_key_encrypted, has_passphrase
     FROM smime_certificates";

/// Owns S/MIME certificate storage and the selection rules used by crypto
/// operations. PEM parsing, key encryption and unlock sessions stay in the app
/// crate.
pub struct SmimeCertificateRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SmimeCertificateRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_metadata(
        &self,
    ) -> Result<Vec<SmimeCertificateMetadataRecord>, SmimeRepositoryError> {
        sqlx::query_as::<_, StoredSmimeCertificateMetadata>(&format!(
            "{METADATA_SELECT} ORDER BY created_at DESC"
        ))
        .fetch_all(self.pool)
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
    }

    /// Inserts a parsed certificate, deriving private/public type from whether
    /// encrypted private-key material is present. A duplicate fingerprint of
    /// the same type returns `None`, matching the database's unique index.
    pub async fn insert(
        &self,
        input: SmimeCertificateInsert,
    ) -> Result<Option<SmimeCertificateMetadataRecord>, SmimeRepositoryError> {
        let id = format!("smime-cert-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        let cert_type = if input.pem_private_key_encrypted.is_some() {
            "private"
        } else {
            "public"
        };
        let label = input
            .label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(input.subject_email.as_deref())
            .or(input.subject_cn.as_deref())
            .unwrap_or("S/MIME certificate")
            .to_owned();

        let result = sqlx::query(
            "INSERT OR IGNORE INTO smime_certificates
                (id, label, cert_type, trust_origin, fingerprint, subject_email, subject_cn,
                 issuer_cn, not_before, not_after, pem_certificate, pem_private_key_encrypted,
                 has_passphrase, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&label)
        .bind(cert_type)
        .bind(input.trust_origin.as_str())
        .bind(&input.fingerprint)
        .bind(input.subject_email.as_deref())
        .bind(input.subject_cn.as_deref())
        .bind(input.issuer_cn.as_deref())
        .bind(input.not_before.as_deref())
        .bind(input.not_after.as_deref())
        .bind(input.pem_certificate.trim())
        .bind(input.pem_private_key_encrypted.as_deref())
        .bind(if input.has_passphrase { 1_i64 } else { 0_i64 })
        .bind(&now)
        .bind(&now)
        .execute(self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        Ok(Some(SmimeCertificateMetadataRecord {
            id,
            label,
            cert_type: cert_type.to_owned(),
            fingerprint: input.fingerprint,
            subject_email: input.subject_email,
            subject_cn: input.subject_cn,
            issuer_cn: input.issuer_cn,
            not_before: input.not_before,
            not_after: input.not_after,
            has_passphrase: input.has_passphrase,
            created_at: now.clone(),
            updated_at: now,
        }))
    }

    pub async fn delete(&self, cert_id: &str) -> Result<(), SmimeRepositoryError> {
        let result = sqlx::query("DELETE FROM smime_certificates WHERE id = ?")
            .bind(cert_id)
            .execute(self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(SmimeRepositoryError::NotFound);
        }
        Ok(())
    }

    /// Resolves one current certificate per non-empty recipient in input order.
    ///
    /// Ranked by origin, not by age: public certificates win over private
    /// copies, a manual import wins over an auto-import, and only within one
    /// rank does the newest import win. Where the rank leaves a genuine choice
    /// between different fingerprints, the address is reported as ambiguous
    /// instead of picked — the newest certificate is not the more trustworthy
    /// one, and encrypting to the wrong key is silent.
    ///
    /// Missing and ambiguous recipients are each reported together.
    pub async fn resolve_recipient_certificates(
        &self,
        emails: &[String],
    ) -> Result<Vec<SmimeRecipientCertificate>, SmimeRepositoryError> {
        let mut resolved = Vec::new();
        let mut missing = Vec::new();
        let mut ambiguous = Vec::new();
        for email in emails {
            let normalized = email.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                continue;
            }
            let rows = sqlx::query_as::<_, StoredSmimeRecipientCertificate>(
                "SELECT subject_email, pem_certificate, fingerprint,
                        CASE cert_type WHEN 'public' THEN 0 ELSE 1 END AS cert_rank,
                        CASE trust_origin WHEN 'manual' THEN 0 ELSE 1 END AS origin_rank
                 FROM smime_certificates
                 WHERE LOWER(subject_email) = ? AND cert_type IN ('public', 'private')
                 ORDER BY cert_rank, origin_rank, created_at DESC",
            )
            .bind(&normalized)
            .fetch_all(self.pool)
            .await?;
            let Some(best) = rows.first() else {
                missing.push(email.clone());
                continue;
            };
            let contested = rows.iter().skip(1).any(|row| {
                row.cert_rank == best.cert_rank
                    && row.origin_rank == best.origin_rank
                    && row.fingerprint != best.fingerprint
            });
            if contested {
                ambiguous.push(email.clone());
                continue;
            }
            resolved.push(SmimeRecipientCertificate {
                subject_email: best.subject_email.clone().unwrap_or_else(|| email.clone()),
                pem_certificate: best.pem_certificate.clone(),
            });
        }
        if !missing.is_empty() {
            return Err(SmimeRepositoryError::MissingRecipients {
                recipients: missing,
            });
        }
        if !ambiguous.is_empty() {
            return Err(SmimeRepositoryError::AmbiguousRecipients {
                recipients: ambiguous,
            });
        }
        Ok(resolved)
    }

    pub async fn private_certificates_for_unlock(
        &self,
        cert_id: Option<&str>,
    ) -> Result<Vec<SmimePrivateCertificateMaterial>, SmimeRepositoryError> {
        let rows = match cert_id {
            Some(cert_id) => {
                sqlx::query_as::<_, StoredSmimePrivateCertificateMaterial>(&format!(
                    "{PRIVATE_MATERIAL_SELECT}
                     WHERE id = ? AND cert_type = 'private'
                     LIMIT 1"
                ))
                .bind(cert_id)
                .fetch_all(self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, StoredSmimePrivateCertificateMaterial>(&format!(
                    "{PRIVATE_MATERIAL_SELECT}
                     WHERE cert_type = 'private'
                     ORDER BY created_at DESC"
                ))
                .fetch_all(self.pool)
                .await?
            }
        };
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn private_signing_material(
        &self,
        cert_id: &str,
    ) -> Result<Option<SmimePrivateCertificateMaterial>, SmimeRepositoryError> {
        sqlx::query_as::<_, StoredSmimePrivateCertificateMaterial>(&format!(
            "{PRIVATE_MATERIAL_SELECT}
             WHERE id = ? AND cert_type = 'private'
             LIMIT 1"
        ))
        .bind(cert_id)
        .fetch_optional(self.pool)
        .await
        .map(|row| row.map(Into::into))
        .map_err(Into::into)
    }

    pub async fn private_decryption_material(
        &self,
        cert_ids: &[String],
    ) -> Result<Vec<SmimePrivateCertificateMaterial>, SmimeRepositoryError> {
        if cert_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "{PRIVATE_MATERIAL_SELECT}
             WHERE cert_type = 'private' AND id IN ("
        ));
        let mut separated = query.separated(", ");
        for cert_id in cert_ids {
            separated.push_bind(cert_id);
        }
        separated.push_unseparated(")");
        query
            .build_query_as::<StoredSmimePrivateCertificateMaterial>()
            .fetch_all(self.pool)
            .await
            .map(|rows| rows.into_iter().map(Into::into).collect())
            .map_err(Into::into)
    }

    pub async fn has_private_certificate(&self) -> Result<bool, SmimeRepositoryError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1) FROM smime_certificates WHERE cert_type = 'private'",
        )
        .fetch_one(self.pool)
        .await
        .map(|count| count > 0)
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

    use super::{
        SmimeCertificateInsert, SmimeCertificateRepository, SmimeRepositoryError, SmimeTrustOrigin,
    };

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    fn certificate(fingerprint: &str, email: Option<&str>) -> SmimeCertificateInsert {
        SmimeCertificateInsert {
            label: None,
            trust_origin: SmimeTrustOrigin::Manual,
            fingerprint: fingerprint.to_owned(),
            subject_email: email.map(str::to_owned),
            subject_cn: Some("Certificate Owner".to_owned()),
            issuer_cn: Some("Test CA".to_owned()),
            not_before: Some("2026-01-01T00:00:00Z".to_owned()),
            not_after: Some("2027-01-01T00:00:00Z".to_owned()),
            pem_certificate: format!(" CERTIFICATE {fingerprint} \n"),
            pem_private_key_encrypted: None,
            has_passphrase: false,
        }
    }

    #[tokio::test]
    async fn insert_deduplicates_by_fingerprint_and_type_and_derives_labels() {
        let pool = pool().await;
        let repo = SmimeCertificateRepository::new(&pool);
        let public = repo
            .insert(certificate("AA11", Some("Person@Example.test")))
            .await
            .expect("insert public")
            .expect("new public");
        assert_eq!(public.label, "Person@Example.test");
        assert_eq!(public.cert_type, "public");
        assert!(!public.has_passphrase);

        assert!(repo
            .insert(certificate("AA11", Some("other@example.test")))
            .await
            .expect("duplicate")
            .is_none());

        let mut private = certificate("AA11", Some("Person@Example.test"));
        private.label = Some("  Signing certificate  ".to_owned());
        private.pem_private_key_encrypted = Some("encrypted-private-key".to_owned());
        private.has_passphrase = true;
        let private = repo
            .insert(private)
            .await
            .expect("insert private")
            .expect("new private");
        assert_eq!(private.label, "Signing certificate");
        assert_eq!(private.cert_type, "private");
        assert!(private.has_passphrase);

        sqlx::query(
            "UPDATE smime_certificates
             SET created_at = CASE id WHEN ? THEN '2026-01-01T00:00:00Z'
                                      ELSE '2026-02-01T00:00:00Z' END
             WHERE id IN (?, ?)",
        )
        .bind(&public.id)
        .bind(&public.id)
        .bind(&private.id)
        .execute(&pool)
        .await
        .expect("set deterministic list order");

        let listed = repo.list_metadata().await.expect("list metadata");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, private.id);
        assert!(listed.iter().all(|record| record.fingerprint == "AA11"));
    }

    /// The manual import stays the encryption target even when a signed
    /// message delivers a newer certificate for the same address.
    #[tokio::test]
    async fn recipient_resolution_prefers_the_manually_imported_certificate() {
        let pool = pool().await;
        let repo = SmimeCertificateRepository::new(&pool);
        let mut private = certificate("PRIVATE", Some("person@example.test"));
        private.pem_certificate = "private-pem".to_owned();
        private.pem_private_key_encrypted = Some("encrypted-private-key".to_owned());
        repo.insert(private).await.expect("insert private");

        let mut public = certificate("PUBLIC", Some("Person@Example.test"));
        public.pem_certificate = "manual-public-pem".to_owned();
        let manual_public = repo
            .insert(public)
            .await
            .expect("insert public")
            .expect("new public");
        let mut auto_public = certificate("NEWER-PUBLIC", Some("Person@Example.test"));
        auto_public.pem_certificate = "auto-public-pem".to_owned();
        auto_public.trust_origin = SmimeTrustOrigin::Auto;
        let auto_public = repo
            .insert(auto_public)
            .await
            .expect("insert auto public")
            .expect("auto public");
        sqlx::query(
            "UPDATE smime_certificates
             SET created_at = CASE id WHEN ? THEN '2026-01-01T00:00:00Z'
                                      ELSE '2026-02-01T00:00:00Z' END
             WHERE id IN (?, ?)",
        )
        .bind(&manual_public.id)
        .bind(&manual_public.id)
        .bind(&auto_public.id)
        .execute(&pool)
        .await
        .expect("set deterministic import order");

        let resolved = repo
            .resolve_recipient_certificates(&[
                " PERSON@example.test ".to_owned(),
                "".to_owned(),
                "person@example.test".to_owned(),
            ])
            .await
            .expect("resolve");
        assert_eq!(resolved.len(), 2);
        assert!(resolved
            .iter()
            .all(|certificate| certificate.pem_certificate == "manual-public-pem"));
        assert!(resolved
            .iter()
            .all(|certificate| certificate.subject_email == "Person@Example.test"));

        let error = match repo
            .resolve_recipient_certificates(&[
                "person@example.test".to_owned(),
                "missing@example.test".to_owned(),
                "also-missing@example.test".to_owned(),
            ])
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("missing recipients should fail"),
        };
        match error {
            SmimeRepositoryError::MissingRecipients { recipients } => assert_eq!(
                recipients,
                ["missing@example.test", "also-missing@example.test"]
            ),
            other => panic!("unexpected error: {other}"),
        }
    }

    /// Two certificates of the same standing are a question for the user, not
    /// a race the newer one wins.
    #[tokio::test]
    async fn recipient_resolution_reports_an_unresolved_tie_instead_of_choosing() {
        let pool = pool().await;
        let repo = SmimeCertificateRepository::new(&pool);
        for fingerprint in ["FIRST", "SECOND"] {
            let mut public = certificate(fingerprint, Some("person@example.test"));
            public.pem_certificate = format!("{fingerprint}-pem");
            repo.insert(public).await.expect("insert public");
        }

        let error = match repo
            .resolve_recipient_certificates(&["person@example.test".to_owned()])
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("a tie between two manual certificates must not be resolved"),
        };
        match error {
            SmimeRepositoryError::AmbiguousRecipients { recipients } => {
                assert_eq!(recipients, ["person@example.test"]);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn private_material_queries_enforce_intent_and_never_return_public_rows() {
        let pool = pool().await;
        let repo = SmimeCertificateRepository::new(&pool);
        let public = repo
            .insert(certificate("PUBLIC", Some("public@example.test")))
            .await
            .expect("insert public")
            .expect("new public");
        let mut private = certificate("PRIVATE", Some("private@example.test"));
        private.pem_certificate = "private-cert-pem".to_owned();
        private.pem_private_key_encrypted = Some("encrypted-private-key".to_owned());
        private.has_passphrase = true;
        let private = repo
            .insert(private)
            .await
            .expect("insert private")
            .expect("new private");

        assert!(repo.has_private_certificate().await.expect("has private"));
        assert!(repo
            .private_signing_material(&public.id)
            .await
            .expect("public lookup")
            .is_none());
        let signing = repo
            .private_signing_material(&private.id)
            .await
            .expect("private lookup")
            .expect("private material");
        assert_eq!(signing.id, private.id);
        assert_eq!(signing.pem_certificate, "private-cert-pem");
        assert_eq!(
            signing.pem_private_key_encrypted.as_deref(),
            Some("encrypted-private-key")
        );
        assert!(signing.has_passphrase);

        let selected = repo
            .private_decryption_material(&[public.id.clone(), private.id.clone()])
            .await
            .expect("decryption material");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, private.id);

        let all = repo
            .private_certificates_for_unlock(None)
            .await
            .expect("all private");
        assert_eq!(all.len(), 1);
        let selected = repo
            .private_certificates_for_unlock(Some(&public.id))
            .await
            .expect("public by id");
        assert!(selected.is_empty());
    }

    #[tokio::test]
    async fn delete_reports_not_found_and_updates_private_presence() {
        let pool = pool().await;
        let repo = SmimeCertificateRepository::new(&pool);
        assert!(!repo.has_private_certificate().await.expect("empty"));
        let mut private = certificate("PRIVATE", None);
        private.subject_cn = None;
        private.pem_private_key_encrypted = Some("encrypted-private-key".to_owned());
        let private = repo
            .insert(private)
            .await
            .expect("insert")
            .expect("new certificate");
        assert_eq!(private.label, "S/MIME certificate");
        assert!(repo.has_private_certificate().await.expect("present"));

        repo.delete(&private.id).await.expect("delete");
        assert!(!repo.has_private_certificate().await.expect("deleted"));
        assert!(matches!(
            repo.delete(&private.id).await,
            Err(SmimeRepositoryError::NotFound)
        ));
    }
}
