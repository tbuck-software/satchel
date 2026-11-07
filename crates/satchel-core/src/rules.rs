//! The actions a mail rule can take, shared by the local engine (in the app)
//! and the Sieve compiler ([`crate::server_rules::compiler`]).
//!
//! A rule stores its actions as an ordered list of strings (`kind` or
//! `kind:value`); one rule can do several things, e.g. star and move. Moving
//! and forwarding are the two actions that are not idempotent enough to run
//! twice blindly, so both sides follow the same rules:
//!
//! - **Moves.** A message moves at most once per rule run: the first matching
//!   rule (by priority) with a move decides the folder, flags of every
//!   matching rule are set first. The Sieve script expresses this as an
//!   `if … elsif …` chain after all other actions. A rule never moves mail
//!   into the Inbox (rules run on Inbox mail; that would run them again).
//! - **Forwards.** A forward goes out once per rule and Message-ID, whoever
//!   sends it. Before Satchel forwards, it checks [`decide_forward`]: mail the
//!   server received while a forward of the rule to the same address, whose
//!   trigger matches the mail, may have run there is the server's. "May have
//!   run" survives failed and unfinished pushes, and ends only when Satchel
//!   has seen that its script is no longer active
//!   (`server_rules::service::verify`). The server also marks what it
//!   forwarded with one IMAP keyword ([`FORWARD_MARK`]). Neither side
//!   forwards automatic, bounce, report, list, bulk or spam mail, nor mail
//!   from the target itself ([`forward_loop_guard`]). Satchel's forward
//!   carries `Auto-Submitted: auto-generated`, stays under
//!   [`MAX_FORWARD_BYTES`], and more than [`FORWARDS_PER_HOUR`] from one rule
//!   pause that rule's forward.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// One rule action. `Display`/[`RuleAction::parse`] are the stored form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    SetState(String),
    SetPriority(i64),
    SetCategory(String),
    AddTag(String),
    MarkRead,
    Star,
    /// A folder (local id) of the rule's account.
    MoveToFolder(String),
    /// Forward to `to`. `keep_copy` only changes the server: `redirect :copy`
    /// keeps the message in the mailbox, plain `redirect` stores it nowhere
    /// (unless the same run files it). Satchel keeps the original either way:
    /// by the time it runs a rule the message is already there.
    Forward {
        to: String,
        keep_copy: bool,
    },
}

pub const KIND_MOVE: &str = "move_to_folder";
pub const KIND_FORWARD: &str = "forward";
pub const KIND_FORWARD_NO_COPY: &str = "forward_no_copy";
pub const KIND_STAR: &str = "star";

impl RuleAction {
    pub fn parse(action: &str) -> Option<Self> {
        let action = action.trim();
        match action {
            "mark_read" => return Some(Self::MarkRead),
            KIND_STAR => return Some(Self::Star),
            _ => {}
        }
        let (kind, value) = action.split_once(':')?;
        Some(match kind {
            "set_state" => Self::SetState(value.to_string()),
            "set_priority" => Self::SetPriority(value.parse().ok()?),
            "set_category" => Self::SetCategory(value.to_string()),
            "add_tag" => Self::AddTag(value.to_string()),
            KIND_MOVE if !value.trim().is_empty() => Self::MoveToFolder(value.trim().to_string()),
            KIND_FORWARD => Self::Forward {
                to: value.trim().to_string(),
                keep_copy: true,
            },
            KIND_FORWARD_NO_COPY => Self::Forward {
                to: value.trim().to_string(),
                keep_copy: false,
            },
            _ => return None,
        })
    }

    /// The stored kind, as the UI and the "local only" reason name it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SetState(_) => "set_state",
            Self::SetPriority(_) => "set_priority",
            Self::SetCategory(_) => "set_category",
            Self::AddTag(_) => "add_tag",
            Self::MarkRead => "mark_read",
            Self::Star => KIND_STAR,
            Self::MoveToFolder(_) => KIND_MOVE,
            Self::Forward {
                keep_copy: true, ..
            } => KIND_FORWARD,
            Self::Forward {
                keep_copy: false, ..
            } => KIND_FORWARD_NO_COPY,
        }
    }
}

/// Parse a stored action list; `None` if any entry is unknown.
pub fn parse_actions(actions: &[String]) -> Option<Vec<RuleAction>> {
    actions
        .iter()
        .map(|action| RuleAction::parse(action))
        .collect()
}

