//! A rule's forward of one message, between the rule run and the outbox.
//!
//! The rule run decides *whether* a message is forwarded and records that in
//! `rule_forwards` (see `satchel_core::rules`). This step, run by the queue
//! worker, fetches the original, checks it may be forwarded at all and hands
//! the forward to the outbox as an ordinary send:
//!
//! - The original travels as a `message/rfc822` attachment, byte for byte: a
//!   forward that keeps its headers, signatures and every MIME part. The new
//!   message is from the account the mail arrived in, subject `Fwd: …`, with
//!   no text of its own, and carries `Auto-Submitted: auto-generated`
//!   (RFC 3834) so the other side neither answers nor forwards it again.
//! - Mail that was itself sent automatically, a bounce or report, list, bulk
//!   or spam mail, mail from the target itself, and anything over
//!   [`satchel_core::rules::MAX_FORWARD_BYTES`] is not forwarded
//!   ([`satchel_core::rules::forward_loop_guard`]). That needs the
//!   original's headers, which is why the check runs here and not in the
//!   rule run: Satchel does not keep them.
//! - The send has a Message-ID and queue id derived from rule, message and
//!   address. Running this step twice (a crash between the two writes) queues
//!   the same send, which the queue keeps once and the outbox sends once.

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::sync::queue::{QueuedAction, SyncQueue};
use satchel_core::drafts::outgoing::OutgoingMessageInput;
use satchel_core::drafts::send_queue::SendActionPayload;
use satchel_core::sync::errors::SyncResult;
use satchel_core::sync::queue::ACTION_SEND_MESSAGE;

/// The original is read only this far to find its header block.
const HEADER_SCAN_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuleForwardPayload {
    pub rule_id: String,
    pub rule_token: String,
    /// Local message id at the time of the rule run.
    pub message_id: String,
    /// The `rule_forwards` key: Message-ID header, or the local id.
    pub message_key: String,
    pub to: String,
}

