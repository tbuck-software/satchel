//! The model in SQLite: token counts, class totals, the training ledger and
//! verdicts. Every function takes a connection so callers compose it with the
//! write transaction that owns the user's action.

use sqlx::{QueryBuilder, Sqlite, SqliteConnection};

use super::classifier::{self, ModelTotals, Verdict};
use super::tokenizer::{decode_tokens, encode_tokens, tokenize, SpamFeatures};

/// The model keeps at most this many tokens; pruning brings it back to
/// [`PRUNE_TARGET`]. A token seen once in one message a year ago is the first
/// to go.
pub const MAX_MODEL_TOKENS: i64 = 200_000;
pub const PRUNE_TARGET: i64 = 160_000;

/// Rows per multi-row statement: three binds each, well under SQLite's limit.
const CHUNK: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Spam,
    Ham,
}

impl Label {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Ham => "ham",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "spam" => Some(Self::Spam),
            "ham" => Some(Self::Ham),
            _ => None,
        }
    }
}

/// How sure the signal behind a training call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// The user said so: Spam, Not spam, a move out of Spam. Replaces any
    /// earlier label of the message.
    Explicit,
    /// Starring or answering. Teaches ham only about a message nobody has
    /// labelled yet, and never overrides a label.
    Weak,
}

/// What the training ledger holds about one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainedLabel {
    pub label: Label,
    pub previous_label: Option<Label>,
    pub receipt_id: Option<String>,
}

pub async fn totals(conn: &mut SqliteConnection) -> Result<ModelTotals, sqlx::Error> {
    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT spam_messages, ham_messages FROM spam_model WHERE id = 1")
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row
        .map(|(spam, ham)| ModelTotals { spam, ham })
        .unwrap_or_default())
}

pub async fn token_count(conn: &mut SqliteConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM spam_tokens")
        .fetch_one(&mut *conn)
        .await
}

async fn adjust_totals(
    conn: &mut SqliteConnection,
    label: Label,
    delta: i64,
    now: &str,
) -> Result<(), sqlx::Error> {
    let (spam, ham) = match label {
        Label::Spam => (delta, 0),
        Label::Ham => (0, delta),
    };
    sqlx::query(
        "INSERT INTO spam_model (id, spam_messages, ham_messages, updated_at)
         VALUES (1, MAX(0, ?), MAX(0, ?), ?)
         ON CONFLICT(id) DO UPDATE SET
            spam_messages = MAX(0, spam_messages + ?),
            ham_messages = MAX(0, ham_messages + ?),
            updated_at = excluded.updated_at",
    )
    .bind(spam)
    .bind(ham)
    .bind(now)
    .bind(spam)
    .bind(ham)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Add one message's tokens to a class.
async fn add_tokens(
    conn: &mut SqliteConnection,
    tokens: &[i64],
    label: Label,
    now: &str,
) -> Result<(), sqlx::Error> {
    let (spam, ham) = match label {
        Label::Spam => (1_i64, 0_i64),
        Label::Ham => (0, 1),
    };
    for chunk in tokens.chunks(CHUNK) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO spam_tokens (token, spam_count, ham_count, last_seen) ",
        );
        query.push_values(chunk, |mut row, token| {
            row.push_bind(*token)
                .push_bind(spam)
                .push_bind(ham)
                .push_bind(now);
        });
        query.push(
            " ON CONFLICT(token) DO UPDATE SET
                spam_count = spam_tokens.spam_count + excluded.spam_count,
                ham_count = spam_tokens.ham_count + excluded.ham_count,
                last_seen = excluded.last_seen",
        );
        query.build().execute(&mut *conn).await?;
    }
    adjust_totals(conn, label, 1, now).await
}

/// Take one message's tokens back out of a class. Counts never go below zero:
/// a token pruned since has nothing left to subtract.
async fn remove_tokens(
    conn: &mut SqliteConnection,
    tokens: &[i64],
    label: Label,
    now: &str,
) -> Result<(), sqlx::Error> {
    let column = match label {
        Label::Spam => "spam_count",
        Label::Ham => "ham_count",
    };
    for chunk in tokens.chunks(CHUNK) {
        let mut query = QueryBuilder::<Sqlite>::new(format!(
            "UPDATE spam_tokens SET {column} = MAX(0, {column} - 1) WHERE token IN ("
        ));
        let mut separated = query.separated(", ");
        for token in chunk {
            separated.push_bind(*token);
        }
        query.push(")");
        query.build().execute(&mut *conn).await?;

        let mut query = QueryBuilder::<Sqlite>::new(
            "DELETE FROM spam_tokens WHERE spam_count = 0 AND ham_count = 0 AND token IN (",
        );
        let mut separated = query.separated(", ");
        for token in chunk {
            separated.push_bind(*token);
        }
        query.push(")");
        query.build().execute(&mut *conn).await?;
    }
    adjust_totals(conn, label, -1, now).await
}

