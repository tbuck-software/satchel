use super::*;

async fn pool() -> SqlitePool {
    crate::utils::crypto::seed_master_key_for_tests();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::init(&pool).await.unwrap();
    sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at) VALUES ('a', 'Me', 'me@example.com', 'imap', '{}', '', '{}', '', 1, 'now', 'now')").execute(&pool).await.unwrap();
    let config = crate::accounts::encrypt_account_config("a", "{}").unwrap();
    sqlx::query("UPDATE accounts SET config = ? WHERE id = 'a'")
        .bind(config)
        .execute(&pool)
        .await
        .unwrap();
    pool
}

fn input() -> EventInput {
    EventInput {
        id: Some("event".into()),
        expected_updated_at: None,
        account_id: "a".into(),
        title: "Planning".into(),
        description: None,
        location: None,
        starts_at: "2026-03-28T10:00:00Z".into(),
        ends_at: "2026-03-28T11:00:00Z".into(),
        all_day: false,
        attendees: vec!["guest@example.com".into()],
        send_invites: false,
    }
}

async fn create_event(pool: &SqlitePool, mut event: EventInput) -> CalendarEventRecord {
    event.id = None;
    let saved = save_event(pool, &SyncQueue::new(), event).await.unwrap();
    sqlx::query("UPDATE calendar_events SET id = 'event' WHERE id = ?")
        .bind(&saved.id)
        .execute(pool)
        .await
        .unwrap();
    saved
}

async fn raw(pool: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT raw_ics FROM calendar_events WHERE id = 'event'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn title_edit_preserves_recurrence_alarms_exceptions_and_participation() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-CALENDAR-META:keep\r\nBEGIN:VEVENT\r\nUID:series\r\nSUMMARY:Planning\r\nDTSTART;TZID=Europe/Berlin:20260328T110000\r\nDTEND;TZID=Europe/Berlin:20260328T120000\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEXDATE;TZID=Europe/Berlin:20260329T110000\r\nSEQUENCE:8\r\nORGANIZER;CN=\"Original organizer\";SENT-BY=\"mailto:delegate@example.com\":mailto:me@example.com\r\nATTENDEE;CN=Guest;PARTSTAT=ACCEPTED;RSVP=FALSE:mailto:guest@example.com\r\nX-VENDOR;TYPE=custom:keep\r\n continuation\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Alarm\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:series\r\nRECURRENCE-ID;TZID=Europe/Berlin:20260330T110000\r\nDTSTART;TZID=Europe/Berlin:20260330T140000\r\nDTEND;TZID=Europe/Berlin:20260330T150000\r\nSUMMARY:Exception\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    sqlx::query("UPDATE calendar_events SET uid = 'series', raw_ics = ? WHERE id = 'event'")
        .bind(original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "New title".into();
    save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    let edited = raw(&pool).await;
    for property in ["RRULE:FREQ=DAILY;COUNT=3", "EXDATE;TZID=Europe/Berlin:20260329T110000", "BEGIN:VALARM", "SEQUENCE:9", "ORGANIZER;CN=\"Original organizer\";SENT-BY=\"mailto:delegate@example.com\":mailto:me@example.com", "ATTENDEE;CN=Guest;PARTSTAT=ACCEPTED;RSVP=FALSE:mailto:guest@example.com", "X-VENDOR;TYPE=custom:keep\r\n continuation", "RECURRENCE-ID;TZID=Europe/Berlin:20260330T110000", "SUMMARY:Exception", "DTSTART;TZID=Europe/Berlin:20260328T110000"] {
        assert!(edited.contains(property), "lost {property}: {edited}");
    }
    assert!(edited.contains("SUMMARY:New title"));
}

#[tokio::test]
async fn attendee_storage_is_independent_of_mail_notification_intent() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(raw(&pool).await.contains("mailto:guest@example.com"));
}