impl RuleForwardPayload {
    fn digest(&self) -> String {
        let digest = Sha256::digest(
            format!("{}\0{}\0{}", self.rule_token, self.message_key, self.to).as_bytes(),
        );
        digest
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// One queue row per rule, message and address.
    pub fn queue_id(&self) -> String {
        format!("rule-forward-{}", self.digest())
    }

    fn send_id(&self) -> String {
        format!("rule-forward-send-{}", self.digest())
    }

    fn message_id_header(&self) -> String {
        format!("<rule-forward-{}@satchel.local>", self.digest())
    }
}

/// What became of a forward step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardStep {
    /// Handed to the outbox under this queue id.
    Queued(String),
    /// Not forwarded; the reason is recorded as the outcome.
    Skipped(&'static str),
}

pub async fn execute_rule_forward(
    pool: &SqlitePool,
    queue: &SyncQueue,
    action: &QueuedAction,
) -> SyncResult<()> {
    execute_rule_forward_with(pool, queue, action, |message_id| async move {
        crate::commands::messages::fetch_remote_message_source(pool, pool, &message_id).await
    })
    .await
    .map(|_| ())
}

/// [`execute_rule_forward`] with the original's source supplied by `source`.
pub async fn execute_rule_forward_with<F, Fut>(
    pool: &SqlitePool,
    queue: &SyncQueue,
    action: &QueuedAction,
    source: F,
) -> SyncResult<ForwardStep>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<Option<Vec<u8>>, String>>,
{
    let payload: RuleForwardPayload = serde_json::from_value(action.payload.clone())
        .map_err(|error| format!("decode rule forward payload: {error}"))?;
    let outcome: Option<String> = sqlx::query_scalar(
        "SELECT outcome FROM rule_forwards WHERE rule_token = ? AND message_key = ?",
    )
    .bind(&payload.rule_token)
    .bind(&payload.message_key)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    // Only a forward the rule run queued is sent; anything else was decided
    // otherwise (or already handed to the outbox).
    if outcome.as_deref() != Some("queued") {
        return Ok(ForwardStep::Skipped("not_queued"));
    }
    let message: Option<(Option<String>, String, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT m.subject, m.account_id, m.raw_size, f.folder_type
         FROM messages m LEFT JOIN folders f ON f.id = m.folder_id WHERE m.id = ?",
    )
    .bind(&payload.message_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    let Some((subject, account_id, raw_size, folder_type)) = message else {
        return skip(pool, &payload, "skipped_missing").await;
    };
    // Moved to Spam or Trash since the rule ran: not forwarded after all.
    if matches!(folder_type.as_deref(), Some("spam" | "trash")) {
        return skip(pool, &payload, "skipped_spam").await;
    }
    let too_large = |size: usize| size > satchel_core::rules::MAX_FORWARD_BYTES;
    if raw_size
        .and_then(|size| usize::try_from(size).ok())
        .is_some_and(too_large)
    {
        return skip(pool, &payload, "skipped_too_large").await;
    }
    let raw = match source(payload.message_id.clone()).await? {
        Some(raw) => raw,
        // No original to forward (a local import without its source): it
        // would not come back on a retry either.
        None => return skip(pool, &payload, "skipped_no_source").await,
    };
    if too_large(raw.len()) {
        return skip(pool, &payload, "skipped_too_large").await;
    }
    let head = String::from_utf8_lossy(&raw[..raw.len().min(HEADER_SCAN_BYTES)]);
    if let Some(reason) = satchel_core::rules::forward_loop_guard(&head, &payload.to) {
        tracing::info!(rule_id = %payload.rule_id, reason, "rule forward: loop guard");
        return skip(pool, &payload, "skipped_automatic").await;
    }
    // The breaker again, here: a burst queued at once passes the rule run's
    // check before any of it reached the outbox. Paused meanwhile: not sent.
    let paused: Option<Option<String>> =
        sqlx::query_scalar("SELECT forward_paused_at FROM rules WHERE id = ?")
            .bind(&payload.rule_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?;
    if matches!(paused, Some(Some(_))) {
        return skip(pool, &payload, "skipped_paused").await;
    }
    let now = chrono::Utc::now();
    if crate::commands::rules::forward_limit_reached(pool, &payload.rule_id, now).await? {
        crate::commands::rules::pause_forwarding(pool, &payload.rule_id, now).await?;
        return skip(pool, &payload, "skipped_rate_limit").await;
    }

    let subject = subject.unwrap_or_default();
    let input = OutgoingMessageInput {
        to: vec![payload.to.clone()],
        subject: forward_subject(&subject),
        body: String::new(),
        attachments: vec![crate::commands::messages::eml_attachments::eml_attachment(
            Some(&subject),
            &raw,
        )],
        auto_submitted: Some("auto-generated".to_string()),
        ..Default::default()
    };
    let send = SendActionPayload {
        account_id: account_id.clone(),
        input,
        draft_reference: None,
        scheduled_for: None,
        message_id_header: Some(payload.message_id_header()),
    };
    let mut send_action = QueuedAction::new(
        account_id,
        ACTION_SEND_MESSAGE,
        serde_json::to_value(&send).map_err(|error| format!("encode send payload: {error}"))?,
    );
    send_action.id = payload.send_id();
    send_action.status = "pending".into();

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    queue.enqueue_in_transaction(&mut tx, &send_action).await?;
    // Handed to the outbox, which sends it once (stable Message-ID) or
    // shows it as failed; not "sent" until the outbox says so.
    set_outcome(&mut *tx, &payload, "outbox").await?;
    tx.commit().await.map_err(|error| error.to_string())?;
    queue.wake_worker();
    Ok(ForwardStep::Queued(send_action.id))
}

/// `Fwd: <subject>`, once: a subject that already says so stays as it is.
fn forward_subject(subject: &str) -> String {
    let subject = subject.trim();
    let lower = subject.to_ascii_lowercase();
    if lower.starts_with("fwd:") || lower.starts_with("fw:") {
        subject.to_string()
    } else if subject.is_empty() {
        "Fwd:".to_string()
    } else {
        format!("Fwd: {subject}")
    }
}

async fn skip(
    pool: &SqlitePool,
    payload: &RuleForwardPayload,
    outcome: &'static str,
) -> SyncResult<ForwardStep> {
    tracing::info!(rule_id = %payload.rule_id, message_id = %payload.message_id, outcome, "rule forward not sent");
    set_outcome(pool, payload, outcome).await?;
    Ok(ForwardStep::Skipped(outcome))
}

async fn set_outcome<'e, E>(
    executor: E,
    payload: &RuleForwardPayload,
    outcome: &str,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE rule_forwards SET outcome = ?, updated_at = ?
         WHERE rule_token = ? AND message_key = ? AND outcome = 'queued'",
    )
    .bind(outcome)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(&payload.rule_token)
    .bind(&payload.message_key)
    .execute(executor)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    const ORIGINAL: &str = "Message-ID: <invoice-7@example.test>\r\nFrom: billing@example.test\r\nTo: me@example.test\r\nSubject: Invoice 7\r\n\r\nPlease pay.\r\n";

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        for statement in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('inbox', 'acc', 'INBOX', 'INBOX', 'inbox', 'INBOX', '2026-01-01', '2026-01-01')",
            "INSERT INTO threads (id, account_id, subject, last_message_date, created_at, updated_at)
             VALUES ('th', 'acc', 'Invoice 7', '2026-06-01T10:00:00Z', '2026-01-01', '2026-01-01')",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, from_address, to_addresses, subject, date, created_at, updated_at)
             VALUES ('msg', 'acc', 'inbox', 'th', '<invoice-7@example.test>', 'billing@example.test', '[]', 'Invoice 7', '2026-06-01T10:00:00Z', '2026-01-01', '2026-01-01')",
            "INSERT INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
             VALUES ('acc', 'tok', '<invoice-7@example.test>', 'msg', 'queued', '2026-06-01', '2026-06-01')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool
    }