/// The stored columns of one message as tokenizer input.
pub async fn features_for_message(
    conn: &mut SqliteConnection,
    message_id: &str,
) -> Result<Option<SpamFeatures>, sqlx::Error> {
    let row: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT account_id, COALESCE(from_address, ''), from_name, reply_to, subject,
                body_text, body_html,
                COALESCE(json_array_length(CASE WHEN json_valid(to_addresses) THEN to_addresses END), 0)
                  + COALESCE(json_array_length(CASE WHEN json_valid(cc_addresses) THEN cc_addresses END), 0),
                COALESCE(has_attachments, 0),
                CASE WHEN TRIM(COALESCE(list_unsubscribe, '')) <> '' THEN 1 ELSE 0 END,
                auth_spf, auth_dkim, auth_dmarc,
                (SELECT hint_headers FROM spam_verdicts WHERE spam_verdicts.message_id = messages.id)
         FROM messages WHERE id = ?",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(
        |(
            account_id,
            from_address,
            from_name,
            reply_to,
            subject,
            body_text,
            body_html,
            recipients,
            attachments,
            unsubscribe,
            auth_spf,
            auth_dkim,
            auth_dmarc,
            hint_headers,
        )| SpamFeatures {
            account_id,
            from_address,
            from_name,
            reply_to,
            subject: subject.unwrap_or_default(),
            body_text,
            body_html,
            recipient_count: recipients.max(0) as usize,
            has_attachments: attachments != 0,
            has_list_unsubscribe: unsubscribe != 0,
            auth_spf,
            auth_dkim,
            auth_dmarc,
            hint_headers,
        },
    ))
}

/// The tokens a message is trained with: its stored columns plus the headers
/// kept when it was classified, so training sees what classification saw.
pub async fn message_tokens(
    conn: &mut SqliteConnection,
    message_id: &str,
) -> Result<Option<Vec<i64>>, sqlx::Error> {
    Ok(features_for_message(conn, message_id)
        .await?
        .map(|features| tokenize(&features)))
}

pub async fn trained_label(
    conn: &mut SqliteConnection,
    message_id: &str,
) -> Result<Option<TrainedLabel>, sqlx::Error> {
    let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT label, previous_label, receipt_id FROM spam_training WHERE message_id = ?",
    )
    .bind(message_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.and_then(|(label, previous, receipt_id)| {
        Some(TrainedLabel {
            label: Label::parse(&label)?,
            previous_label: previous.as_deref().and_then(Label::parse),
            receipt_id,
        })
    }))
}

/// Train one message. Returns whether the model changed.
///
/// An explicit label replaces an earlier one and remembers it, so undoing
/// `receipt_id` restores exactly the state before. Training the label a
/// message already has changes nothing — counting it twice would make the
/// user's tenth click on Spam worth ten messages.
pub async fn train_message(
    conn: &mut SqliteConnection,
    message_id: &str,
    label: Label,
    signal: Signal,
    receipt_id: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let existing: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT label, tokens FROM spam_training WHERE message_id = ?")
            .bind(message_id)
            .fetch_optional(&mut *conn)
            .await?;
    match existing {
        None => {
            let Some(tokens) = message_tokens(conn, message_id).await? else {
                return Ok(false);
            };
            if tokens.is_empty() {
                return Ok(false);
            }
            add_tokens(conn, &tokens, label, &now).await?;
            sqlx::query(
                "INSERT INTO spam_training (message_id, label, previous_label, receipt_id, tokens, trained_at)
                 VALUES (?, ?, NULL, ?, ?, ?)",
            )
            .bind(message_id)
            .bind(label.as_str())
            .bind(receipt_id)
            .bind(encode_tokens(&tokens))
            .bind(&now)
            .execute(&mut *conn)
            .await?;
            Ok(true)
        }
        Some(_) if signal == Signal::Weak => Ok(false),
        Some((previous, _)) if previous == label.as_str() => Ok(false),
        Some((previous, blob)) => {
            let Some(previous) = Label::parse(&previous) else {
                return Ok(false);
            };
            let tokens = decode_tokens(&blob);
            remove_tokens(conn, &tokens, previous, &now).await?;
            add_tokens(conn, &tokens, label, &now).await?;
            sqlx::query(
                "UPDATE spam_training SET label = ?, previous_label = ?, receipt_id = ?, trained_at = ?
                 WHERE message_id = ?",
            )
            .bind(label.as_str())
            .bind(previous.as_str())
            .bind(receipt_id)
            .bind(&now)
            .bind(message_id)
            .execute(&mut *conn)
            .await?;
            Ok(true)
        }
    }
}

