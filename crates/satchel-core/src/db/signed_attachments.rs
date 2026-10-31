//! Attachments beside a root signature come from the signed entity.
//!
//! A message shown with a root signature offers only parts of the entity that
//! signature covers: the strict splitter's entity for IMAP, POP3 and import,
//! the provider's own list matched against that entity for Gmail and Outlook.
//! Rows written that way carry `listing_rules = 1`.
//!
//! Rows written before (`0`) may name parts from outside that entity: the
//! signature part under whatever name its unsigned headers chose, or an outer
//! part an older parser kept beside a nested signature it took for the root.
//! Listings hide them beside a root signature, so no `cid:` reference resolves
//! from them either, until [`SIGNED_ENTITY_ATTACHMENTS`] has rebuilt them from
//! the stored bundle. The binding mark cannot stand in for this: a refetched
//! body clears it and leaves the attachments as they were.

use futures::future::BoxFuture;
use sqlx::{SqliteConnection, SqlitePool};

use super::data_repairs::{off_runtime, DataRepair, Prepared};
use crate::providers::traits::RemoteAttachment;

/// The rules sync and import list attachments by.
pub const LISTING_RULES: i64 = 1;

/// Whether message `m` is shown beside a root signature.
macro_rules! beside_root_signature {
    () => {
        "(m.pgp_signed IS NOT NULL OR m.smime_signed IS NOT NULL)
     AND COALESCE(CASE WHEN json_valid(m.trust_metadata)
         THEN json_extract(m.trust_metadata, '$.root_crypto') END, 0)"
    };
}

/// Whether attachment row `a` of message `m` stays out of a listing: written
/// under older rules, beside a root signature.
///
/// A part of decrypted content is not one of these. Its row is the decrypted
/// body's, rewritten each time the message is decrypted, and the signed
/// entity's own list does not hold it: see [`DECRYPTED_BESIDE_ROOT_SIGNATURE`].
pub const HIDDEN_BESIDE_ROOT_SIGNATURE: &str = concat!(
    "(a.listing_rules < 1
     AND a.id NOT LIKE 'att-pgp-%'
     AND ",
    beside_root_signature!(),
    ")"
);

/// Whether attachment row `a` of message `m` is a part of decrypted content
/// beside a root signature.
///
/// The signature covers the ciphertext, never the part. Such a row may be
/// listed only while the read it is listed beside shows the body it was
/// decrypted from; without the key the block stays armored and the verdict
/// is valid again. The listing decides that per read, not this repair.
pub const DECRYPTED_BESIDE_ROOT_SIGNATURE: &str =
    concat!("(a.id LIKE 'att-pgp-%' AND ", beside_root_signature!(), ")");

/// What a stored bundle makes of a message's rows from older rules.
#[derive(Debug, Clone)]
pub enum LegacyRows {
    /// Replace them with the signed entity's own parts, bytes included.
    Rebuild(Vec<RemoteAttachment>),
    /// List them again. The bundle is opaque S/MIME, a root without siblings,
    /// or does not split, and then no verdict is shown beside them.
    Keep,
}

impl LegacyRows {
    pub fn from_bundle(bundle: &[u8]) -> Self {
        match crate::providers::imap::signed_entity_attachments(bundle) {
            Ok(attachments) => Self::Rebuild(attachments),
            Err(_) => Self::Keep,
        }
    }
}