/// A short, stable name for a rule: its key in `rule_forwards`. Derived from
/// the rule id only, so editing a rule keeps it and a message forwarded under
/// the old version is still recognized.
pub fn rule_token(rule_id: &str) -> String {
    let digest = Sha256::digest(format!("satchel-rule:{rule_id}").as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Keyword the server sets on the copy it keeps of a message it ran any
/// forwarding rule on. One keyword for every rule: servers cap the custom
/// keywords of a mailbox (Dovecot's Maildir at 26, Cyrus at 128), and which
/// rules the server ran follows from what Satchel pushed to it.
pub const FORWARD_MARK: &str = "$SatchelFwd";
/// Keyword the server sets on a message it files away from the Inbox, so
/// Satchel still runs its local-only rules on it.
pub const FILED_MARK: &str = "$SatchelFiled";

/// The rule marks among a message's IMAP keywords, lower-cased (keywords
/// compare case-insensitively). Everything else, the workflow keywords
/// (`$Satchel-Done`, …) included, is ignored.
pub fn satchel_marks<'a>(keywords: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let known = [
        FORWARD_MARK.to_ascii_lowercase(),
        FILED_MARK.to_ascii_lowercase(),
    ];
    let mut marks: Vec<String> = keywords
        .into_iter()
        .map(|keyword| keyword.trim().to_ascii_lowercase())
        .filter(|keyword| known.contains(keyword))
        .collect();
    marks.sort();
    marks.dedup();
    marks
}

pub fn has_forward_mark(marks: &[String]) -> bool {
    marks.contains(&FORWARD_MARK.to_ascii_lowercase())
}

pub fn has_filed_mark(marks: &[String]) -> bool {
    marks.contains(&FILED_MARK.to_ascii_lowercase())
}

/// Mail the server received this long before a rule version went live there
/// still counts as the server's: the server's receive time and Satchel's
/// clock are not the same clock. Failing towards "the server did it" can drop
/// a forward from these minutes; the other way round would send it twice.
pub const SERVER_LIVE_MARGIN: chrono::Duration = chrono::Duration::minutes(2);

/// More forwards than this from one rule within an hour pause the rule's
/// forward (a loop the guards missed, or a flood). Only forwards that passed
/// the loop guard count, from the person's last resume on; only the explicit
/// resume turns it back on. Applies to Satchel's forwards; Sieve has no counter.
pub const FORWARDS_PER_HOUR: i64 = 20;

/// The largest original Satchel forwards, 20 MiB. Larger mail is recorded as
/// `skipped_too_large`: most servers refuse a message of that size anyway.
pub const MAX_FORWARD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardDecision {
    /// Satchel forwards it.
    Forward,
    /// Already decided for this rule and message (here or on another account).
    AlreadyDecided,
    /// A version of the rule that may run on the account's server was live
    /// when the message arrived there.
    ServerLive,
    /// The server marked the message as forwarded, and this rule may run there.
    ServerMarked,
}

impl ForwardDecision {
    /// Stored as `rule_forwards.outcome` when Satchel records the decision.
    pub fn outcome(self) -> Option<&'static str> {
        match self {
            Self::Forward => Some("queued"),
            Self::ServerLive => Some("skipped_server_live"),
            Self::ServerMarked => Some("server"),
            Self::AlreadyDecided => None,
        }
    }
}

/// Whether Satchel forwards a message for a rule.
///
/// `recorded`: a `rule_forwards` row for this rule and Message-ID exists.
/// `server_live_since`: since when a forward of the rule to the same address,
/// with a trigger that matches the message, may run on the account's server
/// (`None`: none does), kept through failed and unfinished pushes. `received_at`: the server's receive
/// time. `marked`: the message carries [`FORWARD_MARK`].
pub fn decide_forward(
    recorded: bool,
    server_live_since: Option<DateTime<Utc>>,
    received_at: Option<DateTime<Utc>>,
    marked: bool,
) -> ForwardDecision {
    if recorded {
        return ForwardDecision::AlreadyDecided;
    }
    let Some(since) = server_live_since else {
        return ForwardDecision::Forward;
    };
    match received_at {
        // Live, and no way to tell when the mail came: leave it to the server.
        None => ForwardDecision::ServerLive,
        Some(received) if received >= since - SERVER_LIVE_MARGIN => ForwardDecision::ServerLive,
        // Older than this version's start, but the server says it forwarded:
        // an earlier push of the same version, a record Satchel lost.
        Some(_) if marked => ForwardDecision::ServerMarked,
        Some(_) => ForwardDecision::Forward,
    }
}

