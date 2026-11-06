use super::*;

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

async fn pool() -> SqlitePool {
    crate::utils::crypto::seed_master_key_for_tests();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::init(&pool).await.unwrap();
    let config = crate::accounts::encrypt_account_config("imap", "{}").unwrap();
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('imap', 'Me', 'Me@Example.com', 'imap', ?, '', '{}', '', 1, 'now', 'now')",
    )
    .bind(config)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO calendar_events (id, account_id, uid, title, starts_at, ends_at, source, created_at, updated_at)
         VALUES ('e', 'imap', 'e', 'Review', '2026-11-09T09:00:00Z', '2026-11-09T11:00:00Z', 'local', 'now', 'now')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[test]
fn addresses_are_normalized_deduplicated_and_never_the_owner() {
    let input = [
        " <Lena@Example.com> ".to_string(),
        "lena@example.com".to_string(),
        "me@example.com".to_string(),
        "not an address".to_string(),
        "Bo@Example.org".to_string(),
    ];
    assert_eq!(
        normalize_addresses(&input, &["ME@example.com".to_string()]),
        vec!["lena@example.com", "bo@example.org"]
    );
}

#[test]
fn only_plain_addresses_are_asked_about() {
    assert_eq!(
        parse_address("Lena Vogel <Lena@Example.com>").as_deref(),
        Some("lena@example.com")
    );
    assert_eq!(
        parse_address(" bo@example.org ").as_deref(),
        Some("bo@example.org")
    );
    for hostile in [
        "a@example.com\r\nATTENDEE:mailto:x@evil.test",
        "a@example.com\nX-INJECT:1",
        "a b@example.com",
        "a@exa mple.com",
        "<a@example.com",
        "Name <a@example.com> trailing",
        "a@b@example.com",
        "a;x=y@example.com",
        "a@localhost",
        "@example.com",
        "a@",
    ] {
        assert_eq!(parse_address(hostile), None, "{hostile:?}");
    }
}

#[test]
fn provider_ranges_cover_whole_utc_days() {
    let (from, to) = day_range(at("2026-11-09T13:20:00Z"), at("2026-11-10T00:00:00Z"));
    assert_eq!(from, at("2026-11-09T00:00:00Z"));
    assert_eq!(to, at("2026-11-10T00:00:00Z"));
    let (from, to) = day_range(at("2026-11-09T00:00:00Z"), at("2026-11-11T00:00:01Z"));
    assert_eq!(from, at("2026-11-09T00:00:00Z"));
    assert_eq!(to, at("2026-11-12T00:00:00Z"));
    // Never more than Graph's 62 days.
    let (from, to) = day_range(at("2026-11-09T12:00:00Z"), at("2027-01-10T12:00:00Z"));
    assert_eq!(to - from, Duration::days(62));
}

#[tokio::test]
async fn the_edited_event_does_not_make_its_attendees_busy_at_its_own_slot() {
    clear_cache();
    let pool = pool().await;
    let (from, to) = day_range(at("2026-11-09T00:00:00Z"), at("2026-11-10T00:00:00Z"));
    let block = |start: &str, end: &str| Busy {
        start: at(start),
        end: at(end),
        tentative: false,
    };
    CACHE.lock().unwrap().insert(
        (
            "imap".into(),
            "lena@example.com".into(),
            from.timestamp(),
            to.timestamp(),
        ),
        CacheEntry {
            at: Instant::now(),
            ttl: ANSWER_TTL,
            busy: Some(vec![
                block("2026-11-09T09:00:00Z", "2026-11-09T11:00:00Z"),
                block("2026-11-09T13:00:00Z", "2026-11-09T14:00:00Z"),
            ]),
        },
    );
    let attendees = ["lena@example.com".to_string()];
    let ask = |editing| {
        let attendees = &attendees;
        let pool = &pool;
        async move {
            query(
                pool,
                &Participants {
                    account_id: Some("imap"),
                    attendees,
                    excluded_calendars: &[],
                    editing,
                },
                at("2026-11-09T00:00:00Z"),
                at("2026-11-10T00:00:00Z"),
                "UTC",
            )
            .await
            .unwrap()
        }
    };
    let plain = ask(None).await;
    assert_eq!(plain.own.len(), 1);
    assert_eq!(plain.attendees[0].busy.len(), 2);

    // Editing event 'e' (09:00–11:00): neither the owner nor Lena is busy with it.
    let editing = ask(Some(Editing {
        event_id: "e",
        starts_at: Some(at("2026-11-09T09:00:00Z")),
        ends_at: Some(at("2026-11-09T11:00:00Z")),
    }))
    .await;
    assert!(editing.own.is_empty());
    assert_eq!(
        editing.attendees[0].busy,
        vec![BusyBlock {
            starts_at: "2026-11-09T13:00:00Z".into(),
            ends_at: "2026-11-09T14:00:00Z".into(),
            tentative: false,
        }]
    );
}