/// Settle a message's rows from older rules, inside the caller's transaction.
///
/// Rows written since under the current rules are the message's list; then
/// the older rows only go. Parts of decrypted content are neither: they stay
/// as they are and do not count as the list.
pub async fn settle_legacy_rows(
    connection: &mut SqliteConnection,
    message_id: &str,
    rows: &LegacyRows,
) -> Result<(), sqlx::Error> {
    let attachments = match rows {
        LegacyRows::Keep => {
            sqlx::query(
                "UPDATE attachments SET listing_rules = ?
                 WHERE message_id = ? AND listing_rules < ? AND id NOT LIKE ?",
            )
            .bind(LISTING_RULES)
            .bind(message_id)
            .bind(LISTING_RULES)
            .bind(decrypted_attachment_ids())
            .execute(&mut *connection)
            .await?;
            return Ok(());
        }
        LegacyRows::Rebuild(attachments) => attachments,
    };
    let listed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM attachments
                       WHERE message_id = ? AND listing_rules >= ? AND id NOT LIKE ?)",
    )
    .bind(message_id)
    .bind(LISTING_RULES)
    .bind(decrypted_attachment_ids())
    .fetch_one(&mut *connection)
    .await?;
    sqlx::query(
        "DELETE FROM attachments WHERE message_id = ? AND listing_rules < ? AND id NOT LIKE ?",
    )
    .bind(message_id)
    .bind(LISTING_RULES)
    .bind(decrypted_attachment_ids())
    .execute(&mut *connection)
    .await?;
    if !listed {
        insert_entity_rows(&mut *connection, message_id, attachments).await?;
    }
    // The flag counted the rows just replaced, perhaps only the signature
    // part. Like sync, it counts every listed row, inline ones too.
    sqlx::query(
        "UPDATE messages SET has_attachments =
             EXISTS(SELECT 1 FROM attachments WHERE message_id = messages.id)
         WHERE id = ?",
    )
    .bind(message_id)
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "UPDATE threads SET has_attachments =
             (SELECT COALESCE(MAX(has_attachments), 0) FROM messages WHERE thread_id = threads.id)
         WHERE id = (SELECT thread_id FROM messages WHERE id = ?)",
    )
    .bind(message_id)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

fn decrypted_attachment_ids() -> String {
    crate::email::decrypted::attachment_id_pattern()
}

async fn insert_entity_rows(
    connection: &mut SqliteConnection,
    message_id: &str,
    attachments: &[RemoteAttachment],
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    for attachment in attachments {
        // The bytes come from the bundle. No part id goes with them: a fetch
        // by one would read the server's message, not the signed entity.
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, content_type, size, content_id,
                 is_inline, created_at, content_base64, listing_rules)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 filename = excluded.filename, content_type = excluded.content_type,
                 size = excluded.size, content_id = excluded.content_id,
                 is_inline = excluded.is_inline, cached_path = NULL,
                 remote_uid = NULL, remote_part_id = NULL,
                 content_base64 = excluded.content_base64,
                 listing_rules = excluded.listing_rules",
        )
        .bind(format!(
            "att-signed-{message_id}-{}",
            attachment.remote_part_id
        ))
        .bind(message_id)
        .bind(
            attachment
                .filename
                .as_deref()
                .map(crate::utils::filename::strip_filename_controls),
        )
        .bind(&attachment.content_type)
        .bind(attachment.size)
        .bind(attachment.content_id.as_deref())
        .bind(i64::from(attachment.is_inline))
        .bind(&now)
        .bind(attachment.content_base64.as_deref())
        .bind(LISTING_RULES)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

/// Rows from older rules beside a root signature, rebuilt from the bundle.
pub const SIGNED_ENTITY_ATTACHMENTS: DataRepair = DataRepair {
    name: "signed_entity_attachments",
    block: signed_entity_attachments_block,
    prepare: Some(signed_entity_attachments_prepare),
};

/// Messages a block looks at. Few carry hidden rows; those that do hold
/// their bundles in memory until the block has run.
const SCAN_LIMIT: i64 = 200;

/// Bundle bytes a block reads before it ends early, so one large bundle
/// cannot keep a block slow or its memory high. The first bundle is read
/// whatever its size.
const BLOCK_BYTES: usize = 16 * 1024 * 1024;

struct Candidate {
    message_id: String,
    pgp_signed: Option<Vec<u8>>,
    smime_signed: Option<Vec<u8>>,
    rows: LegacyRows,
}

struct Work {
    last: Option<i64>,
    candidates: Vec<Candidate>,
}

