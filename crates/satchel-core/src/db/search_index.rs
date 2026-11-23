//! Verify the indexes and row mappings used by message search.

use sqlx::{Connection, Executor, SqliteConnection, SqlitePool};

#[derive(Debug)]
struct SearchIndexHealth {
    text: bool,
    projection: bool,
    cc: bool,
}

impl SearchIndexHealth {
    fn intact(&self) -> bool {
        self.text && self.projection && self.cc
    }
}

/// Check the active search indexes against a single snapshot of the messages.
/// FTS5's integrity commands require a writable database connection.
pub async fn message_index_intact(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let health = check_writable(&mut tx).await?;
    tx.rollback().await?;
    Ok(health.intact())
}

/// Check a backup without writing to its database. The text index is checked by
/// document count here. The writable restore check also verifies its terms.
pub async fn backup_search_index_intact(
    pool: &SqlitePool,
    schema_version: i64,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let (text, projection, cc): (bool, bool, bool) = sqlx::query_as(
        "SELECT
            EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'messages_fts_docsize'),
            EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'message_search_projection'),
            EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'message_cc_fts')",
    )
    .fetch_one(&mut *tx)
    .await?;
    // Older backups legitimately predate the projection (0053) or Cc index
    // (0076). A missing table after its migration is search damage.
    let text_ok = if text {
        text_document_count_matches(&mut tx).await?
    } else {
        schema_version == 0
    };
    let projection_ok = if projection {
        projection_matches(&mut tx).await?
    } else {
        schema_version < 53
    };
    let cc_ok = if cc {
        cc_matches(&mut tx).await?
    } else {
        schema_version < 76
    };
    tx.rollback().await?;
    Ok(text_ok && projection_ok && cc_ok)
}

/// Rebuild only the inconsistent components, then verify the repaired index
/// before committing. Sync and the prefix backfill cannot change the source
/// rows or swap the active text index during the check and repair.
pub async fn verify_and_repair_message_index(pool: &SqlitePool) -> Result<bool, String> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|err| err.to_string())?;
    let health = check_writable(&mut tx)
        .await
        .map_err(|err| err.to_string())?;
    if health.intact() {
        tx.rollback().await.map_err(|err| err.to_string())?;
        return Ok(false);
    }

    tracing::warn!(
        ?health,
        "message search index failed its integrity check; rebuilding"
    );
    rebuild(&mut tx, &health)
        .await
        .map_err(|err| err.to_string())?;
    if !check_writable(&mut tx)
        .await
        .map_err(|err| err.to_string())?
        .intact()
    {
        return Err("message search index is still inconsistent after a rebuild".into());
    }
    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(true)
}

async fn check_writable(conn: &mut SqliteConnection) -> Result<SearchIndexHealth, sqlx::Error> {
    // rank = 1 compares the external-content text index against messages.
    let text = fts_integrity_result(
        sqlx::query("INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)")
            .execute(&mut *conn)
            .await,
    )? && text_document_count_matches(conn).await?;

    let projection = projection_matches(conn).await?;

    // The contentless Cc index only gets a structural check from FTS5.
    let cc = fts_integrity_result(
        sqlx::query("INSERT INTO message_cc_fts(message_cc_fts) VALUES('integrity-check')")
            .execute(&mut *conn)
            .await,
    )? && cc_matches(conn).await?;

    Ok(SearchIndexHealth {
        text,
        projection,
        cc,
    })
}