#[tokio::test]
async fn an_account_without_free_busy_reports_attendees_as_unknown_not_as_an_error() {
    clear_cache();
    let pool = pool().await;
    let attendees = ["lena@example.com".to_string(), "me@example.com".to_string()];
    let report = query(
        &pool,
        &Participants {
            account_id: Some("imap"),
            attendees: &attendees,
            excluded_calendars: &[],
            editing: None,
        },
        at("2026-11-09T00:00:00Z"),
        at("2026-11-10T00:00:00Z"),
        "Europe/Berlin",
    )
    .await
    .unwrap();
    assert_eq!(
        report.own,
        vec![BusyBlock {
            starts_at: "2026-11-09T09:00:00Z".into(),
            ends_at: "2026-11-09T11:00:00Z".into(),
            tentative: false,
        }]
    );
    assert_eq!(
        report.attendees,
        vec![AttendeeFreeBusy {
            address: "lena@example.com".into(),
            known: false,
            busy: vec![],
        }]
    );
}

#[tokio::test]
async fn free_slots_leave_out_the_owner_busy_time_and_name_unknown_people() {
    clear_cache();
    let pool = pool().await;
    let hours = WorkingHours::new(9 * 60, 17 * 60, &[1, 2, 3, 4, 5]).unwrap();
    let attendees = ["lena@example.com".to_string()];
    let report = free_slots(
        &pool,
        &Participants {
            account_id: Some("imap"),
            attendees: &attendees,
            excluded_calendars: &[],
            editing: None,
        },
        // Sunday evening in London.
        at("2026-11-08T20:00:00Z"),
        1,
        60,
        &hours,
        "Europe/London",
    )
    .await
    .unwrap();
    assert_eq!(report.time_zone, "Europe/London");
    assert_eq!(report.unknown, vec!["lena@example.com"]);
    assert_eq!(
        report.days,
        vec![FreeDay {
            date: "2026-11-09".into(),
            windows: vec![FreeWindow {
                starts_at: "2026-11-09T11:00:00Z".into(),
                ends_at: "2026-11-09T17:00:00Z".into(),
            }],
        }]
    );

    // A calendar switched off for availability no longer blocks the morning.
    let report = free_slots(
        &pool,
        &Participants {
            account_id: None,
            attendees: &[],
            excluded_calendars: &["local".to_string()],
            editing: None,
        },
        at("2026-11-08T20:00:00Z"),
        1,
        60,
        &hours,
        "Europe/London",
    )
    .await
    .unwrap();
    assert_eq!(report.days[0].windows[0].starts_at, "2026-11-09T09:00:00Z");
}

#[tokio::test]
async fn invalid_ranges_durations_and_zones_are_refused_with_keys() {
    let pool = pool().await;
    let participants = Participants {
        account_id: None,
        attendees: &[],
        excluded_calendars: &[],
        editing: None,
    };
    let error = query(
        &pool,
        &participants,
        at("2026-11-09T00:00:00Z"),
        at("2026-11-09T00:00:00Z"),
        "UTC",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FreeBusyError::Invalid(AvailabilityError::Range)
    ));
    let error = query(
        &pool,
        &participants,
        at("2026-11-09T00:00:00Z"),
        at("2027-11-09T00:00:00Z"),
        "UTC",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FreeBusyError::Invalid(AvailabilityError::Range)
    ));
    let error = query(
        &pool,
        &participants,
        at("2026-11-09T00:00:00Z"),
        at("2026-11-10T00:00:00Z"),
        "Nowhere/Here",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FreeBusyError::Invalid(AvailabilityError::TimeZone)
    ));
    let hours = WorkingHours::new(9 * 60, 17 * 60, &[1]).unwrap();
    let error = free_slots(
        &pool,
        &participants,
        at("2026-11-09T00:00:00Z"),
        3,
        2,
        &hours,
        "UTC",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FreeBusyError::Invalid(AvailabilityError::Duration)
    ));
    let error = free_slots(
        &pool,
        &participants,
        at("2026-11-09T00:00:00Z"),
        21,
        30,
        &hours,
        "UTC",
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        FreeBusyError::Invalid(AvailabilityError::Range)
    ));
}
