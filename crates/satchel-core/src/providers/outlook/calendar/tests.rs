//! Outlook calendar sync and writes against a scripted Graph.

use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone)]
struct Request {
    method: String,
    /// Path and query, as sent.
    target: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Value,
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body,
    }
}

type Route = Box<dyn Fn(&Request, &str) -> Reply + Send + Sync>;

struct FakeGraph {
    base: String,
    log: Arc<Mutex<Vec<Request>>>,
    route: Arc<Mutex<Route>>,
}

impl FakeGraph {
    async fn start(route: impl Fn(&Request, &str) -> Reply + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1.0", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<Request>>> = Arc::default();
        let route: Arc<Mutex<Route>> = Arc::new(Mutex::new(Box::new(route)));
        let (server_log, server_route, server_base) = (log.clone(), route.clone(), base.clone());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (log, route, base) = (
                    server_log.clone(),
                    server_route.clone(),
                    server_base.clone(),
                );
                tokio::spawn(async move {
                    while let Some(request) = read_request(&mut socket).await {
                        log.lock().unwrap().push(request.clone());
                        let answer = (route.lock().unwrap())(&request, &base);
                        let payload = answer.body.to_string();
                        let mut head = format!(
                            "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                            answer.status,
                            payload.len()
                        );
                        for (name, value) in &answer.headers {
                            head.push_str(&format!("{name}: {value}\r\n"));
                        }
                        head.push_str("\r\n");
                        if socket
                            .write_all(format!("{head}{payload}").as_bytes())
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        TEST_API_BASE.with(|slot| *slot.borrow_mut() = Some(base.clone()));
        Self { base, log, route }
    }

    fn reroute(&self, route: impl Fn(&Request, &str) -> Reply + Send + Sync + 'static) {
        *self.route.lock().unwrap() = Box::new(route);
    }

    fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }

    fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }

    fn graph(&self) -> Graph {
        Graph {
            client: reqwest::Client::new(),
            base: self.base.clone(),
            token: "calendar-token".into(),
        }
    }
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_string();
    let target = request_line.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = serde_json::from_slice(&buffer[header_end..]).unwrap_or(Value::Null);
    Some(Request {
        method,
        target,
        headers,
        body,
    })
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-07-01T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

const CALENDAR_SCOPES: &str =
    "openid offline_access https://graph.microsoft.com/Mail.ReadWrite https://graph.microsoft.com/Calendars.ReadWrite";

async fn database(scope: &str) -> SqlitePool {
    crate::utils::crypto::seed_master_key_for_tests();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::init(&pool).await.unwrap();
    let config = json!({"oauth": {
        "provider": "outlook", "client_id": "fixture-client",
        "access_token": "calendar-token", "refresh_token": "refresh",
        "scope": scope, "expires_at": "2099-01-01T00:00:00Z"
    }})
    .to_string();
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES ('account', 'Fixture', 'me@example.test', 'outlook', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')",
    )
    .bind(crate::accounts::encrypt_account_config("account", &config).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    pool
}

fn calendars() -> Value {
    json!({ "value": [
        { "id": "cal-team", "name": "Team", "isDefaultCalendar": false, "canEdit": true },
        { "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true, "canEdit": true },
    ]})
}

fn event(id: &str, subject: &str, start: &str, end: &str) -> Value {
    json!({
        "id": id,
        "changeKey": format!("{id}-1"),
        "iCalUId": format!("uid-{id}"),
        "subject": subject,
        "body": { "contentType": "html", "content": "<html><body><div><p>Agenda</p><p>Points</p></div></body></html>" },
        "location": { "displayName": "Room 1" },
        "start": { "dateTime": start, "timeZone": "UTC" },
        "end": { "dateTime": end, "timeZone": "UTC" },
        "isAllDay": false,
        "isOrganizer": true,
        "type": "singleInstance",
        "attendees": [{ "emailAddress": { "address": "alice@example.test" } }],
    })
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct Row {
    title: String,
    starts_at: String,
    ends_at: String,
    all_day: i64,
    collection_href: Option<String>,
    remote_href: Option<String>,
    remote_etag: Option<String>,
    uid: String,
    read_only: i64,
    recurrence_instance: i64,
    rich_description: i64,
}

async fn rows(pool: &SqlitePool) -> Vec<Row> {
    sqlx::query_as::<_, Row>(
        "SELECT title, starts_at, ends_at, all_day, collection_href, remote_href, remote_etag, uid,
                read_only, recurrence_instance, rich_description
         FROM calendar_events WHERE source = 'outlook' ORDER BY starts_at, remote_href",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn cursor_of(pool: &SqlitePool, calendar: &str) -> Option<DeltaCursor> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT sync_cursor FROM calendar_sources WHERE source = 'outlook' AND remote_id = ?",
    )
    .bind(calendar)
    .fetch_optional(pool)
    .await
    .unwrap()
    .flatten();
    raw.map(|raw| serde_json::from_str(&raw).unwrap())
}

const DEFAULT_DELTA: &str = "/v1.0/me/calendarView/delta?startDateTime=";
const TEAM_LISTING: &str = "/v1.0/me/calendars/cal-team/calendarView?startDateTime=";

fn team_listing(events: Value) -> Reply {
    reply(200, json!({ "value": events }))
}

/// The first pass: the default calendar through its delta, paged, with a
/// timed event, an all-day event and two occurrences of a series; the team
/// calendar as a listing.
fn initial_route(request: &Request, base: &str) -> Reply {
    let target = request.target.as_str();
    if target.starts_with("/v1.0/me/calendars?") {
        return reply(200, calendars());
    }
    if target.starts_with(DEFAULT_DELTA) {
        return reply(
            200,
            json!({
                "value": [
                    event("evt-1", "Planning", "2026-07-14T14:00:00.0000000", "2026-07-14T15:00:00.0000000"),
                    {
                        "id": "evt-allday", "changeKey": "allday-1", "subject": "Holiday",
                        "isAllDay": true, "isOrganizer": true,
                        "start": { "dateTime": "2026-07-19T22:00:00.0000000", "timeZone": "UTC" },
                        "end": { "dateTime": "2026-07-20T22:00:00.0000000", "timeZone": "UTC" },
                    },
                ],
                "@odata.nextLink": format!("{base}/me/calendarView/delta?$skiptoken=page2"),
            }),
        );
    }
    if target.ends_with("$skiptoken=page2") {
        let occurrence = |id: &str, day: u32| {
            let mut value = event(
                id,
                "Standup",
                &format!("2026-07-{day}T08:00:00.0000000"),
                &format!("2026-07-{day}T08:15:00.0000000"),
            );
            value["iCalUId"] = json!("uid-series");
            value["type"] = json!("occurrence");
            value
        };
        return reply(
            200,
            json!({
                "value": [occurrence("occ-1", 15), occurrence("occ-2", 16)],
                "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=main-1"),
            }),
        );
    }
    if target.starts_with(TEAM_LISTING) {
        return team_listing(json!([event(
            "team-1",
            "Review",
            "2026-07-17T09:00:00.0000000",
            "2026-07-17T10:00:00.0000000"
        )]));
    }
    reply(
        404,
        json!({ "error": { "code": "unexpected", "message": target } }),
    )
}

#[tokio::test]
async fn the_first_pass_mirrors_every_calendar_and_expands_series_into_occurrences() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(initial_route).await;

    let count = sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();

    assert_eq!(count, 5);
    let mirrored = rows(&pool).await;
    let summary: Vec<_> = mirrored
        .iter()
        .map(|row| {
            (
                row.title.as_str(),
                row.starts_at.as_str(),
                row.collection_href.as_deref().unwrap(),
                row.recurrence_instance,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Planning", "2026-07-14T14:00:00Z", "cal-main", 0),
            ("Standup", "2026-07-15T08:00:00Z", "cal-main", 1),
            ("Standup", "2026-07-16T08:00:00Z", "cal-main", 1),
            ("Review", "2026-07-17T09:00:00Z", "cal-team", 0),
            // A Berlin all-day event converted to UTC still means its date.
            ("Holiday", "2026-07-20", "cal-main", 0),
        ]
    );
    let holiday = mirrored.iter().find(|row| row.title == "Holiday").unwrap();
    assert_eq!(
        (holiday.all_day, holiday.ends_at.as_str()),
        (1, "2026-07-21")
    );
    let planning = &mirrored[0];
    assert_eq!(planning.remote_href.as_deref(), Some("evt-1"));
    // The mirror keeps Graph's changeKey, which every write compares.
    assert_eq!(planning.remote_etag.as_deref(), Some("evt-1-1"));
    assert_eq!(planning.uid, "uid-evt-1");
    // A typed note wrapped in <div>/<p> is plain enough to edit as text.
    assert_eq!((planning.read_only, planning.rich_description), (0, 0));
    let (description, location, attendees): (Option<String>, Option<String>, String) =
        sqlx::query_as(
            "SELECT description, location, attendees_json FROM calendar_events WHERE remote_href = 'evt-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(description.as_deref(), Some("Agenda\nPoints"));
    assert_eq!(location.as_deref(), Some("Room 1"));
    assert_eq!(attendees, r#"["alice@example.test"]"#);

    let sources = CalendarRepository::new(&pool)
        .list_sources(Some("account"))
        .await
        .unwrap();
    let names: Vec<_> = sources
        .iter()
        .map(|source| {
            (
                source.source.as_str(),
                source.remote_id.as_str(),
                source.name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("outlook", "cal-main", "Calendar"),
            ("outlook", "cal-team", "Team")
        ]
    );
    let cursor = cursor_of(&pool, "cal-main").await.unwrap();
    assert!(cursor.delta_link.ends_with("$deltatoken=main-1"));
    assert_eq!(cursor.window_start, "2026-04-02T12:00:00+00:00");
    // A listing has nothing to resume from.
    assert!(cursor_of(&pool, "cal-team").await.is_none());

    let requests = graph.requests();
    let first_delta = requests
        .iter()
        .find(|request| request.target.starts_with(DEFAULT_DELTA))
        .unwrap();
    assert!(first_delta
        .target
        .contains("startDateTime=2026-04-02T12%3A00%3A00Z&endDateTime=2027-07-01T12%3A00%3A00Z"));
    let prefer = first_delta.header("Prefer").unwrap();
    assert!(prefer.contains("IdType=\"ImmutableId\""), "{prefer}");
    assert!(prefer.contains("outlook.timezone=\"UTC\""), "{prefer}");
    assert!(prefer.contains("odata.maxpagesize=100"), "{prefer}");
    assert_eq!(
        first_delta.header("Authorization"),
        Some("Bearer calendar-token")
    );
    // No per-calendar delta: v1.0 does not document one.
    assert!(requests.iter().all(|request| !request
        .target
        .contains("/calendars/cal-team/calendarView/delta")));
    let listing = requests
        .iter()
        .find(|request| request.target.starts_with(TEAM_LISTING))
        .unwrap();
    assert!(listing.target.contains("%24top=100"), "{}", listing.target);
    assert!(
        listing
            .target
            .contains("%24select=id%2CiCalUId%2CchangeKey"),
        "{}",
        listing.target
    );
    assert!(!listing.header("Prefer").unwrap().contains("maxpagesize"));
}

#[tokio::test]
async fn a_later_pass_applies_only_what_changed_since_the_delta_link() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(initial_route).await;
    sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();
    let untouched_revision: String =
        sqlx::query_scalar("SELECT updated_at FROM calendar_events WHERE remote_href = 'occ-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    graph.take_requests();

    graph.reroute(|request, base| {
        let target = request.target.as_str();
        if target.starts_with("/v1.0/me/calendars?") {
            return reply(200, calendars());
        }
        if target.ends_with("$deltatoken=main-1") {
            let mut moved = event(
                "evt-1",
                "Planning (moved)",
                "2026-07-14T16:00:00.0000000",
                "2026-07-14T17:00:00.0000000",
            );
            moved["changeKey"] = json!("evt-1-2");
            return reply(
                200,
                json!({
                    "value": [
                        moved,
                        { "id": "occ-2", "@removed": { "reason": "deleted" } },
                        { "id": "evt-allday", "isCancelled": true, "subject": "Holiday",
                          "start": { "dateTime": "2026-07-20T00:00:00", "timeZone": "UTC" } },
                        event("evt-new", "Lunch", "2026-07-18T11:00:00.0000000", "2026-07-18T12:00:00.0000000"),
                    ],
                    "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=main-2"),
                }),
            );
        }
        if target.starts_with(TEAM_LISTING) {
            return team_listing(json!([event(
                "team-1",
                "Review",
                "2026-07-17T09:00:00.0000000",
                "2026-07-17T10:00:00.0000000"
            )]));
        }
        reply(404, json!({ "error": { "code": "unexpected", "message": target } }))
    });

    let count = sync_with(
        &pool,
        "account",
        &graph.graph(),
        now() + chrono::Duration::hours(1),
    )
    .await
    .unwrap();

    assert_eq!(count, 4);
    let titles: Vec<_> = rows(&pool)
        .await
        .into_iter()
        .map(|row| (row.title, row.starts_at, row.remote_etag))
        .collect();
    assert_eq!(
        titles,
        [
            (
                "Planning (moved)".into(),
                "2026-07-14T16:00:00Z".into(),
                Some("evt-1-2".into())
            ),
            (
                "Standup".into(),
                "2026-07-15T08:00:00Z".into(),
                Some("occ-1-1".into())
            ),
            (
                "Review".into(),
                "2026-07-17T09:00:00Z".into(),
                Some("team-1-1".into())
            ),
            (
                "Lunch".into(),
                "2026-07-18T11:00:00Z".into(),
                Some("evt-new-1".into())
            ),
        ]
    );
    // Rows the pass did not mention keep their revision for open editors.
    let revision: String =
        sqlx::query_scalar("SELECT updated_at FROM calendar_events WHERE remote_href = 'occ-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(revision, untouched_revision);
    assert!(cursor_of(&pool, "cal-main")
        .await
        .unwrap()
        .delta_link
        .ends_with("main-2"));
    // The default calendar resumed instead of starting over.
    assert!(graph
        .requests()
        .iter()
        .all(|request| !request.target.starts_with(DEFAULT_DELTA)));
}

#[tokio::test]
async fn an_expired_delta_link_starts_the_calendar_over() {
    for (status, body) in [
        (
            410,
            json!({ "error": { "code": "SyncStateNotFound", "message": "expired" } }),
        ),
        (
            400,
            json!({ "error": { "code": "ErrorInvalidSyncStateData", "message": "SyncStateInvalid; resyncRequired" } }),
        ),
        (404, json!({ "error": { "code": "ErrorItemNotFound" } })),
    ] {
        let pool = database(CALENDAR_SCOPES).await;
        let graph = FakeGraph::start(initial_route).await;
        sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap();

        graph.reroute(move |request, base| {
            let target = request.target.as_str();
            if target.starts_with("/v1.0/me/calendars?") {
                return reply(200, calendars());
            }
            if target.ends_with("$deltatoken=main-1") {
                return reply(status, body.clone());
            }
            if target.starts_with(DEFAULT_DELTA) {
                return reply(
                    200,
                    json!({
                        "value": [event("evt-1", "Planning", "2026-07-14T14:00:00.0000000", "2026-07-14T15:00:00.0000000")],
                        "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=main-fresh"),
                    }),
                );
            }
            if target.starts_with(TEAM_LISTING) {
                return team_listing(json!([]));
            }
            reply(404, json!({ "error": { "code": "unexpected", "message": target } }))
        });

        let count = sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap();

        // Everything the fresh enumeration no longer lists is gone.
        assert_eq!(count, 1, "after {status}");
        assert_eq!(rows(&pool).await[0].title, "Planning");
        assert!(cursor_of(&pool, "cal-main")
            .await
            .unwrap()
            .delta_link
            .ends_with("main-fresh"));
    }
}

#[tokio::test]
async fn a_week_of_window_drift_starts_over_and_a_removed_calendar_takes_its_events() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(initial_route).await;
    sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();
    graph.take_requests();

    graph.reroute(|request, base| {
        let target = request.target.as_str();
        if target.starts_with("/v1.0/me/calendars?") {
            return reply(
                200,
                json!({ "value": [{ "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true, "canEdit": true }] }),
            );
        }
        if target.starts_with(DEFAULT_DELTA) {
            return reply(
                200,
                json!({
                    "value": [],
                    "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=main-later"),
                }),
            );
        }
        reply(404, json!({ "error": { "code": "unexpected", "message": target } }))
    });

    let later = now() + chrono::Duration::days(WINDOW_DRIFT_DAYS + 1);
    let count = sync_with(&pool, "account", &graph.graph(), later)
        .await
        .unwrap();

    assert_eq!(count, 0);
    let sources = CalendarRepository::new(&pool)
        .list_sources(Some("account"))
        .await
        .unwrap();
    assert_eq!(sources.len(), 1);
    assert!(graph
        .requests()
        .iter()
        .all(|request| !request.target.contains("deltatoken=main-1")));
}

#[tokio::test]
async fn a_throttled_page_is_fetched_again_after_retry_after() {
    let pool = database(CALENDAR_SCOPES).await;
    let throttled = Arc::new(Mutex::new(false));
    let graph = FakeGraph::start(move |request, base| {
        if request.target.contains("cal-team") && !*throttled.lock().unwrap() {
            *throttled.lock().unwrap() = true;
            return Reply {
                status: 429,
                headers: vec![("Retry-After", "1".into())],
                body: json!({ "error": { "code": "ApplicationThrottled" } }),
            };
        }
        initial_route(request, base)
    })
    .await;

    let count = sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();

    assert_eq!(count, 5);
    let team_requests = graph
        .requests()
        .iter()
        .filter(|request| request.target.contains("cal-team"))
        .count();
    assert_eq!(team_requests, 2);
}

#[tokio::test]
async fn one_failing_calendar_does_not_keep_the_others_stale() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(|request, base| {
        if request.target.contains("cal-team") {
            return reply(500, json!({ "error": { "code": "InternalServerError" } }));
        }
        initial_route(request, base)
    })
    .await;

    let error = sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap_err();

    assert!(!error.is_permanent(), "{error}");
    assert_eq!(rows(&pool).await.len(), 4);
}

#[tokio::test]
async fn a_calendar_graph_will_not_show_is_skipped_without_failing_the_account() {
    for status in [403, 404] {
        let pool = database(CALENDAR_SCOPES).await;
        let graph = FakeGraph::start(move |request, base| {
            if request.target.starts_with("/v1.0/me/calendars?") {
                return reply(
                    200,
                    json!({ "value": [
                        { "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true, "canEdit": true },
                        { "id": "cal-birthdays", "name": "Birthdays", "canEdit": false },
                    ]}),
                );
            }
            if request.target.contains("cal-birthdays") {
                return reply(status, json!({ "error": { "code": "ErrorAccessDenied" } }));
            }
            initial_route(request, base)
        })
        .await;

        let count = sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap();

        assert_eq!(count, 4, "after {status}");
    }
}

#[tokio::test]
async fn the_default_calendar_is_listed_when_graph_refuses_its_delta() {
    for status in [400, 404, 405, 501] {
        let pool = database(CALENDAR_SCOPES).await;
        let graph = FakeGraph::start(move |request, base| {
            let target = request.target.as_str();
            if target.starts_with(DEFAULT_DELTA) {
                return reply(status, json!({ "error": { "code": "BadRequest" } }));
            }
            if target.starts_with("/v1.0/me/calendars/cal-main/calendarView?startDateTime=") {
                return team_listing(json!([event(
                    "evt-1",
                    "Planning",
                    "2026-07-14T14:00:00.0000000",
                    "2026-07-14T15:00:00.0000000"
                )]));
            }
            initial_route(request, base)
        })
        .await;

        assert_eq!(
            sync_with(&pool, "account", &graph.graph(), now())
                .await
                .unwrap(),
            2,
            "after {status}"
        );
        assert!(cursor_of(&pool, "cal-main").await.is_none());
    }
}

#[tokio::test]
async fn a_listed_calendar_loses_what_it_no_longer_holds() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(initial_route).await;
    sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();

    graph.reroute(|request, base| {
        let target = request.target.as_str();
        if target.starts_with(TEAM_LISTING) {
            return team_listing(json!([]));
        }
        if target.ends_with("$deltatoken=main-1") {
            return reply(
                200,
                json!({ "value": [], "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=main-2") }),
            );
        }
        initial_route(request, base)
    });

    assert_eq!(
        sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap(),
        4
    );
    assert!(rows(&pool).await.iter().all(|row| row.title != "Review"));
}

#[tokio::test]
async fn paging_links_off_the_graph_api_are_refused() {
    for link in [
        "https://attacker.example/steal".to_string(),
        // Same host, outside the API root.
        "{base_origin}/v1.0evil/me/calendars".to_string(),
        "{base_origin}/other/me/calendars".to_string(),
    ] {
        let pool = database(CALENDAR_SCOPES).await;
        let graph = FakeGraph::start(move |request, base| {
            if request.target.starts_with("/v1.0/me/calendars?") {
                let origin = base.trim_end_matches("/v1.0");
                return reply(
                    200,
                    json!({ "value": [{ "id": "cal-main", "name": "Calendar" }],
                            "@odata.nextLink": link.replace("{base_origin}", origin) }),
                );
            }
            initial_route(request, base)
        })
        .await;

        let error = sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("leaves the Graph API"),
            "{error}"
        );
        assert_eq!(graph.requests().len(), 1);
    }
}

#[tokio::test]
async fn a_mail_only_account_is_skipped_without_a_single_request() {
    let pool = database("openid offline_access https://graph.microsoft.com/Mail.ReadWrite").await;
    let graph = FakeGraph::start(initial_route).await;

    assert_eq!(sync_account(&pool, "account").await, Ok(0));
    let fields = EventFields {
        title: "Lunch",
        description: None,
        location: None,
        starts_at: now(),
        ends_at: now() + chrono::Duration::hours(1),
        all_day: false,
    };
    assert!(matches!(
        create_event(&pool, "account", &fields, &[]).await,
        Err(WriteError::NotGranted)
    ));
    assert!(graph.requests().is_empty());
    assert!(rows(&pool).await.is_empty());
}

#[tokio::test]
async fn sync_account_reaches_graph_once_calendar_access_is_granted() {
    let pool = database(CALENDAR_SCOPES).await;
    let _graph = FakeGraph::start(initial_route).await;

    assert_eq!(sync_account(&pool, "account").await, Ok(5));
}

#[test]
fn shared_read_only_calendars_and_meetings_one_attends_are_read_only() {
    let parse = |value: Value| serde_json::from_value::<GraphEvent>(value).unwrap();
    let organized = event("a", "Mine", "2026-07-14T14:00:00", "2026-07-14T15:00:00");
    let mut attended = organized.clone();
    attended["isOrganizer"] = json!(false);
    let mut unknown = organized.clone();
    unknown.as_object_mut().unwrap().remove("isOrganizer");

    let flags = |can_edit: bool, value: &Value| {
        map_event("account", can_edit, parse(value.clone()))
            .unwrap()
            .flags
    };
    assert!(!flags(true, &organized).read_only);
    assert!(flags(false, &organized).read_only);
    assert!(flags(true, &attended).read_only);
    assert!(flags(true, &unknown).read_only);
}

#[test]
fn formatted_bodies_are_marked_and_plain_ones_are_not() {
    for content in [
        "<html><head><meta charset=\"utf-8\"></head><body dir=\"ltr\"><div>Bring snacks<br></div></body></html>",
        "<p class=\"x\">Agenda</p><p><span>Points</span></p>",
        "<!-- converted from text --><div>Notes</div>",
        "Plain words, no tags",
    ] {
        assert!(!is_rich_html(content), "plain: {content}");
    }
    for content in [
        "<p>Join: <a href=\"https://teams.example/l/meetup\">Click here</a></p>",
        // A link whose attributes start on the next line.
        "<p><a\nhref=\"https://teams.example\">Join</a></p>",
        "<p><span style=\"color:red\">Points</span></p>",
        "<div style = \"font-family: Aptos\">Text</div>",
        "<p STYLE=\"margin:0\">Text</p>",
        "<table><tr><td>Agenda</td></tr></table>",
        "<ul><li>One</li></ul>",
        "<p><b>Important</b></p>",
        "<font face=\"Arial\">Old</font>",
        "<p><s>gone</s> <strike>gone</strike> H<sub>2</sub>O x<sup>2</sup></p>",
        "<p><code>cargo test</code></p>",
        "<h5>Heading</h5>",
        "<dl><dt>Term</dt></dl>",
        "<img src=\"cid:logo\">",
        "<head><style>p{margin:0}</style></head><body><div>Text</div></body>",
        "<div>Unclosed <",
    ] {
        assert!(is_rich_html(content), "rich: {content}");
    }
    let mut value = event("a", "Mine", "2026-07-14T14:00:00", "2026-07-14T15:00:00");
    value["body"] = json!({ "contentType": "text", "content": "<a href> as typed" });
    let mapped = map_event("account", true, serde_json::from_value(value).unwrap()).unwrap();
    assert!(!mapped.flags.rich_description, "text bodies are text");
}

#[test]
fn an_empty_subject_stays_empty_for_the_frontend_to_name() {
    let mut value = event("a", "  ", "2026-07-14T14:00:00", "2026-07-14T15:00:00");
    let mapped = map_event(
        "account",
        true,
        serde_json::from_value(value.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(mapped.title, "");
    value.as_object_mut().unwrap().remove("subject");
    let mapped = map_event("account", true, serde_json::from_value(value).unwrap()).unwrap();
    assert_eq!(mapped.title, "");
}

/// A Graph holding events by id, for writes: it reports `changeKey`, bumps it
/// on every PATCH and — like v1.0 as documented — ignores `If-Match`.
fn stateful_graph(store: Arc<Mutex<Vec<Value>>>) -> impl Fn(&Request, &str) -> Reply + Send + Sync {
    move |request, _| {
        let mut events = store.lock().unwrap();
        let target = request.target.as_str();
        let event_id = |target: &str| {
            target
                .trim_start_matches("/v1.0/me/events/")
                .split('?')
                .next()
                .unwrap()
                .to_string()
        };
        match request.method.as_str() {
            "GET" if target.starts_with("/v1.0/me/calendar?") => {
                reply(200, json!({ "id": "cal-main" }))
            }
            "GET" if target.starts_with("/v1.0/me/events/") => {
                let id = event_id(target);
                match events.iter().find(|event| event["id"] == id.as_str()) {
                    Some(event) => reply(200, json!({ "id": id, "changeKey": event["changeKey"] })),
                    None => reply(404, json!({ "error": { "code": "ErrorItemNotFound" } })),
                }
            }
            "POST" if target == "/v1.0/me/calendars/cal-main/events" => {
                let transaction = request.body["transactionId"].clone();
                if let Some(existing) = events
                    .iter()
                    .find(|event| event["transactionId"] == transaction)
                {
                    return reply(201, existing.clone());
                }
                let mut created = request.body.clone();
                created["id"] = json!(format!("evt-{}", events.len() + 1));
                created["changeKey"] = json!("ck-1");
                created["isOrganizer"] = json!(true);
                events.push(created.clone());
                reply(201, created)
            }
            "PATCH" => {
                let id = event_id(target);
                let Some(event) = events.iter_mut().find(|event| event["id"] == id.as_str()) else {
                    return reply(404, json!({ "error": { "code": "ErrorItemNotFound" } }));
                };
                for (key, value) in request.body.as_object().unwrap() {
                    event[key] = value.clone();
                }
                let version = event["changeKey"].as_str().unwrap().len();
                event["changeKey"] = json!(format!("ck-{version}"));
                reply(200, event.clone())
            }
            "DELETE" => {
                let id = event_id(target);
                events.retain(|event| event["id"] != id.as_str());
                reply(204, Value::Null)
            }
            _ => reply(404, json!({})),
        }
    }
}

fn lunch() -> EventFields<'static> {
    EventFields {
        title: "Lunch",
        description: Some("Bring snacks"),
        location: Some("Canteen"),
        starts_at: now(),
        ends_at: now() + chrono::Duration::hours(1),
        all_day: false,
    }
}

#[tokio::test]
async fn new_events_go_to_the_default_calendar_and_invite_only_when_asked() {
    let pool = database(CALENDAR_SCOPES).await;
    let store: Arc<Mutex<Vec<Value>>> = Arc::default();
    let graph = FakeGraph::start(stateful_graph(store.clone())).await;

    let created = create_event(&pool, "account", &lunch(), &[]).await.unwrap();

    assert_eq!(created.calendar_id, "cal-main");
    assert_eq!(created.event.remote_id, "evt-1");
    assert_eq!(created.event.id, row_id("account", "evt-1"));
    assert_eq!(created.event.change_key.as_deref(), Some("ck-1"));
    assert_eq!(created.event.starts_at, "2026-07-01T12:00:00Z");
    let mut post = graph
        .requests()
        .into_iter()
        .find(|request| request.method == "POST")
        .unwrap();
    assert!(uuid::Uuid::parse_str(post.body["transactionId"].as_str().unwrap()).is_ok());
    post.body.as_object_mut().unwrap().remove("transactionId");
    assert_eq!(
        post.body,
        json!({
            "subject": "Lunch",
            "body": { "contentType": "text", "content": "Bring snacks" },
            "location": { "displayName": "Canteen" },
            "start": { "dateTime": "2026-07-01T12:00:00", "timeZone": "UTC" },
            "end": { "dateTime": "2026-07-01T13:00:00", "timeZone": "UTC" },
            "isAllDay": false,
        })
    );

    graph.take_requests();
    let all_day = EventFields {
        all_day: true,
        starts_at: now(),
        ends_at: now() + chrono::Duration::days(1),
        ..lunch()
    };
    create_event(&pool, "account", &all_day, &["bob@example.test".into()])
        .await
        .unwrap();
    let post = graph
        .requests()
        .into_iter()
        .find(|request| request.method == "POST")
        .unwrap();
    assert_eq!(
        post.body["start"],
        json!({ "dateTime": "2026-07-01T00:00:00", "timeZone": "UTC" })
    );
    assert_eq!(
        post.body["attendees"],
        json!([{ "emailAddress": { "address": "bob@example.test" }, "type": "required" }])
    );
}

#[tokio::test]
async fn a_throttled_create_is_sent_again_with_the_same_transaction_id() {
    let pool = database(CALENDAR_SCOPES).await;
    let store: Arc<Mutex<Vec<Value>>> = Arc::default();
    let inner = stateful_graph(store.clone());
    let throttled = Arc::new(Mutex::new(false));
    let graph = FakeGraph::start(move |request, base| {
        if request.method == "POST" && !*throttled.lock().unwrap() {
            *throttled.lock().unwrap() = true;
            // The first POST lands, but its answer is lost to throttling.
            inner(request, base);
            return Reply {
                status: 429,
                headers: vec![("Retry-After", "1".into())],
                body: json!({ "error": { "code": "ApplicationThrottled" } }),
            };
        }
        inner(request, base)
    })
    .await;

    let created = create_event(&pool, "account", &lunch(), &[]).await.unwrap();

    let posts: Vec<_> = graph
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST")
        .map(|request| request.body["transactionId"].clone())
        .collect();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0], posts[1]);
    assert_eq!(store.lock().unwrap().len(), 1, "one event, not two");
    assert_eq!(created.event.remote_id, "evt-1");
}

fn seed(store: &Arc<Mutex<Vec<Value>>>, id: &str, change_key: &str) {
    let mut value = event(
        id,
        "Planning",
        "2026-07-14T14:00:00.0000000",
        "2026-07-14T15:00:00.0000000",
    );
    value["changeKey"] = json!(change_key);
    store.lock().unwrap().push(value);
}

#[tokio::test]
async fn writes_compare_the_change_key_and_never_send_if_match() {
    let pool = database(CALENDAR_SCOPES).await;
    let store: Arc<Mutex<Vec<Value>>> = Arc::default();
    seed(&store, "evt-1", "ck-1");
    let graph = FakeGraph::start(stateful_graph(store.clone())).await;
    let patch = json!({ "subject": "Planning v2" });

    // Matching key: the write goes through.
    let updated = update_event(&pool, "account", "cal-main", "evt-1", Some("ck-1"), &patch)
        .await
        .unwrap();
    assert_eq!(updated.event.title, "Planning v2");
    assert_eq!(updated.event.change_key.as_deref(), Some("ck-4"));
    let requests = graph.take_requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| (request.method.as_str(), request.target.as_str()))
            .collect::<Vec<_>>(),
        [
            ("GET", "/v1.0/me/events/evt-1?%24select=changeKey"),
            ("PATCH", "/v1.0/me/events/evt-1"),
        ]
    );
    assert!(requests
        .iter()
        .all(|request| request.header("If-Match").is_none()));

    // A stale key: nothing is written.
    let stale = update_event(&pool, "account", "cal-main", "evt-1", Some("ck-1"), &patch).await;
    assert!(matches!(stale, Err(WriteError::Changed)));
    let stale = delete_event(&pool, "account", "evt-1", Some("ck-1")).await;
    assert!(matches!(stale, Err(WriteError::Changed)));
    // No stored key proves nothing either.
    let unknown = update_event(&pool, "account", "cal-main", "evt-1", None, &patch).await;
    assert!(matches!(unknown, Err(WriteError::Changed)));
    assert!(graph
        .take_requests()
        .iter()
        .all(|request| request.method == "GET"));
    assert_eq!(store.lock().unwrap().len(), 1);

    // Gone: an edit has nothing to change, a deletion is already done.
    let gone = update_event(&pool, "account", "cal-main", "evt-x", Some("ck-1"), &patch).await;
    assert!(matches!(gone, Err(WriteError::Gone)));
    delete_event(&pool, "account", "evt-x", Some("ck-1"))
        .await
        .unwrap();
    assert!(graph
        .take_requests()
        .iter()
        .all(|request| request.method == "GET"));

    delete_event(&pool, "account", "evt-1", Some("ck-4"))
        .await
        .unwrap();
    assert!(store.lock().unwrap().is_empty());
}

#[tokio::test]
async fn graph_refusing_a_write_reads_as_forbidden() {
    let pool = database(CALENDAR_SCOPES).await;
    let _graph = FakeGraph::start(|request, _| {
        if request.method == "GET" {
            return reply(200, json!({ "changeKey": "ck-1" }));
        }
        reply(
            403,
            json!({ "error": { "code": "ErrorAccessDenied", "message": "Access is denied." } }),
        )
    })
    .await;

    let patch = json!({ "subject": "x" });
    assert!(matches!(
        update_event(&pool, "account", "cal-main", "evt-1", Some("ck-1"), &patch).await,
        Err(WriteError::Forbidden)
    ));
    assert!(matches!(
        delete_event(&pool, "account", "evt-1", Some("ck-1")).await,
        Err(WriteError::Forbidden)
    ));
}

#[test]
fn edits_send_only_changed_fields_and_never_a_formatted_body() {
    let previous = MirroredEventView {
        title: "Planning",
        description: Some("Agenda\nPoints"),
        location: Some("Room 1"),
        starts_at: "2026-07-14T14:00:00Z",
        ends_at: "2026-07-14T15:00:00Z",
        all_day: false,
        rich_description: false,
    };
    let start = chrono::DateTime::parse_from_rfc3339("2026-07-14T14:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let fields = EventFields {
        title: "Planning v2",
        description: Some("Agenda\nPoints"),
        location: Some("Room 1"),
        starts_at: start,
        ends_at: start + chrono::Duration::hours(1),
        all_day: false,
    };
    assert_eq!(
        patch_body(&previous, &fields),
        json!({ "subject": "Planning v2" })
    );

    let moved = EventFields {
        starts_at: start + chrono::Duration::hours(2),
        ends_at: start + chrono::Duration::hours(3),
        ..fields.clone()
    };
    let patch = patch_body(&previous, &moved);
    assert_eq!(
        patch["start"],
        json!({ "dateTime": "2026-07-14T16:00:00", "timeZone": "UTC" })
    );
    assert_eq!(patch["isAllDay"], json!(false));
    assert!(patch.get("body").is_none());

    let rewritten = EventFields {
        description: Some("New agenda"),
        ..fields.clone()
    };
    assert_eq!(
        patch_body(&previous, &rewritten)["body"],
        json!({ "contentType": "text", "content": "New agenda" })
    );
    assert!(!changes_rich_description(&previous, &rewritten));
    let formatted = MirroredEventView {
        rich_description: true,
        ..previous.clone()
    };
    assert!(patch_body(&formatted, &rewritten).get("body").is_none());
    assert!(changes_rich_description(&formatted, &rewritten));
    assert!(!changes_rich_description(&formatted, &fields));
}

#[test]
fn graph_times_become_utc_and_all_day_boundaries_become_dates() {
    let time = |date_time: &str, zone: Option<&str>| GraphDateTime {
        date_time: date_time.into(),
        time_zone: zone.map(str::to_string),
    };
    assert_eq!(
        normalize_time(&time("2026-07-14T16:00:00.0000000", Some("UTC")), false).as_deref(),
        Some("2026-07-14T16:00:00Z")
    );
    assert_eq!(
        normalize_time(&time("2026-07-14T16:00:00", Some("Europe/Berlin")), false).as_deref(),
        Some("2026-07-14T14:00:00Z")
    );
    assert_eq!(
        normalize_time(&time("2026-07-14T00:00:00.0000000", Some("UTC")), true).as_deref(),
        Some("2026-07-14")
    );
    assert_eq!(
        normalize_time(&time("2026-07-14T05:00:00.0000000", Some("UTC")), true).as_deref(),
        Some("2026-07-14")
    );
    assert_eq!(normalize_time(&time("tomorrow", None), false), None);
}

#[test]
fn refusals_are_told_apart_by_pass_and_status() {
    use PassKind::*;
    let kind = |outcome: Option<PassOutcome>| match outcome {
        None => "error",
        Some(PassOutcome::Expired) => "expired",
        Some(PassOutcome::Unsupported) => "unsupported",
        Some(PassOutcome::Inaccessible(_)) => "inaccessible",
        Some(PassOutcome::Done(_)) => "done",
    };
    assert_eq!(kind(refusal(Resume, true, 410, "")), "expired");
    assert_eq!(
        kind(refusal(
            Resume,
            false,
            400,
            r#"{"code":"SyncStateNotFound"}"#
        )),
        "expired"
    );
    assert_eq!(
        kind(refusal(Resume, true, 400, r#"{"code":"BadRequest"}"#)),
        "error"
    );
    assert_eq!(kind(refusal(Resume, true, 403, "")), "inaccessible");
    for status in [400, 404, 405, 501] {
        assert_eq!(
            kind(refusal(FreshDelta, true, status, "")),
            "unsupported",
            "{status}"
        );
    }
    for status in [401, 429, 500] {
        assert_eq!(
            kind(refusal(FreshDelta, true, status, "")),
            "error",
            "{status}"
        );
    }
    assert_eq!(kind(refusal(FreshDelta, true, 403, "")), "inaccessible");
    assert_eq!(kind(refusal(FreshDelta, false, 400, "")), "error");
    assert_eq!(kind(refusal(Listing, true, 404, "")), "inaccessible");
    assert_eq!(kind(refusal(Listing, true, 400, "")), "error");
}

#[test]
fn only_outlook_sign_ins_with_calendar_access_count_as_granted() {
    let config = |provider: &str, scope: &str| {
        json!({"oauth": {"provider": provider, "scope": scope}}).to_string()
    };
    assert!(calendar_granted(&config("outlook", CALENDAR_SCOPES)));
    assert!(!calendar_granted(&config(
        "outlook",
        "https://graph.microsoft.com/Mail.ReadWrite"
    )));
    assert!(!calendar_granted(&config("gmail", CALENDAR_SCOPES)));
    assert!(!calendar_granted(r#"{"caldav":{}}"#));
}

fn editor_input(
    id: Option<&str>,
    title: &str,
    start: &str,
    end: &str,
) -> crate::calendar::EventInput {
    crate::calendar::EventInput {
        id: id.map(str::to_string),
        expected_updated_at: None,
        account_id: "account".into(),
        title: title.into(),
        description: None,
        location: None,
        starts_at: start.into(),
        ends_at: end.into(),
        all_day: false,
        attendees: Vec::new(),
        send_invites: false,
    }
}

fn validation(
    result: Result<CalendarEventRecordAlias, crate::calendar::SaveError>,
) -> &'static str {
    match result {
        Err(crate::calendar::SaveError::Validation(key)) => key,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

type CalendarEventRecordAlias = crate::db::calendar::CalendarEventRecord;

#[tokio::test]
async fn the_editor_creates_edits_and_deletes_through_graph() {
    let pool = database(CALENDAR_SCOPES).await;
    let store: Arc<Mutex<Vec<Value>>> = Arc::default();
    let graph = FakeGraph::start(stateful_graph(store.clone())).await;
    let queue = crate::sync::queue::SyncQueue::new();

    let created = crate::calendar::save_event(
        &pool,
        &queue,
        editor_input(
            None,
            "Lunch",
            "2026-07-14T11:00:00Z",
            "2026-07-14T12:00:00Z",
        ),
    )
    .await
    .unwrap();
    assert_eq!(created.source, "outlook");
    assert_eq!(created.collection_href.as_deref(), Some("cal-main"));
    assert_eq!(created.id, row_id("account", "evt-1"));
    assert_eq!(created.read_only, 0);
    assert_eq!(store.lock().unwrap().len(), 1);

    let edited = crate::calendar::save_event(
        &pool,
        &queue,
        crate::calendar::EventInput {
            expected_updated_at: Some(created.updated_at.clone()),
            ..editor_input(
                Some(&created.id),
                "Long lunch",
                "2026-07-14T11:00:00Z",
                "2026-07-14T13:00:00Z",
            )
        },
    )
    .await
    .unwrap();
    assert_eq!(edited.title, "Long lunch");
    assert_eq!(edited.ends_at, "2026-07-14T13:00:00Z");
    let patch = graph
        .requests()
        .into_iter()
        .find(|request| request.method == "PATCH")
        .unwrap();
    assert_eq!(patch.header("If-Match"), None);
    assert_eq!(
        patch.body.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["end", "isAllDay", "start", "subject"]
    );

    // Changing guests needs scheduling messages the form does not send.
    let guests = crate::calendar::save_event(
        &pool,
        &queue,
        crate::calendar::EventInput {
            attendees: vec!["bob@example.test".into()],
            ..editor_input(
                Some(&created.id),
                "Long lunch",
                "2026-07-14T11:00:00Z",
                "2026-07-14T13:00:00Z",
            )
        },
    )
    .await;
    assert_eq!(validation(guests), "backend.errors.calendarEditUnsupported");

    crate::calendar::delete_event(&pool, &created.id)
        .await
        .unwrap();
    assert!(store.lock().unwrap().is_empty());
    assert!(rows(&pool).await.is_empty());
    // Nothing went through the mail queue: Exchange schedules on its own.
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(queued, 0);
}

/// Mirror one event through a sync pass, so the editor has a row to change.
async fn mirrored(pool: &SqlitePool, event: Value, can_edit: bool) -> String {
    let graph = FakeGraph::start(move |request, base| {
        let target = request.target.as_str();
        if target.starts_with("/v1.0/me/calendars?") {
            return reply(
                200,
                json!({ "value": [{ "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true, "canEdit": can_edit }] }),
            );
        }
        if target.starts_with(DEFAULT_DELTA) {
            return reply(
                200,
                json!({ "value": [event.clone()], "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=1") }),
            );
        }
        reply(404, json!({}))
    })
    .await;
    sync_with(pool, "account", &graph.graph(), now())
        .await
        .unwrap();
    rows(pool).await;
    sqlx::query_scalar("SELECT id FROM calendar_events WHERE source = 'outlook'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn events_one_may_not_change_are_refused_before_graph_is_asked() {
    for (organizer, can_edit) in [(false, true), (true, false)] {
        let pool = database(CALENDAR_SCOPES).await;
        let mut value = event(
            "evt-1",
            "Board meeting",
            "2026-07-14T14:00:00",
            "2026-07-14T15:00:00",
        );
        value["isOrganizer"] = json!(organizer);
        let id = mirrored(&pool, value, can_edit).await;
        assert_eq!(rows(&pool).await[0].read_only, 1);
        let graph = FakeGraph::start(|_, _| reply(500, json!({}))).await;

        let edit = crate::calendar::save_event(
            &pool,
            &crate::sync::queue::SyncQueue::new(),
            crate::calendar::EventInput {
                attendees: vec!["alice@example.test".into()],
                ..editor_input(
                    Some(&id),
                    "Renamed",
                    "2026-07-14T14:00:00Z",
                    "2026-07-14T15:00:00Z",
                )
            },
        )
        .await;
        assert_eq!(validation(edit), "backend.errors.calendarEventReadOnly");
        assert!(matches!(
            crate::calendar::delete_event(&pool, &id).await,
            Err(crate::calendar::SaveError::Validation(
                "backend.errors.calendarEventReadOnly"
            ))
        ));
        assert!(graph.requests().is_empty());
        assert_eq!(rows(&pool).await.len(), 1);
    }
}

#[tokio::test]
async fn a_formatted_description_cannot_be_rewritten_but_the_rest_can() {
    let pool = database(CALENDAR_SCOPES).await;
    let mut value = event(
        "evt-1",
        "Sync",
        "2026-07-14T14:00:00",
        "2026-07-14T15:00:00",
    );
    value["body"] = json!({ "contentType": "html",
        "content": "<p>Join: <a href=\"https://teams.example/l/meetup\">Microsoft Teams</a></p>" });
    let id = mirrored(&pool, value.clone(), true).await;
    assert_eq!(rows(&pool).await[0].rich_description, 1);
    let store: Arc<Mutex<Vec<Value>>> = Arc::default();
    store.lock().unwrap().push(value);
    let graph = FakeGraph::start(stateful_graph(store.clone())).await;
    let queue = crate::sync::queue::SyncQueue::new();
    let base = || crate::calendar::EventInput {
        attendees: vec!["alice@example.test".into()],
        description: Some("Join: Microsoft Teams".into()),
        location: Some("Room 1".into()),
        ..editor_input(
            Some(&id),
            "Sync",
            "2026-07-14T14:00:00Z",
            "2026-07-14T15:00:00Z",
        )
    };

    let rewrite = crate::calendar::save_event(
        &pool,
        &queue,
        crate::calendar::EventInput {
            description: Some("Plain words".into()),
            ..base()
        },
    )
    .await;
    assert_eq!(
        validation(rewrite),
        "backend.errors.calendarRichDescriptionReadOnly"
    );
    assert!(graph.take_requests().is_empty());

    crate::calendar::save_event(
        &pool,
        &queue,
        crate::calendar::EventInput {
            title: "Weekly sync".into(),
            ..base()
        },
    )
    .await
    .unwrap();
    let patch = graph
        .requests()
        .into_iter()
        .find(|request| request.method == "PATCH")
        .unwrap();
    assert_eq!(patch.body, json!({ "subject": "Weekly sync" }));
    assert!(store.lock().unwrap()[0]["body"]["content"]
        .as_str()
        .unwrap()
        .contains("<a href"));
}

#[tokio::test]
async fn a_conflict_resyncs_the_account_and_asks_for_a_retry() {
    let pool = database(CALENDAR_SCOPES).await;
    // Phase one mirrors the event; then someone changes it in Outlook. The
    // resync runs on the real clock, so it may resume or start over.
    let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let changed_in_route = changed.clone();
    let graph = FakeGraph::start(move |request, base| {
        let moved = changed_in_route.load(std::sync::atomic::Ordering::SeqCst);
        let target = request.target.as_str();
        if request.method == "GET" && target.starts_with("/v1.0/me/events/evt-1") {
            return reply(200, json!({ "changeKey": "evt-1-2" }));
        }
        if target.starts_with("/v1.0/me/calendars?") {
            return reply(
                200,
                json!({ "value": [{ "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true, "canEdit": true }] }),
            );
        }
        let current = if moved {
            let mut changed =
                event("evt-1", "Planning (moved)", "2026-07-14T16:00:00", "2026-07-14T17:00:00");
            changed["changeKey"] = json!("evt-1-2");
            changed
        } else {
            event("evt-1", "Planning", "2026-07-14T14:00:00", "2026-07-14T15:00:00")
        };
        if target.starts_with(DEFAULT_DELTA) || target.contains("$deltatoken=") {
            return reply(
                200,
                json!({
                    "value": [current],
                    "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=1"),
                }),
            );
        }
        reply(500, json!({}))
    })
    .await;
    sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT id FROM calendar_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    changed.store(true, std::sync::atomic::Ordering::SeqCst);
    graph.take_requests();

    let edit = crate::calendar::save_event(
        &pool,
        &crate::sync::queue::SyncQueue::new(),
        crate::calendar::EventInput {
            attendees: vec!["alice@example.test".into()],
            location: Some("Room 1".into()),
            ..editor_input(
                Some(&id),
                "Renamed",
                "2026-07-14T14:00:00Z",
                "2026-07-14T15:00:00Z",
            )
        },
    )
    .await;

    assert_eq!(
        validation(edit),
        "backend.errors.outlookEventChangedRemotely"
    );
    assert!(graph
        .requests()
        .iter()
        .all(|request| request.method == "GET"));
    // The mirror now holds Outlook's version for the retry.
    let rows = rows(&pool).await;
    assert_eq!(rows[0].title, "Planning (moved)");
    assert_eq!(rows[0].remote_etag.as_deref(), Some("evt-1-2"));
}

#[tokio::test]
async fn a_server_refusal_leaves_the_local_event_as_it_was() {
    let pool = database(CALENDAR_SCOPES).await;
    let store: Arc<Mutex<Vec<Value>>> = Arc::default();
    let graph = FakeGraph::start(stateful_graph(store.clone())).await;
    let queue = crate::sync::queue::SyncQueue::new();
    let created = crate::calendar::save_event(
        &pool,
        &queue,
        editor_input(
            None,
            "Lunch",
            "2026-07-14T11:00:00Z",
            "2026-07-14T12:00:00Z",
        ),
    )
    .await
    .unwrap();

    graph.reroute(|request, _| {
        if request.method == "GET" {
            return reply(200, json!({ "changeKey": "ck-1" }));
        }
        reply(
            500,
            json!({ "error": { "code": "ErrorInternalServerError" } }),
        )
    });
    let refused = crate::calendar::save_event(
        &pool,
        &queue,
        editor_input(
            Some(&created.id),
            "Renamed",
            "2026-07-14T11:00:00Z",
            "2026-07-14T12:00:00Z",
        ),
    )
    .await;
    assert!(matches!(
        refused,
        Err(crate::calendar::SaveError::Remote(_))
    ));
    assert!(crate::calendar::delete_event(&pool, &created.id)
        .await
        .is_err());
    // A 403 reads as "you may not change this", not as Graph's English.
    graph.reroute(|request, _| {
        if request.method == "GET" {
            return reply(200, json!({ "changeKey": "ck-1" }));
        }
        reply(
            403,
            json!({ "error": { "code": "ErrorAccessDenied", "message": "Access is denied." } }),
        )
    });
    let forbidden = crate::calendar::save_event(
        &pool,
        &queue,
        editor_input(
            Some(&created.id),
            "Renamed",
            "2026-07-14T11:00:00Z",
            "2026-07-14T12:00:00Z",
        ),
    )
    .await;
    assert_eq!(
        validation(forbidden),
        "backend.errors.calendarEventReadOnly"
    );
    let rows = rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Lunch");
}

#[tokio::test]
async fn a_mail_only_outlook_account_keeps_creating_local_events() {
    let pool = database("openid https://graph.microsoft.com/Mail.ReadWrite").await;
    let graph = FakeGraph::start(|_, _| reply(500, json!({}))).await;

    let created = crate::calendar::save_event(
        &pool,
        &crate::sync::queue::SyncQueue::new(),
        editor_input(
            None,
            "Lunch",
            "2026-07-14T11:00:00Z",
            "2026-07-14T12:00:00Z",
        ),
    )
    .await
    .unwrap();

    assert_eq!(created.source, "local");
    assert!(graph.requests().is_empty());
}