async fn candidates(
    connection: &mut SqliteConnection,
    cursor: &str,
    limit: i64,
    budget: usize,
) -> Result<Work, sqlx::Error> {
    let after: i64 = cursor.parse().unwrap_or(0);
    let scanned: Vec<(i64, String, bool, i64)> = sqlx::query_as(&format!(
        "SELECT m.rowid, m.id,
                EXISTS(SELECT 1 FROM attachments a
                       WHERE a.message_id = m.id AND {HIDDEN_BESIDE_ROOT_SIGNATURE}),
                COALESCE(length(m.pgp_signed), 0) + COALESCE(length(m.smime_signed), 0)
         FROM messages m WHERE m.rowid > ? ORDER BY m.rowid LIMIT ?"
    ))
    .bind(after)
    .bind(limit.clamp(1, SCAN_LIMIT))
    .fetch_all(&mut *connection)
    .await?;
    let mut last = None;
    let mut read = 0usize;
    let mut bundles = Vec::new();
    for (rowid, message_id, hidden, size) in scanned {
        if hidden {
            let size = usize::try_from(size).unwrap_or(usize::MAX);
            if !bundles.is_empty() && read.saturating_add(size) > budget {
                break;
            }
            read = read.saturating_add(size);
        }
        last = Some(rowid);
        if !hidden {
            continue;
        }
        let (pgp_signed, smime_signed): (Option<Vec<u8>>, Option<Vec<u8>>) =
            sqlx::query_as("SELECT pgp_signed, smime_signed FROM messages WHERE id = ?")
                .bind(&message_id)
                .fetch_one(&mut *connection)
                .await?;
        bundles.push((message_id, pgp_signed, smime_signed));
    }
    let candidates = off_runtime(move || {
        bundles
            .into_iter()
            .map(|(message_id, pgp_signed, smime_signed)| {
                let rows = pgp_signed
                    .as_deref()
                    .or(smime_signed.as_deref())
                    .map_or(LegacyRows::Keep, LegacyRows::from_bundle);
                Candidate {
                    message_id,
                    pgp_signed,
                    smime_signed,
                    rows,
                }
            })
            .collect()
    })
    .await?;
    Ok(Work { last, candidates })
}

fn signed_entity_attachments_prepare<'c>(
    pool: &'c SqlitePool,
    cursor: &'c str,
    limit: i64,
) -> BoxFuture<'c, Result<Prepared, sqlx::Error>> {
    Box::pin(async move {
        let mut connection = pool.acquire().await?;
        Ok(Box::new(candidates(&mut connection, cursor, limit, BLOCK_BYTES).await?) as Prepared)
    })
}