#[tokio::test]
async fn all_day_accepts_dates_and_stores_exclusive_date_boundaries() {
    let pool = pool().await;
    let mut event = input();
    event.all_day = true;
    event.starts_at = "2026-03-28".into();
    event.ends_at = "2026-03-31".into();
    let saved = create_event(&pool, event.clone()).await;
    assert_eq!(saved.starts_at, "2026-03-28");
    assert_eq!(saved.ends_at, "2026-03-31");
    event.title = "Edited holiday".into();
    save_event(&pool, &SyncQueue::new(), event).await.unwrap();
    let ics = raw(&pool).await;
    assert!(ics.contains("DTSTART;VALUE=DATE:20260328\r\n"));
    assert!(ics.contains("DTEND;VALUE=DATE:20260331\r\n"));
}
#[tokio::test]
async fn local_save_and_invitation_queue_are_atomic_and_retryable() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let queue = SyncQueue::new();
    sqlx::query("CREATE TRIGGER reject_invitation BEFORE INSERT ON sync_queue BEGIN SELECT RAISE(ABORT, 'queue unavailable'); END").execute(&pool).await.unwrap();
    let mut edit = input();
    edit.title = "Changed".into();
    edit.send_invites = true;
    assert!(save_event(&pool, &queue, edit.clone()).await.is_err());
    assert_eq!(
        CalendarRepository::new(&pool)
            .get_event("a", "event")
            .await
            .unwrap()
            .title,
        "Planning"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER reject_invitation")
        .execute(&pool)
        .await
        .unwrap();
    save_event(&pool, &queue, edit.clone()).await.unwrap();
    save_event(&pool, &queue, edit).await.unwrap();
    let (status, payload): (String, String) =
        sqlx::query_as("SELECT status, payload FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "pending");
    let payload: SendActionPayload = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload.input.to, ["guest@example.com"]);
    use base64::Engine as _;
    let attachment = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(&payload.input.attachments[0].content_base64)
            .unwrap(),
    )
    .unwrap();
    assert!(attachment.contains("METHOD:REQUEST\r\n"));
    assert!(attachment.contains("SUMMARY:Changed\r\n"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn unsupported_edits_leave_the_original_resource_untouched() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = raw(&pool)
        .await
        .replace("END:VEVENT", "RRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT");
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(&original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.starts_at = "2026-03-28T09:00:00Z".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
    let mut edit = input();
    edit.attendees.clear();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
    assert_eq!(raw(&pool).await, original);
    sqlx::query("UPDATE calendar_events SET raw_ics = NULL WHERE id = 'event'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), input()).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
}

#[tokio::test]
async fn deleted_event_is_not_recreated_by_a_stale_editor() {
    let pool = pool().await;
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), input()).await,
        Err(SaveError::Calendar(CalendarError::EventNotFound))
    ));
}

#[tokio::test]
async fn single_event_time_change_preserves_alarm_and_unknown_properties() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = raw(&pool).await.replace("END:VEVENT", "X-PRIVATE:keep\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT");
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(&original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.starts_at = "2026-03-28T09:00:00Z".into();
    save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    let ics = raw(&pool).await;
    assert!(ics.contains("DTSTART:20260328T090000Z"));
    assert!(ics.contains("DESCRIPTION:Reminder"));
    assert!(ics.contains("X-PRIVATE:keep"));
}

async fn dav_server(status: u16) -> (String, tokio::task::JoinHandle<String>) {
    dav_server_with_hook(status, || async {}).await
}

async fn dav_server_with_hook<F, Fut>(
    status: u16,
    before_response: F,
) -> (String, tokio::task::JoinHandle<String>)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        before_response().await;
        let response = format!("HTTP/1.1 {status} Result\r\nETag: \"v2\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (address, task)
}