    fn action() -> QueuedAction {
        let payload = RuleForwardPayload {
            rule_id: "rule-1".into(),
            rule_token: "tok".into(),
            message_id: "msg".into(),
            message_key: "<invoice-7@example.test>".into(),
            to: "books@example.test".into(),
        };
        let mut action = QueuedAction::new(
            "acc".into(),
            satchel_core::sync::queue::ACTION_RULE_FORWARD,
            serde_json::to_value(&payload).unwrap(),
        );
        action.id = payload.queue_id();
        action
    }

    async fn outcome(pool: &SqlitePool) -> String {
        sqlx::query_scalar("SELECT outcome FROM rule_forwards WHERE rule_token = 'tok'")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_forward_goes_to_the_outbox_with_the_original_attached() {
        let pool = pool().await;
        let queue = SyncQueue::new();
        let step = execute_rule_forward_with(&pool, &queue, &action(), |_| async {
            Ok(Some(ORIGINAL.as_bytes().to_vec()))
        })
        .await
        .unwrap();
        let ForwardStep::Queued(send_id) = step else {
            panic!("not queued: {step:?}");
        };
        assert_eq!(outcome(&pool).await, "outbox");
        let (action_type, payload): (String, String) =
            sqlx::query_as("SELECT action_type, payload FROM sync_queue WHERE id = ?")
                .bind(&send_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(action_type, ACTION_SEND_MESSAGE);
        let send: SendActionPayload = serde_json::from_str(&payload).unwrap();
        assert_eq!(send.input.to, vec!["books@example.test".to_string()]);
        assert_eq!(send.input.subject, "Fwd: Invoice 7");
        assert_eq!(send.input.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(send.input.attachments.len(), 1);
        let attachment = &send.input.attachments[0];
        assert_eq!(attachment.mime_type, "message/rfc822");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&attachment.content_base64)
            .unwrap();
        assert_eq!(bytes, ORIGINAL.as_bytes(), "byte for byte");
        assert!(send
            .message_id_header
            .as_deref()
            .is_some_and(|id| id.starts_with("<rule-forward-")));

        // A second run (crash before the queue row was completed) sends nothing new.
        let again = execute_rule_forward_with(&pool, &queue, &action(), |_| async {
            Ok(Some(ORIGINAL.as_bytes().to_vec()))
        })
        .await
        .unwrap();
        assert_eq!(again, ForwardStep::Skipped("not_queued"));
        let sends: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_queue WHERE action_type = 'send_message'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(sends, 1);
    }

    #[tokio::test]
    async fn automatic_and_list_mail_is_not_forwarded() {
        for (headers, label) in [
            ("Auto-Submitted: auto-replied\r\n", "auto reply"),
            ("List-Id: <news.example.test>\r\n", "list"),
            (
                "List-Unsubscribe: <mailto:off@example.test>\r\n",
                "newsletter",
            ),
            ("Precedence: bulk\r\n", "bulk"),
            ("Return-Path: <>\r\n", "bounce"),
            (
                "Content-Type: multipart/report; report-type=delivery-status\r\n",
                "dsn",
            ),
            ("X-Spam-Flag: YES\r\n", "spam"),
            ("Reply-To: books@example.test\r\n", "answers to the target"),
        ] {
            let pool = pool().await;
            let raw = format!("{headers}{ORIGINAL}");
            let step =
                execute_rule_forward_with(&pool, &SyncQueue::new(), &action(), |_| async move {
                    Ok(Some(raw.into_bytes()))
                })
                .await
                .unwrap();
            assert_eq!(step, ForwardStep::Skipped("skipped_automatic"), "{label}");
            assert_eq!(outcome(&pool).await, "skipped_automatic");
            let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(queued, 0, "{label}");
        }
    }

    #[tokio::test]
    async fn a_failed_fetch_retries_and_a_missing_source_gives_up() {
        let pool = pool().await;
        let error = execute_rule_forward_with(&pool, &SyncQueue::new(), &action(), |_| async {
            Err("offline".to_string())
        })
        .await;
        assert!(error.is_err());
        assert_eq!(outcome(&pool).await, "queued", "still to be sent");
        let step =
            execute_rule_forward_with(&pool, &SyncQueue::new(), &action(), |_| async { Ok(None) })
                .await
                .unwrap();
        assert_eq!(step, ForwardStep::Skipped("skipped_no_source"));
    }

    #[tokio::test]
    async fn a_message_too_large_or_in_spam_is_not_forwarded() {
        let sized = pool().await;
        sqlx::query("UPDATE messages SET raw_size = ?")
            .bind(i64::try_from(satchel_core::rules::MAX_FORWARD_BYTES + 1).unwrap())
            .execute(&sized)
            .await
            .unwrap();
        let fetched = std::sync::atomic::AtomicBool::new(false);
        let step = execute_rule_forward_with(&sized, &SyncQueue::new(), &action(), |_| {
            fetched.store(true, std::sync::atomic::Ordering::SeqCst);
            async { Ok(Some(ORIGINAL.as_bytes().to_vec())) }
        })
        .await
        .unwrap();
        assert_eq!(step, ForwardStep::Skipped("skipped_too_large"));
        assert!(!fetched.into_inner(), "not even downloaded");

        let unknown_size = pool().await;
        let big = vec![b'x'; satchel_core::rules::MAX_FORWARD_BYTES + 1];
        let step = execute_rule_forward_with(
            &unknown_size,
            &SyncQueue::new(),
            &action(),
            |_| async move { Ok(Some(big)) },
        )
        .await
        .unwrap();
        assert_eq!(
            step,
            ForwardStep::Skipped("skipped_too_large"),
            "size unknown before"
        );

        let in_spam = pool().await;
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, created_at, updated_at)
             VALUES ('junk', 'acc', 'Junk', 'Junk', 'spam', 'Junk', '2026-01-01', '2026-01-01')",
        )
        .execute(&in_spam)
        .await
        .unwrap();
        sqlx::query("UPDATE messages SET folder_id = 'junk'")
            .execute(&in_spam)
            .await
            .unwrap();
        let step = execute_rule_forward_with(&in_spam, &SyncQueue::new(), &action(), |_| async {
            Ok(Some(ORIGINAL.as_bytes().to_vec()))
        })
        .await
        .unwrap();
        assert_eq!(step, ForwardStep::Skipped("skipped_spam"));
    }

