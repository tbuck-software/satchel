use satchel_lib::{
    commands::calendar::{queue_invite_response, InviteResponse},
    db::calendar::CalendarRepository,
    email::calendar::{build_invite_reply_ics, parse_invite},
    AppState,
};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

const SERIES_UID: &str = "series-123@example.com";

const UNBOUNDED_SERIES: &str = concat!(
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:series-123@example.com\r\n",
    "ORGANIZER;CN=Alice:mailto:alice@example.com\r\n",
    "ATTENDEE;RSVP=TRUE:mailto:me@example.com\r\n",
    "SUMMARY:Monthly review\r\n",
    "DTSTART;TZID=Europe/Berlin:20260924T090000\r\n",
    "DTEND;TZID=Europe/Berlin:20260924T093000\r\n",
    "RRULE:FREQ=MONTHLY;INTERVAL=1;BYDAY=4TH\r\n",
    "SEQUENCE:2\r\n",
    "END:VEVENT\r\nEND:VCALENDAR\r\n",
);

const BOUNDED_SERIES: &str = concat!(
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:series-123@example.com\r\n",
    "ORGANIZER:mailto:alice@example.com\r\n",
    "SUMMARY:Monthly review\r\n",
    "DTSTART:20260924T070000Z\r\n",
    "DTEND:20260924T073000Z\r\n",
    "RRULE:FREQ=MONTHLY;COUNT=16;BYDAY=4TH\r\n",
    "SEQUENCE:2\r\n",
    "END:VEVENT\r\nEND:VCALENDAR\r\n",
);

const OCCURRENCE: &str = concat!(
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:series-123@example.com\r\n",
    "RECURRENCE-ID;TZID=Europe/Berlin:20261022T090000\r\n",
    "ORGANIZER:mailto:alice@example.com\r\n",
    "SUMMARY:Rescheduled review\r\n",
    "DTSTART;TZID=Europe/Berlin:20261022T100000\r\n",
    "DTEND;TZID=Europe/Berlin:20261022T103000\r\n",
    "SEQUENCE:3\r\n",
    "END:VEVENT\r\nEND:VCALENDAR\r\n",
);

fn unfold(raw: &str) -> String {
    raw.replace("\r\n ", "").replace("\r\n\t", "")
}