async fn remote_event(pool: &SqlitePool, server: &str) {
    create_event(pool, input()).await;
    let config = crate::accounts::encrypt_account_config(
        "a",
        &serde_json::json!({"caldav": {"server": server, "username": "test", "password": "test"}})
            .to_string(),
    )
    .unwrap();
    sqlx::query("UPDATE accounts SET config = ? WHERE id = 'a'")
        .bind(config)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE calendar_events SET source = 'caldav', remote_href = ?, remote_etag = '\"v1\"' WHERE id = 'event'").bind(format!("{server}calendar/event.ics")).execute(pool).await.unwrap();
}

#[tokio::test]
async fn dav_put_preserves_guests_without_requesting_email_invitations() {
    let pool = pool().await;
    let (server, request) = dav_server(204).await;
    remote_event(&pool, &server).await;
    let mut edit = input();
    edit.title = "Remote edit".into();
    let saved = save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), request)
        .await
        .unwrap()
        .unwrap();
    assert!(request.starts_with("PUT /calendar/event.ics "));
    assert!(request.to_ascii_lowercase().contains("if-match: \"v1\""));
    assert!(request.contains("mailto:guest@example.com"));
    assert!(request.contains("SUMMARY:Remote edit"));
    assert!(!request.contains("METHOD:REQUEST"));
    assert_eq!(saved.source, "caldav");
    assert_eq!(saved.attendees_json, "[\"guest@example.com\"]");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn dav_conflict_does_not_change_the_local_event_or_queue_mail() {
    let pool = pool().await;
    let (server, request) = dav_server(412).await;
    remote_event(&pool, &server).await;
    let original = raw(&pool).await;
    let mut edit = input();
    edit.title = "Conflict".into();
    edit.send_invites = true;
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Remote(_))
    ));
    request.await.unwrap();
    assert_eq!(raw(&pool).await, original);
    assert_eq!(
        CalendarRepository::new(&pool)
            .get_event("a", "event")
            .await
            .unwrap()
            .title,
        "Planning"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn remote_commit_followed_by_local_failure_requires_sync() {
    let pool = pool().await;
    let (server, request) = dav_server(204).await;
    remote_event(&pool, &server).await;
    sqlx::query("CREATE TRIGGER reject_event BEFORE UPDATE ON calendar_events BEGIN SELECT RAISE(ABORT, 'local write failed'); END").execute(&pool).await.unwrap();
    let mut edit = input();
    edit.title = "Saved remotely".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::RemoteCommittedLocalFailed(_))
    ));
    request.await.unwrap();
    assert_eq!(
        CalendarRepository::new(&pool)
            .get_event("a", "event")
            .await
            .unwrap()
            .title,
        "Planning"
    );
}

#[tokio::test]
async fn remote_events_need_a_version_and_cannot_be_downgraded_to_local() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    sqlx::query("UPDATE calendar_events SET source = 'caldav', remote_href = 'https://calendar.example/event.ics', remote_etag = NULL WHERE id = 'event'").execute(&pool).await.unwrap();
    let mut edit = input();
    edit.title = "Unsafe".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit.clone()).await,
        Err(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable"
        ))
    ));
    sqlx::query("UPDATE calendar_events SET remote_etag = '\"v1\"' WHERE id = 'event'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable"
        ))
    ));
    assert_eq!(
        CalendarRepository::new(&pool)
            .get_event("a", "event")
            .await
            .unwrap()
            .source,
        "caldav"
    );
}

#[tokio::test]
async fn cannot_replace_another_organizers_event_or_reset_its_sequence() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = raw(&pool).await;
    let foreign = original.replace("mailto:me@example.com", "mailto:organizer@example.com");
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(&foreign)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "Unsafe".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit.clone()).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
    let invalid_sequence = original.replace("END:VEVENT", "SEQUENCE:not-a-number\r\nEND:VEVENT");
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(&invalid_sequence)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
    assert_eq!(raw(&pool).await, invalid_sequence);
}