fn signed_entity_attachments_block<'c>(
    connection: &'c mut SqliteConnection,
    cursor: &'c str,
    limit: i64,
    prepared: Option<Prepared>,
) -> BoxFuture<'c, Result<Option<String>, sqlx::Error>> {
    Box::pin(async move {
        let work = match prepared.and_then(|work| work.downcast::<Work>().ok()) {
            Some(work) => *work,
            None => candidates(&mut *connection, cursor, limit, BLOCK_BYTES).await?,
        };
        let Some(last) = work.last else {
            return Ok(None);
        };
        for candidate in work.candidates {
            // A message synced again since it was read has its rows from
            // that sync, and maybe another bundle.
            let unchanged: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM messages
                     WHERE id = ? AND pgp_signed IS ? AND smime_signed IS ?)",
            )
            .bind(&candidate.message_id)
            .bind(candidate.pgp_signed.as_deref())
            .bind(candidate.smime_signed.as_deref())
            .fetch_one(&mut *connection)
            .await?;
            if unchanged {
                settle_legacy_rows(&mut *connection, &candidate.message_id, &candidate.rows)
                    .await?;
            }
        }
        Ok(Some(last.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::data_repairs::run_repair;

    const SIGNED_WITH_ATTACHMENT: &[u8] =
        include_bytes!("../../tests/corpus/pgp-mime/thunderbird-mixed-attachment.eml");

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('a', 'Account', 'me@example.test', 'imap', '{}', '', '', '');
            INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
                VALUES ('f', 'a', 'Inbox', 'INBOX', 'inbox', '', '');")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn message(pool: &SqlitePool, id: &str, pgp: Option<&[u8]>, smime: Option<&[u8]>) {
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date,
                 trust_metadata, pgp_signed, smime_signed, created_at, updated_at)
             VALUES (?, 'a', 'f', 'sender@example.test', '[]', '', '{\"root_crypto\":true}', ?, ?, '', '')",
        )
        .bind(id)
        .bind(pgp)
        .bind(smime)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn attachment(pool: &SqlitePool, id: &str, message: &str, rules: i64) {
        sqlx::query(
            "INSERT INTO attachments (id, message_id, filename, content_type, listing_rules, created_at)
             VALUES (?, ?, ?, 'application/pdf', ?, '')",
        )
        .bind(id)
        .bind(message)
        .bind(format!("{id}.pdf"))
        .bind(rules)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn rows(pool: &SqlitePool, message: &str) -> Vec<(String, i64)> {
        sqlx::query_as("SELECT id, listing_rules FROM attachments WHERE message_id = ? ORDER BY id")
            .bind(message)
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// The signed entity's parts replace the older rows. An opaque bundle, a
    /// root without siblings, and one that does not split, beside which no
    /// verdict is shown, keep theirs.
    #[tokio::test]
    async fn the_repair_rebuilds_older_rows_from_the_bundle() {
        let bundle = crate::providers::imap::parse_raw_message_body(SIGNED_WITH_ATTACHMENT)
            .pgp_signed
            .unwrap();
        let malformed = b"Content-Type: multipart/signed; boundary=x\r\n\r\n--x--\r\n";
        let pool = pool().await;
        message(&pool, "signed", Some(&bundle), None).await;
        message(&pool, "opaque", None, Some(b"\x30\x82\x01\x00signed-data")).await;
        message(&pool, "malformed", Some(malformed), None).await;
        message(&pool, "resynced", Some(&bundle), None).await;
        attachment(&pool, "outer", "signed", 0).await;
        attachment(&pool, "p7m", "opaque", 0).await;
        attachment(&pool, "kept", "malformed", 0).await;
        attachment(&pool, "older", "resynced", 0).await;
        attachment(&pool, "synced", "resynced", 1).await;

        run_repair(&pool, &SIGNED_ENTITY_ATTACHMENTS, 2, None, |_| {})
            .await
            .unwrap();

        assert_eq!(
            rows(&pool, "signed").await,
            [("att-signed-signed-mime-1".to_string(), 1)]
        );
        let (name, inline, part, payload): (String, i64, Option<String>, String) = sqlx::query_as(
            "SELECT filename, is_inline, remote_part_id, content_base64 FROM attachments WHERE message_id = 'signed'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((name.as_str(), inline, part), ("figures.csv", 0, None));
        assert!(!payload.is_empty());
        assert_eq!(rows(&pool, "opaque").await, [("p7m".to_string(), 1)]);
        assert_eq!(rows(&pool, "malformed").await, [("kept".to_string(), 1)]);
        assert_eq!(rows(&pool, "resynced").await, [("synced".to_string(), 1)]);
        assert!(
            !crate::db::data_repairs::is_pending(&pool, &SIGNED_ENTITY_ATTACHMENTS)
                .await
                .unwrap()
        );
    }

    /// Parts of decrypted content are the decrypted body's, written again
    /// each time the message is decrypted. The repair neither rewrites them
    /// nor counts them as the signed entity's list. Beside a root signature
    /// they are marked for the listing, which offers them only with the
    /// decrypted body.
    #[tokio::test]
    async fn decrypted_parts_stay_outside_the_signed_entity_rules() {
        assert_eq!(
            crate::email::decrypted::attachment_id_pattern(),
            "att-pgp-%",
            "the listing conditions spell the pattern out"
        );
        let bundle = crate::providers::imap::parse_raw_message_body(SIGNED_WITH_ATTACHMENT)
            .pgp_signed
            .unwrap();
        let pool = pool().await;
        message(&pool, "signed", Some(&bundle), None).await;
        let fresh = crate::email::decrypted::attachment_id("signed", "mime-1");
        let older = crate::email::decrypted::attachment_id("signed", "mime-2");
        attachment(&pool, "outer", "signed", 0).await;
        attachment(&pool, &fresh, "signed", LISTING_RULES).await;
        attachment(&pool, &older, "signed", 0).await;

        let hidden: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT a.id FROM attachments a JOIN messages m ON m.id = a.message_id
             WHERE {HIDDEN_BESIDE_ROOT_SIGNATURE} ORDER BY a.id"
        ))
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(hidden, ["outer"]);
        message(&pool, "unsigned", None, None).await;
        let unsigned = crate::email::decrypted::attachment_id("unsigned", "mime-1");
        attachment(&pool, &unsigned, "unsigned", LISTING_RULES).await;
        let decrypted: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT a.id FROM attachments a JOIN messages m ON m.id = a.message_id
             WHERE {DECRYPTED_BESIDE_ROOT_SIGNATURE} ORDER BY a.id"
        ))
        .fetch_all(&pool)
        .await
        .unwrap();
        let mut expected_decrypted = vec![fresh.clone(), older.clone()];
        expected_decrypted.sort();
        assert_eq!(decrypted, expected_decrypted);

        run_repair(&pool, &SIGNED_ENTITY_ATTACHMENTS, 10, None, |_| {})
            .await
            .unwrap();

        let mut expected = vec![
            ("att-signed-signed-mime-1".to_string(), LISTING_RULES),
            (fresh, LISTING_RULES),
            (older, 0),
        ];
        expected.sort();
        assert_eq!(rows(&pool, "signed").await, expected);
    }

    async fn threaded(pool: &SqlitePool, message: &str, thread: &str, has_attachments: i64) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, last_message_date, has_attachments, created_at, updated_at)
             VALUES (?, 'a', '', ?, '', '')",
        )
        .bind(thread)
        .bind(has_attachments)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE messages SET thread_id = ?, has_attachments = ? WHERE id = ?")
            .bind(thread)
            .bind(has_attachments)
            .bind(message)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn flags(pool: &SqlitePool, message: &str) -> (i64, i64) {
        sqlx::query_as(
            "SELECT m.has_attachments, t.has_attachments
             FROM messages m JOIN threads t ON t.id = m.thread_id WHERE m.id = ?",
        )
        .bind(message)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// A rebuild sets the message's and its thread's attachment flag from the
    /// rows it leaves: the older ones may have been only the signature part.
    #[tokio::test]
    async fn the_repair_sets_the_attachment_flag_from_the_rows_it_leaves() {
        let with_file = crate::providers::imap::parse_raw_message_body(SIGNED_WITH_ATTACHMENT)
            .pgp_signed
            .unwrap();
        let without = crate::providers::imap::parse_raw_message_body(include_bytes!(
            "../../tests/corpus/pgp-mime/forged-signature-part-attachment.eml"
        ))
        .pgp_signed
        .unwrap();
        let pool = pool().await;
        message(&pool, "with-file", Some(&with_file), None).await;
        message(&pool, "without", Some(&without), None).await;
        threaded(&pool, "with-file", "t-with-file", 0).await;
        threaded(&pool, "without", "t-without", 1).await;
        attachment(&pool, "outer", "with-file", 0).await;
        attachment(&pool, "signature", "without", 0).await;

        run_repair(&pool, &SIGNED_ENTITY_ATTACHMENTS, 10, None, |_| {})
            .await
            .unwrap();

        assert_eq!(rows(&pool, "without").await, []);
        assert_eq!(flags(&pool, "with-file").await, (1, 1));
        assert_eq!(flags(&pool, "without").await, (0, 0));
    }

    /// A block reads bundles up to its byte budget, at least one, and its
    /// cursor stops at the last message it read.
    #[tokio::test]
    async fn a_block_stops_at_its_byte_budget() {
        let bundle = crate::providers::imap::parse_raw_message_body(SIGNED_WITH_ATTACHMENT)
            .pgp_signed
            .unwrap();
        let pool = pool().await;
        for id in ["first", "second", "third"] {
            message(&pool, id, Some(&bundle), None).await;
            attachment(&pool, &format!("{id}-row"), id, 0).await;
        }
        let rowids: Vec<i64> = sqlx::query_scalar("SELECT rowid FROM messages ORDER BY rowid")
            .fetch_all(&pool)
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();

        let work = candidates(&mut connection, "", 10, 1).await.unwrap();
        assert_eq!(work.candidates.len(), 1);
        assert_eq!(work.candidates[0].message_id, "first");
        assert_eq!(work.last, Some(rowids[0]));

        let work = candidates(&mut connection, "", 10, bundle.len() * 2)
            .await
            .unwrap();
        let read: Vec<&str> = work
            .candidates
            .iter()
            .map(|candidate| candidate.message_id.as_str())
            .collect();
        assert_eq!(read, ["first", "second"]);
        assert_eq!(work.last, Some(rowids[1]));
    }

    /// A message synced again after its block was read keeps what that sync
    /// wrote: its bundle is not the one the rebuild was derived from.
    #[tokio::test]
    async fn a_bundle_replaced_since_it_was_read_is_left_to_its_sync() {
        let bundle = crate::providers::imap::parse_raw_message_body(SIGNED_WITH_ATTACHMENT)
            .pgp_signed
            .unwrap();
        let pool = pool().await;
        message(&pool, "signed", Some(&bundle), None).await;
        attachment(&pool, "outer", "signed", 0).await;
        let prepare = SIGNED_ENTITY_ATTACHMENTS.prepare.unwrap();
        let prepared = prepare(&pool, "", 10).await.unwrap();
        sqlx::query("UPDATE messages SET pgp_signed = CAST('Content-Type: text/plain' AS BLOB)")
            .execute(&pool)
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        (SIGNED_ENTITY_ATTACHMENTS.block)(&mut connection, "", 10, Some(prepared))
            .await
            .unwrap();
        drop(connection);
        assert_eq!(rows(&pool, "signed").await, [("outer".to_string(), 0)]);
    }
}