/// Why a message must not be forwarded to `target` automatically, if it must
/// not. Forwarding any of these is how mail loops and floods start:
///
/// - sent automatically (RFC 3834 `Auto-Submitted` other than `no`), a bounce
///   (`Return-Path: <>`, from `postmaster` or `mailer-daemon`), a delivery or
///   read report (`multipart/report`);
/// - list or bulk mail (`List-Id`, `List-Unsubscribe`, `Precedence`);
/// - marked as spam (`X-Spam-Flag: YES`, `X-Spam-Status: Yes…`);
/// - from or answering to `target` itself.
///
/// Reads a raw header block. The Sieve script tests the same headers
/// ([`crate::server_rules::compiler`]); mail in a Spam folder is the caller's.
pub fn forward_loop_guard(raw_headers: &str, target: &str) -> Option<&'static str> {
    for (name, value) in unfolded_headers(raw_headers) {
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        match name.to_ascii_lowercase().as_str() {
            "auto-submitted" if lower != "no" => return Some("auto_submitted"),
            "list-id" | "list-unsubscribe" => return Some("list"),
            "precedence" if ["bulk", "list", "junk"].contains(&lower.as_str()) => {
                return Some("bulk")
            }
            "return-path" if lower.replace(' ', "") == "<>" => return Some("bounce"),
            "content-type" if lower.starts_with("multipart/report") => return Some("report"),
            "x-spam-flag" if lower == "yes" => return Some("spam"),
            "x-spam-status" if lower.starts_with("yes") => return Some("spam"),
            "from" => {
                let address = header_address(value);
                let local = address
                    .as_deref()
                    .and_then(|address| address.split('@').next())
                    .unwrap_or_default();
                if local == "postmaster" || local == "mailer-daemon" {
                    return Some("bounce");
                }
                if address.is_some_and(|address| same_mailbox(&address, target)) {
                    return Some("target");
                }
            }
            "reply-to" if header_address(value).is_some_and(|a| same_mailbox(&a, target)) => {
                return Some("target")
            }
            _ => {}
        }
    }
    None
}

/// The first address in a header value: the one in angle brackets, else the
/// first word with an `@`. Lower-cased.
fn header_address(value: &str) -> Option<String> {
    if let Some(start) = value.find('<') {
        let rest = &value[start + 1..];
        let end = rest.find('>')?;
        let address = rest[..end].trim();
        return (!address.is_empty()).then(|| address.to_ascii_lowercase());
    }
    value
        .split([' ', ',', ';', '"'])
        .find(|word| word.contains('@'))
        .map(|word| word.trim().to_ascii_lowercase())
}

