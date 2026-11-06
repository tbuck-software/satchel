use super::*;

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn busy(start: &str, end: &str) -> Busy {
    Busy {
        start: at(start),
        end: at(end),
        tentative: false,
    }
}

fn weekdays() -> WorkingHours {
    WorkingHours::new(9 * 60, 17 * 60, &[1, 2, 3, 4, 5]).unwrap()
}

fn berlin() -> Tz {
    parse_zone("Europe/Berlin").unwrap()
}

/// Windows as Berlin wall-clock text, one entry per day.
fn local(days: &[FreeDay], zone: Tz) -> Vec<(String, Vec<String>)> {
    days.iter()
        .map(|day| {
            (
                day.date.format("%a %d").to_string(),
                day.windows
                    .iter()
                    .map(|(start, end)| {
                        format!(
                            "{}-{}",
                            start.with_timezone(&zone).format("%H:%M"),
                            end.with_timezone(&zone).format("%H:%M")
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn working_hours_reject_empty_reversed_and_out_of_range_values() {
    assert_eq!(
        WorkingHours::new(17 * 60, 9 * 60, &[1]),
        Err(AvailabilityError::WorkingHours)
    );
    assert_eq!(
        WorkingHours::new(0, 24 * 60 + 1, &[1]),
        Err(AvailabilityError::WorkingHours)
    );
    assert_eq!(
        WorkingHours::new(9 * 60, 17 * 60, &[]),
        Err(AvailabilityError::WorkingHours)
    );
    assert_eq!(
        WorkingHours::new(9 * 60, 17 * 60, &[8]),
        Err(AvailabilityError::WorkingHours)
    );
    assert!(WorkingHours::new(0, 24 * 60, &[7]).is_ok());
}

#[test]
fn merge_joins_overlapping_and_touching_blocks() {
    let merged = merge(&[
        busy("2026-11-09T10:00:00Z", "2026-11-09T11:00:00Z"),
        busy("2026-11-09T09:00:00Z", "2026-11-09T10:00:00Z"),
        busy("2026-11-09T10:30:00Z", "2026-11-09T10:45:00Z"),
        busy("2026-11-09T13:00:00Z", "2026-11-09T14:00:00Z"),
        busy("2026-11-09T15:00:00Z", "2026-11-09T15:00:00Z"),
    ]);
    assert_eq!(
        merged,
        vec![
            (at("2026-11-09T09:00:00Z"), at("2026-11-09T11:00:00Z")),
            (at("2026-11-09T13:00:00Z"), at("2026-11-09T14:00:00Z")),
        ]
    );
}

#[test]
fn free_days_skip_weekends_and_cut_busy_time_out_of_working_hours() {
    // Friday 6 November 2026, 07:00 in Berlin.
    let from = at("2026-11-06T06:00:00Z");
    let blocks = [
        // 10:00–11:00 and 13:10–14:50 Berlin (UTC+1).
        busy("2026-11-06T09:00:00Z", "2026-11-06T10:00:00Z"),
        busy("2026-11-06T12:10:00Z", "2026-11-06T13:50:00Z"),
        // Monday all morning.
        busy("2026-11-09T07:30:00Z", "2026-11-09T11:00:00Z"),
    ];
    let days = free_days(
        &blocks,
        berlin(),
        &weekdays(),
        from,
        3,
        Duration::minutes(30),
    )
    .unwrap();
    assert_eq!(
        local(&days, berlin()),
        vec![
            (
                "Fri 06".into(),
                vec![
                    "09:00-10:00".into(),
                    "11:00-13:00".into(),
                    "15:00-17:00".into()
                ]
            ),
            ("Mon 09".into(), vec!["12:00-17:00".into()]),
            ("Tue 10".into(), vec!["09:00-17:00".into()]),
        ]
    );
}

#[test]
fn today_starts_at_the_next_quarter_hour_and_counts_only_with_time_left() {
    let zone = berlin();
    // 14:07 Berlin: the day continues from 14:15.
    let days = free_days(
        &[],
        zone,
        &weekdays(),
        at("2026-11-06T13:07:00Z"),
        1,
        Duration::minutes(60),
    )
    .unwrap();
    assert_eq!(
        local(&days, zone),
        vec![("Fri 06".into(), vec!["14:15-17:00".into()])]
    );

    // 16:20 Berlin with a 60-minute meeting: today is over, Monday is first.
    let days = free_days(
        &[],
        zone,
        &weekdays(),
        at("2026-11-06T15:20:00Z"),
        1,
        Duration::minutes(60),
    )
    .unwrap();
    assert_eq!(
        local(&days, zone),
        vec![("Mon 09".into(), vec!["09:00-17:00".into()])]
    );
}

#[test]
fn a_fully_booked_working_day_still_counts_as_one_of_the_days() {
    let zone = berlin();
    let blocks = [busy("2026-11-09T08:00:00Z", "2026-11-09T16:00:00Z")];
    let days = free_days(
        &blocks,
        zone,
        &weekdays(),
        at("2026-11-08T12:00:00Z"),
        2,
        Duration::minutes(30),
    )
    .unwrap();
    assert_eq!(
        local(&days, zone),
        vec![
            ("Mon 09".into(), vec![]),
            ("Tue 10".into(), vec!["09:00-17:00".into()]),
        ]
    );
}

#[test]
fn short_gaps_and_ragged_edges_snap_to_the_quarter_hour() {
    let zone = parse_zone("UTC").unwrap();
    let blocks = [
        busy("2026-11-09T09:00:00Z", "2026-11-09T09:50:00Z"),
        // A 25-minute gap after 09:50 is too short once snapped (10:00–10:15).
        busy("2026-11-09T10:15:00Z", "2026-11-09T12:10:00Z"),
    ];
    let days = free_days(
        &blocks,
        zone,
        &weekdays(),
        at("2026-11-09T00:00:00Z"),
        1,
        Duration::minutes(30),
    )
    .unwrap();
    assert_eq!(
        local(&days, zone),
        vec![("Mon 09".into(), vec!["12:15-17:00".into()])]
    );
}

#[test]
fn working_hours_keep_their_wall_clock_across_the_spring_change() {
    // Berlin moves from UTC+1 to UTC+2 on Sunday 29 March 2026 at 02:00.
    let zone = berlin();
    let hours = WorkingHours::new(9 * 60, 17 * 60, &[5, 1]).unwrap();
    let days = free_days(
        &[],
        zone,
        &hours,
        at("2026-03-27T00:00:00Z"),
        2,
        Duration::minutes(60),
    )
    .unwrap();
    assert_eq!(
        days[0].windows,
        vec![(at("2026-03-27T08:00:00Z"), at("2026-03-27T16:00:00Z"))]
    );
    assert_eq!(
        days[1].windows,
        vec![(at("2026-03-30T07:00:00Z"), at("2026-03-30T15:00:00Z"))]
    );
}

#[test]
fn a_start_inside_the_skipped_hour_moves_to_the_first_minute_that_exists() {
    let zone = berlin();
    // Sunday hours from 02:30, which does not exist on 29 March 2026.
    let hours = WorkingHours::new(2 * 60 + 30, 5 * 60, &[7]).unwrap();
    let days = free_days(
        &[],
        zone,
        &hours,
        at("2026-03-28T12:00:00Z"),
        1,
        Duration::minutes(30),
    )
    .unwrap();
    // 03:00 CEST is 01:00 UTC; 05:00 CEST is 03:00 UTC.
    assert_eq!(
        days[0].windows,
        vec![(at("2026-03-29T01:00:00Z"), at("2026-03-29T03:00:00Z"))]
    );
}

#[test]
fn hours_through_the_repeated_autumn_hour_cover_both_passes_of_it() {
    // Berlin goes back from 03:00 CEST to 02:00 CET on 25 October 2026.
    let zone = berlin();
    let hours = WorkingHours::new(60, 2 * 60 + 30, &[7]).unwrap();
    let days = free_days(
        &[],
        zone,
        &hours,
        at("2026-10-24T12:00:00Z"),
        1,
        Duration::minutes(30),
    )
    .unwrap();
    // 01:00 CEST (23:00 UTC) until the later 02:30, CET (01:30 UTC): 2½ hours of wall clock, 2½ + 1 of time.
    assert_eq!(
        days[0].windows,
        vec![(at("2026-10-24T23:00:00Z"), at("2026-10-25T01:30:00Z"))]
    );
    // The regular weekday on the Monday after is back on UTC+1.
    let days = free_days(
        &[],
        zone,
        &weekdays(),
        at("2026-10-25T12:00:00Z"),
        1,
        Duration::minutes(30),
    )
    .unwrap();
    assert_eq!(
        days[0].windows,
        vec![(at("2026-10-26T08:00:00Z"), at("2026-10-26T16:00:00Z"))]
    );
}

#[test]
fn days_follow_the_working_hours_zone_not_utc() {
    // 23:30 UTC on Sunday is already Monday morning in Tokyo.
    let zone = parse_zone("Asia/Tokyo").unwrap();
    let days = free_days(
        &[],
        zone,
        &weekdays(),
        at("2026-11-08T23:30:00Z"),
        1,
        Duration::minutes(30),
    )
    .unwrap();
    assert_eq!(days[0].date.to_string(), "2026-11-09");
    assert_eq!(
        days[0].windows,
        vec![(at("2026-11-09T00:00:00Z"), at("2026-11-09T08:00:00Z"))]
    );
}

#[test]
fn free_days_reject_impossible_requests() {
    let from = at("2026-11-06T06:00:00Z");
    assert_eq!(
        free_days(&[], berlin(), &weekdays(), from, 3, Duration::zero()),
        Err(AvailabilityError::Duration)
    );
    assert_eq!(
        free_days(&[], berlin(), &weekdays(), from, 0, Duration::minutes(30)),
        Err(AvailabilityError::Range)
    );
    assert_eq!(parse_zone("Mars/Olympus"), Err(AvailabilityError::TimeZone));
}

#[test]
fn an_event_without_transp_is_busy_whole_day_or_not() {
    let event = |props: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\n{props}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    // RFC 5545 §3.8.2.7: OPAQUE unless said otherwise. A whole-day
    // "Vacation" without TRANSP blocks its days.
    assert!(!ics_transparent(&event("DTSTART:20261109T090000Z\r\n")));
    assert!(!ics_transparent(&event(
        "SUMMARY:Vacation\r\nDTSTART;VALUE=DATE:20261109\r\n"
    )));
    assert!(ics_transparent(&event("TRANSP:TRANSPARENT\r\n")));
    assert!(!ics_transparent(&event("TRANSP:OPAQUE\r\n")));
    assert!(ics_transparent(&event("STATUS:CANCELLED\r\n")));
    // An alarm's properties are not the event's.
    assert!(!ics_transparent(&event(
        "BEGIN:VALARM\r\nTRANSP:TRANSPARENT\r\nEND:VALARM\r\n"
    )));
    // Folded and lower-case input.
    assert!(ics_transparent(&event("transp:TRANS\r\n PARENT\r\n")));
}

#[test]
fn a_meeting_the_owner_declined_is_recognised_by_any_of_their_addresses() {
    let event = |attendee: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nORGANIZER:mailto:boss@example.com\r\n\
             ATTENDEE;PARTSTAT=ACCEPTED:mailto:other@example.com\r\n{attendee}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    let own = [
        "me@example.com".to_string(),
        "alias@example.org".to_string(),
    ];
    assert!(ics_declined_by(
        &event("ATTENDEE;CN=Me;PARTSTAT=DECLINED:mailto:Me@Example.com\r\n"),
        &own
    ));
    assert!(ics_declined_by(
        &event("ATTENDEE;PARTSTAT=\"DECLINED\";RSVP=TRUE:MAILTO:alias@example.org\r\n"),
        &own
    ));
    assert!(!ics_declined_by(
        &event("ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\n"),
        &own
    ));
    // A quoted name may hold `:` and `;`.
    assert!(ics_declined_by(
        &event("ATTENDEE;CN=\"Doe; Jane: Ops\";PARTSTAT=DECLINED:mailto:me@example.com\r\n"),
        &own
    ));
    assert!(!ics_declined_by(
        &event("ATTENDEE;CN=\"PARTSTAT=DECLINED:\";PARTSTAT=ACCEPTED:mailto:me@example.com\r\n"),
        &own
    ));
    // Someone else declining changes nothing for the owner.
    assert!(!ics_declined_by(
        &event("ATTENDEE;PARTSTAT=DECLINED:mailto:other2@example.com\r\n"),
        &own
    ));
}

async fn pool() -> SqlitePool {
    crate::utils::crypto::seed_master_key_for_tests();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::init(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES ('a', 'Me', 'me@example.com', 'imap', '{}', '', '{}', '', 1, 'now', 'now')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[allow(clippy::too_many_arguments)]
async fn event(
    pool: &SqlitePool,
    id: &str,
    source: &str,
    collection: Option<&str>,
    starts_at: &str,
    ends_at: &str,
    all_day: bool,
    transparent: bool,
    raw_ics: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO calendar_events
            (id, account_id, uid, title, starts_at, ends_at, all_day, source, collection_href,
             transparent, raw_ics, created_at, updated_at)
         VALUES (?, 'a', ?, 'x', ?, ?, ?, ?, ?, ?, ?, 'now', 'now')",
    )
    .bind(id)
    .bind(id)
    .bind(starts_at)
    .bind(ends_at)
    .bind(all_day)
    .bind(source)
    .bind(collection)
    .bind(transparent)
    .bind(raw_ics)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn own_busy_reads_every_counting_calendar_and_skips_free_time() {
    let pool = pool().await;
    let zone = berlin();
    event(
        &pool,
        "local",
        "local",
        None,
        "2026-11-09T09:00:00+00:00",
        "2026-11-09T10:00:00+00:00",
        false,
        false,
        None,
    )
    .await;
    event(
        &pool,
        "free",
        "local",
        None,
        "2026-11-09T11:00:00Z",
        "2026-11-09T12:00:00Z",
        false,
        true,
        None,
    )
    .await;
    event(
        &pool,
        "work",
        "google",
        Some("work@example.com"),
        "2026-11-09T13:00:00Z",
        "2026-11-09T14:00:00Z",
        false,
        false,
        None,
    )
    .await;
    event(
        &pool,
        "family",
        "google",
        Some("family@group.calendar.google.com"),
        "2026-11-09T15:00:00Z",
        "2026-11-09T16:00:00Z",
        false,
        false,
        None,
    )
    .await;
    // A whole-day opaque event covers the Berlin day, 23:00 UTC to 23:00 UTC.
    event(
        &pool,
        "trip",
        "outlook",
        Some("AAMk"),
        "2026-11-10",
        "2026-11-11",
        true,
        false,
        None,
    )
    .await;
    // Outside the range.
    event(
        &pool,
        "later",
        "local",
        None,
        "2026-11-20T09:00:00Z",
        "2026-11-20T10:00:00Z",
        false,
        false,
        None,
    )
    .await;
    // A daily series that began a month earlier.
    let series = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:standup\r\nDTSTART:20261001T080000Z\r\nDTEND:20261001T081500Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    event(
        &pool,
        "standup",
        "caldav",
        Some("https://dav.example/cal/"),
        "2026-10-01T08:00:00Z",
        "2026-10-01T08:15:00Z",
        false,
        false,
        Some(series),
    )
    .await;
    // An accepted invitation that no calendar mirrors yet, and a maybe.
    for (id, status, start, end) in [
        (
            "inv-yes",
            "accepted",
            "2026-11-09T16:30:00Z",
            "2026-11-09T17:00:00Z",
        ),
        (
            "inv-maybe",
            "tentative",
            "2026-11-09T17:00:00Z",
            "2026-11-09T17:30:00Z",
        ),
        (
            "inv-open",
            "needs-action",
            "2026-11-09T18:00:00Z",
            "2026-11-09T19:00:00Z",
        ),
    ] {
        sqlx::query(
            "INSERT INTO calendar_invites (id, account_id, uid, starts_at, ends_at, all_day, rsvp_status, raw_ics, method, created_at, updated_at)
             VALUES (?, 'a', ?, ?, ?, 0, ?, '', 'REQUEST', 'now', 'now')",
        )
        .bind(id)
        .bind(id)
        .bind(start)
        .bind(end)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
    }

    let blocks = own_busy(
        &pool,
        at("2026-11-09T00:00:00Z"),
        at("2026-11-11T00:00:00Z"),
        zone,
        &["google:family@group.calendar.google.com".to_string()],
        None,
    )
    .await
    .unwrap();
    let spans = blocks
        .iter()
        .map(|block| {
            (
                block.start.to_rfc3339(),
                block.end.to_rfc3339(),
                block.tentative,
            )
        })
        .collect::<Vec<_>>();
    let span = |start: &str, end: &str, tentative: bool| {
        (at(start).to_rfc3339(), at(end).to_rfc3339(), tentative)
    };
    assert_eq!(
        spans,
        vec![
            span("2026-11-09T08:00:00Z", "2026-11-09T08:15:00Z", false),
            span("2026-11-09T09:00:00Z", "2026-11-09T10:00:00Z", false),
            span("2026-11-09T13:00:00Z", "2026-11-09T14:00:00Z", false),
            span("2026-11-09T16:30:00Z", "2026-11-09T17:00:00Z", false),
            span("2026-11-09T17:00:00Z", "2026-11-09T17:30:00Z", true),
            span("2026-11-09T23:00:00Z", "2026-11-10T23:00:00Z", false),
            span("2026-11-10T08:00:00Z", "2026-11-10T08:15:00Z", false),
        ]
    );
}

#[test]
fn calendar_keys_match_the_frontend_source_keys() {
    assert_eq!(
        calendar_key("google", Some("me@example.com")),
        "google:me@example.com"
    );
    assert_eq!(calendar_key("google", None), "google:primary");
    assert_eq!(calendar_key("outlook", None), "outlook:default");
    assert_eq!(
        calendar_key("caldav", Some("https://dav.example/cal/")),
        "caldav"
    );
    assert_eq!(calendar_key("local", None), "local");
}

#[tokio::test]
async fn the_edited_event_and_its_mirror_do_not_make_the_owner_busy() {
    let pool = pool().await;
    event(
        &pool,
        "edited",
        "local",
        None,
        "2026-11-09T09:00:00Z",
        "2026-11-09T10:00:00Z",
        false,
        false,
        None,
    )
    .await;
    // The same meeting mirrored from another calendar: same UID, same start.
    sqlx::query(
        "INSERT INTO calendar_events (id, account_id, uid, title, starts_at, ends_at, source, collection_href, created_at, updated_at)
         VALUES ('mirror', 'a', 'edited', 'x', '2026-11-09T09:00:00Z', '2026-11-09T10:00:00Z', 'google', 'work@example.com', 'now', 'now')",
    )
    .execute(&pool)
    .await
    .unwrap();
    event(
        &pool,
        "other",
        "local",
        None,
        "2026-11-09T11:00:00Z",
        "2026-11-09T12:00:00Z",
        false,
        false,
        None,
    )
    .await;

    let range = (at("2026-11-09T00:00:00Z"), at("2026-11-10T00:00:00Z"));
    let all = own_busy(&pool, range.0, range.1, berlin(), &[], None)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    let editing = own_busy(&pool, range.0, range.1, berlin(), &[], Some("edited"))
        .await
        .unwrap();
    assert_eq!(
        editing.iter().map(|block| block.start).collect::<Vec<_>>(),
        vec![at("2026-11-09T11:00:00Z")]
    );
}

#[tokio::test]
async fn moved_skipped_and_declined_occurrences_count_where_they_now_are() {
    let pool = pool().await;
    let series = concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\nUID:weekly\r\n",
        "DTSTART:20261102T090000Z\r\nDURATION:PT30M\r\n",
        "RRULE:FREQ=WEEKLY;COUNT=10\r\n",
        "EXDATE:20261116T090000Z\r\n",
        "END:VEVENT\r\n",
        // 9 November moved to the afternoon.
        "BEGIN:VEVENT\r\nUID:weekly\r\nRECURRENCE-ID:20261109T090000Z\r\n",
        "DTSTART:20261109T140000Z\r\nDTEND:20261109T150000Z\r\nEND:VEVENT\r\n",
        // 23 November: still on, but the owner declined this one.
        "BEGIN:VEVENT\r\nUID:weekly\r\nRECURRENCE-ID:20261123T090000Z\r\n",
        "DTSTART:20261123T090000Z\r\nDTEND:20261123T093000Z\r\n",
        "ATTENDEE;PARTSTAT=DECLINED:mailto:me@example.com\r\nEND:VEVENT\r\n",
        // 30 November: marked free.
        "BEGIN:VEVENT\r\nUID:weekly\r\nRECURRENCE-ID:20261130T090000Z\r\n",
        "DTSTART:20261130T090000Z\r\nDTEND:20261130T093000Z\r\nTRANSP:TRANSPARENT\r\nEND:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );
    event(
        &pool,
        "weekly",
        "caldav",
        Some("https://dav.example/cal/"),
        "2026-11-02T09:00:00Z",
        "2026-11-02T09:30:00Z",
        false,
        false,
        Some(series),
    )
    .await;

    let blocks = own_busy(
        &pool,
        at("2026-11-08T00:00:00Z"),
        at("2026-12-08T00:00:00Z"),
        berlin(),
        &[],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        blocks
            .iter()
            .map(|block| (block.start.to_rfc3339(), block.end.to_rfc3339()))
            .collect::<Vec<_>>(),
        vec![
            (
                "2026-11-09T14:00:00+00:00".to_string(),
                "2026-11-09T15:00:00+00:00".to_string()
            ),
            (
                "2026-12-07T09:00:00+00:00".to_string(),
                "2026-12-07T09:30:00+00:00".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn a_series_with_every_occurrence_excluded_takes_no_time() {
    let pool = pool().await;
    let series = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:gone\r\n",
        "DTSTART:20261109T090000Z\r\nDTEND:20261109T100000Z\r\n",
        "RRULE:FREQ=DAILY;COUNT=2\r\nEXDATE:20261109T090000Z\r\n",
        "END:VEVENT\r\n",
        "BEGIN:VEVENT\r\nUID:gone\r\nRECURRENCE-ID:20261110T090000Z\r\n",
        "DTSTART:20261110T090000Z\r\nDTEND:20261110T100000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );
    event(
        &pool,
        "gone",
        "caldav",
        Some("https://dav.example/cal/"),
        "2026-11-09T09:00:00Z",
        "2026-11-09T10:00:00Z",
        false,
        false,
        Some(series),
    )
    .await;
    // A rule the expander does not follow still counts its first occurrence.
    let unsupported = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:last-weekday\r\n",
        "DTSTART:20261109T130000Z\r\nDTEND:20261109T140000Z\r\n",
        "RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n",
    );
    event(
        &pool,
        "last-weekday",
        "caldav",
        Some("https://dav.example/cal/"),
        "2026-11-09T13:00:00Z",
        "2026-11-09T14:00:00Z",
        false,
        false,
        Some(unsupported),
    )
    .await;

    let blocks = own_busy(
        &pool,
        at("2026-11-09T00:00:00Z"),
        at("2026-11-12T00:00:00Z"),
        berlin(),
        &[],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        blocks.iter().map(|block| block.start).collect::<Vec<_>>(),
        vec![at("2026-11-09T13:00:00Z")]
    );
}