#[tokio::test]
async fn deletion_conflict_leaves_the_local_event_available_for_sync() {
    let pool = pool().await;
    let (server, request) = dav_server(412).await;
    remote_event(&pool, &server).await;
    assert!(matches!(
        delete_event(&pool, "event").await,
        Err(SaveError::Remote(_))
    ));
    let request = request.await.unwrap();
    assert!(request.starts_with("DELETE /calendar/event.ics "));
    assert!(request.to_ascii_lowercase().contains("if-match: \"v1\""));
    assert!(CalendarRepository::new(&pool)
        .get_event("a", "event")
        .await
        .is_ok());
}

#[tokio::test]
async fn legacy_offset_all_day_inputs_keep_the_written_date_across_dst() {
    for (start, end, expected_start, expected_end) in [
        (
            "2026-03-29T00:00:00+01:00",
            "2026-03-30T00:00:00+02:00",
            "20260329",
            "20260330",
        ),
        (
            "2026-10-25T00:00:00+02:00",
            "2026-10-26T00:00:00+01:00",
            "20261025",
            "20261026",
        ),
        (
            "2026-03-08T00:00:00-08:00",
            "2026-03-09T00:00:00-07:00",
            "20260308",
            "20260309",
        ),
        (
            "2026-11-01T00:00:00-07:00",
            "2026-11-02T00:00:00-08:00",
            "20261101",
            "20261102",
        ),
    ] {
        let pool = pool().await;
        let mut event = input();
        event.all_day = true;
        event.starts_at = start.into();
        event.ends_at = end.into();
        create_event(&pool, event).await;
        let ics = raw(&pool).await;
        assert!(
            ics.contains(&format!("DTSTART;VALUE=DATE:{expected_start}\r\n")),
            "{ics}"
        );
        assert!(
            ics.contains(&format!("DTEND;VALUE=DATE:{expected_end}\r\n")),
            "{ics}"
        );
    }
}

#[tokio::test]
async fn title_edit_preserves_the_stored_participant_projection() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let attendees = r#"[{"email":"guest@example.com","partstat":"ACCEPTED","name":"Guest"}]"#;
    sqlx::query("UPDATE calendar_events SET attendees_json = ? WHERE id = 'event'")
        .bind(attendees)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "Updated".into();
    let saved = save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    assert_eq!(saved.attendees_json, attendees);
    assert!(raw(&pool).await.contains("mailto:guest@example.com"));
}

#[tokio::test]
async fn editing_text_retains_language_and_quoted_vendor_parameters() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = raw(&pool).await.replace(
        "SUMMARY:Planning",
        "SUMMARY;LANGUAGE=en;X-VENDOR=\"a:b\":Planning",
    );
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "Changed".into();
    save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    assert!(raw(&pool)
        .await
        .contains("SUMMARY;LANGUAGE=en;X-VENDOR=\"a:b\":Changed"));
}

#[tokio::test]
async fn moving_a_single_zoned_event_keeps_its_timezone_across_dst() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = raw(&pool)
        .await
        .replace(
            "DTSTART:20260328T100000Z",
            "DTSTART;TZID=Europe/Berlin:20260328T110000",
        )
        .replace(
            "DTEND:20260328T110000Z",
            "DTEND;TZID=Europe/Berlin:20260328T120000",
        );
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.starts_at = "2026-03-29T09:00:00Z".into();
    edit.ends_at = "2026-03-29T10:00:00Z".into();
    save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    let ics = raw(&pool).await;
    assert!(ics.contains("DTSTART;TZID=Europe/Berlin:20260329T110000"));
    assert!(ics.contains("DTEND;TZID=Europe/Berlin:20260329T120000"));
}

async fn single_zoned_event(pool: &SqlitePool, zone: chrono_tz::Tz) -> String {
    let event = input();
    create_event(pool, event.clone()).await;
    let mut original = raw(pool).await;
    for (key, value) in [("DTSTART", &event.starts_at), ("DTEND", &event.ends_at)] {
        let instant = chrono::DateTime::parse_from_rfc3339(value).unwrap();
        original = original.replace(
            &format!("{key}:{}", instant.format("%Y%m%dT%H%M%SZ")),
            &format!(
                "{key};TZID={zone}:{}",
                instant.with_timezone(&zone).format("%Y%m%dT%H%M%S")
            ),
        );
    }
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(&original)
        .execute(pool)
        .await
        .unwrap();
    original
}