/// Whether two addresses name the same mailbox: case-insensitive, and a
/// `+tag` in the local part (`me+lists@example.test`) reaches `me@…` too.
pub fn same_mailbox(a: &str, b: &str) -> bool {
    fn base(address: &str) -> Option<(String, String)> {
        let address = address.trim().to_ascii_lowercase();
        let (local, domain) = address.rsplit_once('@')?;
        let local = local.split('+').next().unwrap_or(local);
        (!local.is_empty() && !domain.is_empty()).then(|| (local.to_string(), domain.to_string()))
    }
    match (base(a), base(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// `(name, value)` of every header line up to the first empty line, with
/// continuation lines joined.
fn unfolded_headers(raw: &str) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    headers
}

/// A plain `local@domain` address, the only form a rule forwards to. No
/// display name, no list, nothing a header could be built from.
pub fn is_forward_address(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.len() > 254 {
        return false;
    }
    let Some((local, domain)) = value.rsplit_once('@') else {
        return false;
    };
    let forbidden =
        |ch: char| ch.is_whitespace() || ch.is_control() || "<>()[]\\,;:\"@".contains(ch);
    !local.is_empty()
        && local.len() <= 64
        && !local.chars().any(forbidden)
        && domain.contains('.')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && !domain.contains("..")
        && !domain.chars().any(forbidden)
}

/// How long Satchel remembers a forward decision and a server mark. Rules run
/// on mail newer than the last sync, so mail this old never meets them again.
pub const FORWARD_RECORD_DAYS: i64 = 180;

/// Drop forward decisions and server marks older than
/// [`FORWARD_RECORD_DAYS`]. Rows of a removed account go with it already.
pub async fn prune_forward_records(
    pool: &sqlx::SqlitePool,
    now: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let cutoff = (now - chrono::Duration::days(FORWARD_RECORD_DAYS)).to_rfc3339();
    let decisions = sqlx::query("DELETE FROM rule_forwards WHERE created_at < ?")
        .bind(&cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    let marks = sqlx::query("DELETE FROM rule_server_marks WHERE created_at < ?")
        .bind(&cutoff)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(decisions + marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_round_trip_through_their_stored_form() {
        assert_eq!(RuleAction::parse("star"), Some(RuleAction::Star));
        assert_eq!(
            RuleAction::parse("move_to_folder:folder-1"),
            Some(RuleAction::MoveToFolder("folder-1".into()))
        );
        assert_eq!(
            RuleAction::parse("forward:team@example.test"),
            Some(RuleAction::Forward {
                to: "team@example.test".into(),
                keep_copy: true
            })
        );
        assert_eq!(
            RuleAction::parse("forward_no_copy:team@example.test")
                .unwrap()
                .kind(),
            KIND_FORWARD_NO_COPY
        );
        assert_eq!(RuleAction::parse("move_to_folder:"), None);
        assert_eq!(RuleAction::parse("shell:rm"), None);
    }

    #[test]
    fn marks_are_read_case_insensitively() {
        let marks = satchel_marks([
            "$SATCHELFWD",
            "\\Seen",
            "$Junk",
            "$SatchelFiled",
            "$Satchel-Done",
        ]);
        assert_eq!(marks.len(), 2, "{marks:?}");
        assert!(has_forward_mark(&marks) && has_filed_mark(&marks));
        assert!(!has_forward_mark(&satchel_marks(["$SatchelFwd_0123"])));
        assert_ne!(rule_token("rule-1"), rule_token("rule-2"));
        assert_eq!(rule_token("rule-1").len(), 16);
    }

    #[test]
    fn a_recorded_forward_is_never_repeated() {
        let now = Utc::now();
        assert_eq!(
            decide_forward(true, None, Some(now), false),
            ForwardDecision::AlreadyDecided
        );
        assert_eq!(
            decide_forward(true, Some(now), Some(now), true),
            ForwardDecision::AlreadyDecided
        );
    }

    #[test]
    fn mail_that_arrived_while_the_server_ran_the_rule_is_the_servers() {
        let live = Utc::now() - chrono::Duration::hours(1);
        let after = live + chrono::Duration::minutes(5);
        let just_before = live - chrono::Duration::seconds(30);
        let long_before = live - chrono::Duration::hours(2);
        let decide = |since, received, marked| decide_forward(false, since, received, marked);
        assert_eq!(
            decide(Some(live), Some(after), false),
            ForwardDecision::ServerLive
        );
        assert_eq!(
            decide(Some(live), Some(just_before), false),
            ForwardDecision::ServerLive,
            "inside the clock margin"
        );
        assert_eq!(decide(Some(live), None, false), ForwardDecision::ServerLive);
        assert_eq!(
            decide(Some(live), Some(long_before), false),
            ForwardDecision::Forward,
            "arrived before the server had the rule"
        );
        assert_eq!(
            decide(Some(live), Some(long_before), true),
            ForwardDecision::ServerMarked,
            "the server says it forwarded"
        );
        assert_eq!(
            decide(None, Some(after), true),
            ForwardDecision::Forward,
            "the mark came from another rule: this one does not run there"
        );
    }

    const TARGET: &str = "books@example.test";

    fn guard(headers: &str) -> Option<&'static str> {
        forward_loop_guard(headers, TARGET)
    }

    #[test]
    fn automatic_and_list_mail_is_not_forwarded() {
        assert_eq!(
            guard("From: a@example.test\r\nAuto-Submitted: auto-replied\r\n\r\nbody"),
            Some("auto_submitted")
        );
        assert_eq!(
            guard("Auto-Submitted:\r\n auto-generated\r\n\r\n"),
            Some("auto_submitted"),
            "folded"
        );
        assert_eq!(guard("Auto-Submitted: No\r\n\r\n"), None);
        assert_eq!(guard("List-Id: <news.example.test>\n\n"), Some("list"));
        assert_eq!(
            guard("List-Unsubscribe: <mailto:off@example.test>\n\n"),
            Some("list"),
            "newsletters without List-Id"
        );
        assert_eq!(guard("Precedence: BULK\n\n"), Some("bulk"));
        assert_eq!(guard("Precedence: first-class\n\n"), None);
        assert_eq!(
            guard("Subject: hi\r\n\r\nList-Id: in the body\r\n"),
            None,
            "only the header block counts"
        );
    }

    #[test]
    fn bounces_reports_and_spam_are_not_forwarded() {
        assert_eq!(guard("Return-Path: <>\r\n\r\n"), Some("bounce"));
        assert_eq!(guard("Return-Path: < >\r\n\r\n"), Some("bounce"));
        assert_eq!(guard("Return-Path: <a@example.test>\r\n\r\n"), None);
        assert_eq!(
            guard("From: Mail Delivery System <MAILER-DAEMON@example.test>\r\n\r\n"),
            Some("bounce")
        );
        assert_eq!(
            guard("From: postmaster@example.test\r\n\r\n"),
            Some("bounce")
        );
        assert_eq!(
            guard("Content-Type: multipart/report; report-type=delivery-status;\r\n boundary=x\r\n\r\n"),
            Some("report"),
            "a DSN"
        );
        assert_eq!(
            guard("Content-Type: Multipart/Report; report-type=disposition-notification\r\n\r\n"),
            Some("report"),
            "an MDN"
        );
        assert_eq!(guard("X-Spam-Flag: YES\r\n\r\n"), Some("spam"));
        assert_eq!(guard("X-Spam-Status: Yes, score=9.1\r\n\r\n"), Some("spam"));
        assert_eq!(guard("X-Spam-Status: No, score=0.1\r\n\r\n"), None);
    }

    #[test]
    fn mail_from_or_answering_to_the_target_is_not_sent_back() {
        assert_eq!(
            guard("From: Books <BOOKS@example.test>\r\n\r\n"),
            Some("target")
        );
        assert_eq!(guard("From: books+in@example.test\r\n\r\n"), Some("target"));
        assert_eq!(
            guard("From: a@example.test\r\nReply-To: <books@example.test>\r\n\r\n"),
            Some("target")
        );
        assert_eq!(guard("From: a@example.test\r\n\r\n"), None);
    }

    #[test]
    fn plus_tags_reach_the_same_mailbox() {
        assert!(same_mailbox("Me+lists@Example.test", "me@example.test"));
        assert!(!same_mailbox("me@example.test", "me@other.example.test"));
        assert!(!same_mailbox("me", "me"));
    }

    #[test]
    fn forward_addresses_are_single_plain_addresses() {
        assert!(is_forward_address("team@example.test"));
        assert!(is_forward_address(" first.last+tag@sub.example.test "));
        assert!(!is_forward_address("Team <team@example.test>"));
        assert!(!is_forward_address("a@example.test, b@example.test"));
        assert!(!is_forward_address("a@example"));
        assert!(!is_forward_address("a@.example.test"));
        assert!(!is_forward_address("a\r\nBcc: x@example.test"));
        assert!(!is_forward_address("@example.test"));
    }

    #[tokio::test]
    async fn forward_records_expire_and_go_with_their_account() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        for account in ["acc", "gone"] {
            sqlx::query(
                "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                 VALUES (?, 'Me', ?, 'imap', '{}', '', '2026-01-01', '2026-01-01')",
            )
            .bind(account)
            .bind(format!("{account}@example.test"))
            .execute(&pool)
            .await
            .unwrap();
        }
        let now = Utc::now();
        let old = (now - chrono::Duration::days(FORWARD_RECORD_DAYS + 1)).to_rfc3339();
        let fresh = now.to_rfc3339();
        for (account, key, at) in [
            ("acc", "<old@x>", &old),
            ("acc", "<new@x>", &fresh),
            ("gone", "<g@x>", &fresh),
        ] {
            sqlx::query(
                "INSERT INTO rule_forwards (account_id, rule_token, message_key, message_id, outcome, created_at, updated_at)
                 VALUES (?, 't', ?, 'm', 'outbox', ?, ?)",
            )
            .bind(account)
            .bind(key)
            .bind(at)
            .bind(at)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO rule_server_marks (account_id, message_key, created_at) VALUES (?, ?, ?)")
                .bind(account)
                .bind(key)
                .bind(at)
                .execute(&pool)
                .await
                .unwrap();
        }
        assert_eq!(prune_forward_records(&pool, now).await.unwrap(), 2);
        sqlx::query("DELETE FROM accounts WHERE id = 'gone'")
            .execute(&pool)
            .await
            .unwrap();
        let left: Vec<String> = sqlx::query_scalar(
            "SELECT message_key FROM rule_forwards UNION ALL SELECT message_key FROM rule_server_marks",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(left, vec!["<new@x>".to_string(), "<new@x>".to_string()]);
    }
}