fn fts_integrity_result<T>(result: Result<T, sqlx::Error>) -> Result<bool, sqlx::Error> {
    match result {
        Ok(_) => Ok(true),
        // SQLITE_CORRUPT (11) includes SQLITE_CORRUPT_VTAB (267). Busy,
        // read-only and I/O failures must stay retryable.
        Err(sqlx::Error::Database(err))
            if err
                .code()
                .and_then(|code| code.parse::<i32>().ok())
                .is_some_and(|code| code & 0xff == 11) =>
        {
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

async fn text_document_count_matches(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM messages_fts_docsize) = (SELECT COUNT(*) FROM messages)",
    )
    .fetch_one(conn)
    .await
}

async fn projection_matches(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT NOT EXISTS (
             SELECT 1 FROM messages AS m
             LEFT JOIN message_search_projection AS p ON p.message_rowid = m.rowid
             WHERE p.message_rowid IS NULL
                OR p.message_id IS NOT m.id
                OR p.thread_id IS NOT m.thread_id
                OR p.account_id IS NOT m.account_id
                OR p.date IS NOT COALESCE(m.date, '')
         ) AND NOT EXISTS (
             SELECT 1 FROM message_search_projection AS p
             WHERE NOT EXISTS (SELECT 1 FROM messages AS m WHERE m.rowid = p.message_rowid)
         )",
    )
    .fetch_one(conn)
    .await
}

async fn cc_matches(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    // The document map also includes messages with no Cc tokens. Check the
    // exact rowids to catch missing and extra documents even at equal counts.
    let same_rows: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (
             SELECT 1 FROM messages AS m
             WHERE NOT EXISTS (SELECT 1 FROM message_cc_fts_docsize AS c WHERE c.id = m.rowid)
         ) AND NOT EXISTS (
             SELECT 1 FROM message_cc_fts_docsize AS c
             WHERE NOT EXISTS (SELECT 1 FROM messages AS m WHERE m.rowid = c.id)
         )",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !same_rows {
        return Ok(false);
    }

    // This nested transaction writes only to temp, so it is safe even with a
    // read-only main database. Rolling it back drops the scratch tables.
    let mut scratch = conn.begin().await?;
    let same_terms = temp_cc_postings_match(&mut scratch).await;
    scratch.rollback().await?;
    same_terms
}

async fn temp_cc_postings_match(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    // Use the same FTS5 tokenizer as migration 0076 and compare postings,
    // including positions used by phrase queries.
    conn
        .execute(sqlx::raw_sql(
            "CREATE VIRTUAL TABLE temp.expected_message_cc USING fts5(
                 cc_addresses, content = '', tokenize = 'unicode61 remove_diacritics 2'
             );
             INSERT INTO temp.expected_message_cc (rowid, cc_addresses)
                 SELECT rowid, cc_addresses FROM main.messages
                 WHERE cc_addresses IS NOT NULL AND cc_addresses <> '[]';
             CREATE VIRTUAL TABLE temp.expected_cc_terms USING fts5vocab(temp, expected_message_cc, instance);
             CREATE VIRTUAL TABLE temp.actual_cc_terms USING fts5vocab(main, message_cc_fts, instance);",
        ))
        .await?;
    sqlx::query_scalar(
        "SELECT NOT EXISTS (
             SELECT term, doc, offset FROM temp.expected_cc_terms
             EXCEPT
             SELECT term, doc, offset FROM temp.actual_cc_terms
         ) AND NOT EXISTS (
             SELECT term, doc, offset FROM temp.actual_cc_terms
             EXCEPT
             SELECT term, doc, offset FROM temp.expected_cc_terms
         )",
    )
    .fetch_one(conn)
    .await
}

async fn rebuild(
    conn: &mut SqliteConnection,
    health: &SearchIndexHealth,
) -> Result<(), sqlx::Error> {
    if !health.text {
        sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")
            .execute(&mut *conn)
            .await?;
    }
    if !health.projection {
        conn.execute(sqlx::raw_sql(
            "DELETE FROM message_search_projection;
             INSERT INTO message_search_projection
                 (message_rowid, message_id, thread_id, account_id, date)
             SELECT rowid, id, thread_id, account_id, COALESCE(date, '') FROM messages;",
        ))
        .await?;
    }
    if !health.cc {
        conn.execute(sqlx::raw_sql(
            "INSERT INTO message_cc_fts(message_cc_fts) VALUES('delete-all');
             INSERT INTO message_cc_fts (rowid, cc_addresses)
             SELECT rowid, cc_addresses FROM messages;",
        ))
        .await?;
    }
    Ok(())
}