#[tokio::test]
async fn zoned_fall_back_second_occurrence_is_rejected_without_changing_event_or_queue() {
    for (zone, first, second, end) in [
        (
            chrono_tz::Europe::Berlin,
            "2026-10-25T00:15:00Z",
            "2026-10-25T01:15:00Z",
            "2026-10-25T01:45:00Z",
        ),
        (
            chrono_tz::America::Los_Angeles,
            "2026-11-01T08:15:00Z",
            "2026-11-01T09:15:00Z",
            "2026-11-01T09:45:00Z",
        ),
    ] {
        let pool = pool().await;
        let original = single_zoned_event(&pool, zone).await;
        let repository = CalendarRepository::new(&pool);
        let before = repository.get_event("a", "event").await.unwrap();
        // Check DTSTART, then a representable DTSTART with an ambiguous DTEND.
        for start in [second, first] {
            let mut edit = input();
            edit.starts_at = start.into();
            edit.ends_at = end.into();
            edit.send_invites = true;
            assert!(
                matches!(
                    save_event(&pool, &SyncQueue::new(), edit).await,
                    Err(SaveError::Validation(
                        "backend.errors.calendarEditUnsupported"
                    ))
                ),
                "{zone}: {start} to {end}"
            );
            assert_eq!(raw(&pool).await, original);
            let after = repository.get_event("a", "event").await.unwrap();
            assert_eq!(after.starts_at, before.starts_at);
            assert_eq!(after.ends_at, before.ends_at);
            assert_eq!(after.updated_at, before.updated_at);
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_queue")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
        }
    }
}

#[tokio::test]
async fn zoned_fall_back_first_occurrence_round_trips_without_shifting() {
    for (zone, start, end) in [
        (
            chrono_tz::Europe::Berlin,
            "2026-10-25T00:15:00Z",
            "2026-10-25T00:45:00Z",
        ),
        (
            chrono_tz::America::Los_Angeles,
            "2026-11-01T08:15:00Z",
            "2026-11-01T08:45:00Z",
        ),
    ] {
        let pool = pool().await;
        single_zoned_event(&pool, zone).await;
        let mut edit = input();
        edit.starts_at = start.into();
        edit.ends_at = end.into();
        save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
        let ics = raw(&pool).await;
        assert!(ics.contains(&format!("DTSTART;TZID={zone}:")));
        assert!(ics.contains(&format!("DTEND;TZID={zone}:")));
        let parsed = crate::email::calendar::parse_invite(&ics).unwrap();
        assert_eq!(parsed.starts_at.as_deref(), Some(start));
        assert_eq!(parsed.ends_at.as_deref(), Some(end));
    }
}

#[tokio::test]
async fn floating_and_unknown_zone_time_changes_are_refused_without_loss() {
    for start in [
        "DTSTART:20260328T100000",
        "DTSTART;TZID=Custom/Zone:20260328T100000",
    ] {
        let pool = pool().await;
        create_event(&pool, input()).await;
        let original = raw(&pool).await.replace("DTSTART:20260328T100000Z", start);
        sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
            .bind(&original)
            .execute(&pool)
            .await
            .unwrap();
        let mut edit = input();
        edit.starts_at = "2026-03-28T09:00:00Z".into();
        assert!(matches!(
            save_event(&pool, &SyncQueue::new(), edit).await,
            Err(SaveError::Validation(
                "backend.errors.calendarEditUnsupported"
            ))
        ));
        assert_eq!(raw(&pool).await, original);
    }
}