    #[tokio::test]
    async fn a_burst_queued_at_once_trips_the_breaker_where_it_reaches_the_outbox() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO rules (id, name, is_active, priority, trigger_type, trigger_config, actions, trigger_count, created_at, updated_at)
             VALUES ('rule-1', 'Books', 1, 0, 'on_receive', '{}', '[]', 0, '2026-01-01', '2026-01-01')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let recent = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        for index in 0..satchel_core::rules::FORWARDS_PER_HOUR {
            sqlx::query(
                "INSERT INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
                 VALUES ('acc', ?, ?, 'm', 'outbox', ?, ?)",
            )
            .bind(satchel_core::rules::rule_token("rule-1"))
            .bind(format!("<burst-{index}@example.test>"))
            .bind(&recent)
            .bind(&recent)
            .execute(&pool)
            .await
            .unwrap();
        }
        let step = execute_rule_forward_with(&pool, &SyncQueue::new(), &action(), |_| async {
            Ok(Some(ORIGINAL.as_bytes().to_vec()))
        })
        .await
        .unwrap();
        assert_eq!(step, ForwardStep::Skipped("skipped_rate_limit"));
        let paused: Option<String> =
            sqlx::query_scalar("SELECT forward_paused_at FROM rules WHERE id = 'rule-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(paused.is_some());
    }

    #[test]
    fn the_subject_says_fwd_once() {
        assert_eq!(forward_subject("Invoice"), "Fwd: Invoice");
        assert_eq!(forward_subject("FW: Invoice"), "FW: Invoice");
        assert_eq!(forward_subject("  "), "Fwd:");
    }
}