async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect test database");
    satchel_lib::db::init(&pool)
        .await
        .expect("apply migrations");
    sqlx::query(
        "INSERT INTO accounts
            (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('acc', 'Me', 'me@example.com', 'imap', '{}', 'test', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("insert account");
    sqlx::query(
        "INSERT INTO folders
            (id, account_id, name, full_path, folder_type, created_at, updated_at)
         VALUES ('calendar', 'acc', 'Calendar', 'Calendar', 'custom', '2026-01-01', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect("insert calendar folder");
    for message_id in ["message-master", "message-occurrence"] {
        sqlx::query(
            "INSERT INTO messages
                (id, account_id, folder_id, from_address, to_addresses, subject, date, created_at, updated_at)
             VALUES (?, 'acc', 'calendar', 'alice@example.com', '[\"me@example.com\"]', 'Invite', '2026-01-01', '2026-01-01', '2026-01-01')",
        )
        .bind(message_id)
        .execute(&pool)
        .await
        .expect("insert invitation message");
    }
    pool
}

#[test]
fn classifies_master_series_and_occurrences_without_scanning_vtimezone_rules() {
    let unbounded = parse_invite(UNBOUNDED_SERIES).expect("parse unbounded series");
    assert!(unbounded.recurring);
    assert!(unbounded.recurrence_unbounded);
    assert_eq!(unbounded.recurrence_id, None);

    let bounded = parse_invite(BOUNDED_SERIES).expect("parse bounded series");
    assert!(bounded.recurring);
    assert!(!bounded.recurrence_unbounded);
    assert_eq!(bounded.recurrence_id, None);

    let occurrence = parse_invite(OCCURRENCE).expect("parse occurrence");
    assert!(occurrence.recurring);
    assert!(!occurrence.recurrence_unbounded);
    assert_eq!(occurrence.recurrence_id.as_deref(), Some("20261022T090000"));
}

#[test]
fn builds_one_reply_for_a_whole_series_and_preserves_instance_identity() {
    let master = parse_invite(UNBOUNDED_SERIES).expect("parse series");
    let master_reply = unfold(
        &build_invite_reply_ics(&master, "me@example.com", Some("Me"), "ACCEPTED")
            .expect("build series reply"),
    );
    assert_eq!(master_reply.matches("VERSION:2.0").count(), 1);
    assert_eq!(master_reply.matches("PRODID:").count(), 1);
    assert_eq!(master_reply.matches("METHOD:REPLY").count(), 1);
    assert_eq!(master_reply.matches("BEGIN:VEVENT").count(), 1);
    assert!(master_reply.contains(&format!("UID:{SERIES_UID}")));
    let master_attendee = master_reply
        .lines()
        .find(|line| line.starts_with("ATTENDEE"))
        .expect("series reply attendee");
    assert!(master_attendee.contains("PARTSTAT=ACCEPTED"));
    assert!(!master_reply.contains("RECURRENCE-ID"));

    let occurrence = parse_invite(OCCURRENCE).expect("parse occurrence");
    let occurrence_reply = unfold(
        &build_invite_reply_ics(&occurrence, "me@example.com", None, "DECLINED")
            .expect("build occurrence reply"),
    );
    assert_eq!(occurrence_reply.matches("BEGIN:VEVENT").count(), 1);
    assert!(occurrence_reply.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261022T090000"));
    let occurrence_attendee = occurrence_reply
        .lines()
        .find(|line| line.starts_with("ATTENDEE"))
        .expect("occurrence reply attendee");
    assert!(occurrence_attendee.contains("PARTSTAT=DECLINED"));
}

#[tokio::test]
async fn stores_a_master_and_an_exception_as_distinct_invitations() {
    let pool = test_pool().await;
    let master = parse_invite(BOUNDED_SERIES).expect("parse master");
    let occurrence = parse_invite(OCCURRENCE).expect("parse occurrence");

    CalendarRepository::new(&pool)
        .upsert_invite("acc", Some("message-master"), &master)
        .await
        .expect("store master");
    CalendarRepository::new(&pool)
        .upsert_invite("acc", Some("message-occurrence"), &occurrence)
        .await
        .expect("store occurrence");

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar_invites WHERE account_id = 'acc' AND uid = ?",
    )
    .bind(SERIES_UID)
    .fetch_one(&pool)
    .await
    .expect("count invites");
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn queues_each_logical_rsvp_once_and_leaves_it_ready_for_the_worker() {
    let pool = test_pool().await;
    let invite = parse_invite(BOUNDED_SERIES).expect("parse invite");
    CalendarRepository::new(&pool)
        .upsert_invite("acc", Some("message-master"), &invite)
        .await
        .expect("store invite");
    let invite_id: String =
        sqlx::query_scalar("SELECT id FROM calendar_invites WHERE account_id = 'acc' AND uid = ?")
            .bind(SERIES_UID)
            .fetch_one(&pool)
            .await
            .expect("load invite id");
    let state = AppState::default();
    let accepted = InviteResponse {
        invite_id: invite_id.clone(),
        response: "accepted".into(),
        comment: None,
    };

    queue_invite_response(&state, &pool, accepted.clone())
        .await
        .expect("queue first response");
    queue_invite_response(&state, &pool, accepted)
        .await
        .expect("repeat response is idempotent");

    let queued: (i64, String) = sqlx::query_as(
        "SELECT COUNT(*), MIN(status) FROM sync_queue WHERE action_type = 'send_message'",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect queue");
    assert_eq!(queued.0, 1);
    assert_eq!(queued.1, "pending");

    queue_invite_response(
        &state,
        &pool,
        InviteResponse {
            invite_id,
            response: "declined".into(),
            comment: None,
        },
    )
    .await
    .expect("a changed response is a new logical reply");

    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue WHERE action_type = 'send_message'")
            .fetch_one(&pool)
            .await
            .expect("count changed response");
    assert_eq!(queued, 2);
}