#[tokio::test]
async fn a_display_fallback_end_cannot_overwrite_duration_only_events() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = raw(&pool)
        .await
        .replace("DTEND:20260328T110000Z", "DURATION:PT2H");
    sqlx::query("UPDATE calendar_events SET raw_ics = ?, ends_at = starts_at WHERE id = 'event'")
        .bind(&original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "New title".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
    assert_eq!(raw(&pool).await, original);
}

#[tokio::test]
async fn whitespace_etags_cannot_trigger_an_unconditional_put() {
    let pool = pool().await;
    let (server, request) = dav_server(204).await;
    remote_event(&pool, &server).await;
    sqlx::query("UPDATE calendar_events SET remote_etag = '   ' WHERE id = 'event'")
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "Unsafe".into();
    let result = save_event(&pool, &SyncQueue::new(), edit).await;
    request.abort();
    assert!(matches!(
        result,
        Err(SaveError::Validation(
            "backend.errors.calendarRemoteUnavailable"
        ))
    ));
    assert_eq!(
        CalendarRepository::new(&pool)
            .get_event("a", "event")
            .await
            .unwrap()
            .title,
        "Planning"
    );
}

#[tokio::test]
async fn a_stale_editor_cannot_overwrite_fields_refreshed_by_sync() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let original = CalendarRepository::new(&pool)
        .get_event("a", "event")
        .await
        .unwrap();
    let newer = raw(&pool).await.replace(
        "SUMMARY:Planning",
        "SUMMARY:Synced title\r\nLOCATION:New location",
    );
    sqlx::query("UPDATE calendar_events SET title = 'Synced title', location = 'New location', raw_ics = ?, updated_at = 'new-version' WHERE id = 'event'")
        .bind(&newer).execute(&pool).await.unwrap();
    let mut edit = input();
    edit.expected_updated_at = Some(original.updated_at);
    edit.title = "My title".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Calendar(CalendarError::EventChanged))
    ));
    assert_eq!(raw(&pool).await, newer);
    assert_eq!(
        CalendarRepository::new(&pool)
            .get_event("a", "event")
            .await
            .unwrap()
            .location
            .as_deref(),
        Some("New location")
    );
}

#[tokio::test]
async fn unchanged_writes_keep_the_editor_revision_but_changes_advance_it() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    sqlx::query("UPDATE calendar_events SET updated_at = 'original-version' WHERE id = 'event'")
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.expected_updated_at = Some("original-version".into());
    let same = save_event(&pool, &SyncQueue::new(), edit.clone())
        .await
        .unwrap();
    assert_eq!(same.updated_at, "original-version");
    edit.title = "Changed".into();
    let changed = save_event(&pool, &SyncQueue::new(), edit).await.unwrap();
    assert_ne!(changed.updated_at, "original-version");
}

#[tokio::test]
async fn sync_during_a_put_cannot_be_overwritten_by_its_late_local_commit() {
    let pool = pool().await;
    let hook_pool = pool.clone();
    let (server, request) = dav_server_with_hook(204, move || async move {
        let newer = raw(&hook_pool).await.replace("SUMMARY:Planning", "SUMMARY:Newer sync");
        sqlx::query("UPDATE calendar_events SET title = 'Newer sync', raw_ics = ?, remote_etag = '\"v3\"', updated_at = 'new-version' WHERE id = 'event'")
            .bind(newer).execute(&hook_pool).await.unwrap();
    }).await;
    remote_event(&pool, &server).await;
    let mut edit = input();
    edit.title = "My title".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::RemoteCommittedLocalFailed(
            CalendarError::EventChanged
        ))
    ));
    request.await.unwrap();
    let current = CalendarRepository::new(&pool)
        .event_edit_state("a", "event")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.event.title, "Newer sync");
    assert_eq!(current.remote_etag.as_deref(), Some("\"v3\""));
    assert!(current.raw_ics.unwrap().contains("SUMMARY:Newer sync"));
}