/// Reverse every training that `receipt_id` caused. Returns how many messages
/// changed. Running it twice is a no-op: the rows lose the receipt.
pub async fn untrain_receipt(
    conn: &mut SqliteConnection,
    receipt_id: &str,
) -> Result<usize, sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<(String, String, Option<String>, Vec<u8>)> = sqlx::query_as(
        "SELECT message_id, label, previous_label, tokens FROM spam_training WHERE receipt_id = ?",
    )
    .bind(receipt_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut changed = 0;
    for (message_id, label, previous, blob) in rows {
        let tokens = decode_tokens(&blob);
        if let Some(label) = Label::parse(&label) {
            remove_tokens(conn, &tokens, label, &now).await?;
        }
        match previous.as_deref().and_then(Label::parse) {
            Some(previous) => {
                add_tokens(conn, &tokens, previous, &now).await?;
                sqlx::query(
                    "UPDATE spam_training SET label = ?, previous_label = NULL, receipt_id = NULL, trained_at = ?
                     WHERE message_id = ?",
                )
                .bind(previous.as_str())
                .bind(&now)
                .bind(&message_id)
                .execute(&mut *conn)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM spam_training WHERE message_id = ?")
                    .bind(&message_id)
                    .execute(&mut *conn)
                    .await?;
            }
        }
        changed += 1;
    }
    Ok(changed)
}

/// Score tokens against the model. `None` until the model has seen enough of
/// both classes to say anything.
pub async fn score_tokens(
    conn: &mut SqliteConnection,
    tokens: &[i64],
) -> Result<Option<(f64, Verdict)>, sqlx::Error> {
    let totals = totals(conn).await?;
    if !totals.ready() {
        return Ok(None);
    }
    let mut known = std::collections::HashMap::with_capacity(tokens.len());
    for chunk in tokens.chunks(500) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT token, spam_count, ham_count FROM spam_tokens WHERE token IN (",
        );
        let mut separated = query.separated(", ");
        for token in chunk {
            separated.push_bind(*token);
        }
        query.push(")");
        let rows: Vec<(i64, i64, i64)> = query.build_query_as().fetch_all(&mut *conn).await?;
        for (token, spam, ham) in rows {
            known.insert(token, (spam, ham));
        }
    }
    let counts: Vec<(i64, i64)> = tokens
        .iter()
        .map(|token| known.get(token).copied().unwrap_or((0, 0)))
        .collect();
    let score = classifier::score(&counts, totals);
    Ok(Some((score, Verdict::from_score(score))))
}

/// Record what classification said. `None` keeps the hint headers without a
/// score: the model was not ready, or the sender is protected.
pub async fn store_verdict(
    conn: &mut SqliteConnection,
    message_id: &str,
    scored: Option<(f64, Verdict)>,
    hint_headers: Option<&str>,
) -> Result<(), sqlx::Error> {
    let (score, verdict) = scored
        .map(|(score, verdict)| (score, verdict.as_str()))
        .unwrap_or((0.5, "none"));
    sqlx::query(
        "INSERT INTO spam_verdicts (message_id, score, verdict, hint_headers, receipt_id, classified_at)
         VALUES (?, ?, ?, ?, NULL, ?)
         ON CONFLICT(message_id) DO UPDATE SET
            score = excluded.score, verdict = excluded.verdict,
            hint_headers = COALESCE(excluded.hint_headers, spam_verdicts.hint_headers),
            classified_at = excluded.classified_at",
    )
    .bind(message_id)
    .bind(score)
    .bind(verdict)
    .bind(hint_headers)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Drop the weakest tokens once the model outgrows [`MAX_MODEL_TOKENS`]:
/// fewest sightings first, then least recently seen. Returns how many went.
pub async fn prune(
    conn: &mut SqliteConnection,
    max_tokens: i64,
    target: i64,
) -> Result<u64, sqlx::Error> {
    let count = token_count(conn).await?;
    if count <= max_tokens {
        return Ok(0);
    }
    let excess = count - target.min(max_tokens).max(0);
    let deleted = sqlx::query(
        "DELETE FROM spam_tokens WHERE token IN (
            SELECT token FROM spam_tokens
            ORDER BY spam_count + ham_count ASC, last_seen ASC, token ASC
            LIMIT ?
         )",
    )
    .bind(excess)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(deleted)
}

/// An IMAP move gives a message a new local id. Carry its verdict and ledger
/// row along, so an Undo or a later relabel still finds them.
pub async fn remap_message(
    conn: &mut SqliteConnection,
    source_id: &str,
    target_id: &str,
) -> Result<(), sqlx::Error> {
    for table in ["spam_verdicts", "spam_training"] {
        sqlx::query(&format!(
            "UPDATE OR IGNORE {table} SET message_id = ? WHERE message_id = ?"
        ))
        .bind(target_id)
        .bind(source_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