#[tokio::test]
async fn concurrent_local_edits_of_one_revision_have_a_single_winner() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let current = CalendarRepository::new(&pool)
        .get_event("a", "event")
        .await
        .unwrap();
    let mut first = input();
    first.expected_updated_at = Some(current.updated_at.clone());
    first.title = "First".into();
    let mut second = first.clone();
    second.title = "Second".into();
    let queue = SyncQueue::new();
    let (first, second) = tokio::join!(
        save_event(&pool, &queue, first),
        save_event(&pool, &queue, second)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let loser = if first.is_err() { first } else { second };
    assert!(matches!(
        loser,
        Err(SaveError::Calendar(CalendarError::EventChanged))
    ));
}

#[tokio::test]
async fn embedded_timezone_definitions_survive_text_edits_but_block_time_changes() {
    let pool = pool().await;
    create_event(&pool, input()).await;
    let timezone = "BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0300\r\nTZOFFSETTO:+0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";
    let original = raw(&pool)
        .await
        .replace("BEGIN:VEVENT", &format!("{timezone}BEGIN:VEVENT"))
        .replace(
            "DTSTART:20260328T100000Z",
            "DTSTART;TZID=Europe/Berlin:20260328T110000",
        )
        .replace(
            "DTEND:20260328T110000Z",
            "DTEND;TZID=Europe/Berlin:20260328T120000",
        );
    sqlx::query("UPDATE calendar_events SET raw_ics = ? WHERE id = 'event'")
        .bind(&original)
        .execute(&pool)
        .await
        .unwrap();
    let mut edit = input();
    edit.title = "New title".into();
    save_event(&pool, &SyncQueue::new(), edit.clone())
        .await
        .unwrap();
    let before_move = raw(&pool).await;
    assert!(before_move.contains(timezone));
    assert!(before_move.contains("SUMMARY:New title"));

    // A resource's definition can differ from the system's IANA database even
    // when its TZID has a familiar name. Do not reinterpret that definition.
    edit.starts_at = "2026-03-28T12:00:00Z".into();
    edit.ends_at = "2026-03-28T13:00:00Z".into();
    assert!(matches!(
        save_event(&pool, &SyncQueue::new(), edit).await,
        Err(SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));
    assert_eq!(raw(&pool).await, before_move);
}

#[tokio::test]
async fn all_day_sync_writes_keep_date_boundaries_and_editor_revision() {
    let pool = pool().await;
    let mut edit = input();
    edit.all_day = true;
    edit.starts_at = "2026-03-28".into();
    edit.ends_at = "2026-03-31".into();
    create_event(&pool, edit.clone()).await;
    sqlx::query("UPDATE calendar_events SET updated_at = 'original-version' WHERE id = 'event'")
        .execute(&pool)
        .await
        .unwrap();
    let repository = CalendarRepository::new(&pool);
    let state = repository
        .event_edit_state("a", "event")
        .await
        .unwrap()
        .unwrap();
    let mirrored = repository
        .upsert_event(CalendarEventWrite {
            id: "event",
            account_id: "a",
            uid: &state.event.uid,
            title: &state.event.title,
            description: state.event.description.as_deref(),
            location: state.event.location.as_deref(),
            starts_at: "2026-03-28T00:00:00Z",
            ends_at: "2026-03-31T00:00:00Z",
            all_day: true,
            attendees_json: &state.event.attendees_json,
            source: &state.event.source,
            remote_href: state.remote_href.as_deref(),
            remote_etag: state.remote_etag.as_deref(),
            collection_href: state.event.collection_href.as_deref(),
            raw_ics: state.raw_ics.as_deref(),
        })
        .await
        .unwrap();
    assert_eq!(mirrored.starts_at, "2026-03-28");
    assert_eq!(mirrored.ends_at, "2026-03-31");
    assert_eq!(mirrored.updated_at, "original-version");
    edit.expected_updated_at = Some(mirrored.updated_at);
    edit.title = "After sync".into();
    assert_eq!(
        save_event(&pool, &SyncQueue::new(), edit)
            .await
            .unwrap()
            .title,
        "After sync"
    );
}
