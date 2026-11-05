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
        { "id": "cal-team", "name": "Team", "isDefaultCalendar": false },
        { "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true },
    ]})
}

fn event(id: &str, subject: &str, start: &str, end: &str) -> Value {
    json!({
        "id": id,
        "@odata.etag": format!("W/\"{id}-1\""),
        "iCalUId": format!("uid-{id}"),
        "subject": subject,
        "body": { "contentType": "html", "content": "<p>Agenda</p><p>Points</p>" },
        "location": { "displayName": "Room 1" },
        "start": { "dateTime": start, "timeZone": "UTC" },
        "end": { "dateTime": end, "timeZone": "UTC" },
        "isAllDay": false,
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
}

async fn rows(pool: &SqlitePool) -> Vec<Row> {
    sqlx::query_as::<_, Row>(
        "SELECT title, starts_at, ends_at, all_day, collection_href, remote_href, remote_etag, uid
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

/// The first pass: two calendars, the main one paged, with a timed event,
/// an all-day event and two occurrences of a series.
fn initial_route(request: &Request, base: &str) -> Reply {
    let target = request.target.as_str();
    if target.starts_with("/v1.0/me/calendars?") {
        return reply(200, calendars());
    }
    if target.starts_with("/v1.0/me/calendars/cal-main/calendarView/delta?startDateTime=") {
        return reply(
            200,
            json!({
                "value": [
                    event("evt-1", "Planning", "2026-07-14T14:00:00.0000000", "2026-07-14T15:00:00.0000000"),
                    {
                        "id": "evt-allday", "subject": "Holiday", "isAllDay": true,
                        "start": { "dateTime": "2026-07-19T22:00:00.0000000", "timeZone": "UTC" },
                        "end": { "dateTime": "2026-07-20T22:00:00.0000000", "timeZone": "UTC" },
                    },
                ],
                "@odata.nextLink": format!("{base}/me/calendars/cal-main/calendarView/delta?$skiptoken=page2"),
            }),
        );
    }
    if target.ends_with("$skiptoken=page2") {
        let mut first = event(
            "occ-1",
            "Standup",
            "2026-07-15T08:00:00.0000000",
            "2026-07-15T08:15:00.0000000",
        );
        first["iCalUId"] = json!("uid-series");
        let mut second = event(
            "occ-2",
            "Standup",
            "2026-07-16T08:00:00.0000000",
            "2026-07-16T08:15:00.0000000",
        );
        second["iCalUId"] = json!("uid-series");
        return reply(
            200,
            json!({
                "value": [first, second],
                "@odata.deltaLink": format!("{base}/me/calendars/cal-main/calendarView/delta?$deltatoken=main-1"),
            }),
        );
    }
    if target.starts_with("/v1.0/me/calendars/cal-team/calendarView/delta?startDateTime=") {
        return reply(
            200,
            json!({
                "value": [event("team-1", "Review", "2026-07-17T09:00:00.0000000", "2026-07-17T10:00:00.0000000")],
                "@odata.deltaLink": format!("{base}/me/calendars/cal-team/calendarView/delta?$deltatoken=team-1"),
            }),
        );
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
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Planning", "2026-07-14T14:00:00Z", "cal-main"),
            ("Standup", "2026-07-15T08:00:00Z", "cal-main"),
            ("Standup", "2026-07-16T08:00:00Z", "cal-main"),
            ("Review", "2026-07-17T09:00:00Z", "cal-team"),
            // A Berlin all-day event converted to UTC still means its date.
            ("Holiday", "2026-07-20", "cal-main"),
        ]
    );
    let holiday = mirrored.iter().find(|row| row.title == "Holiday").unwrap();
    assert_eq!(
        (holiday.all_day, holiday.ends_at.as_str()),
        (1, "2026-07-21")
    );
    let planning = &mirrored[0];
    assert_eq!(planning.remote_href.as_deref(), Some("evt-1"));
    assert_eq!(planning.remote_etag.as_deref(), Some("W/\"evt-1-1\""));
    assert_eq!(planning.uid, "uid-evt-1");
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

    let requests = graph.requests();
    let first_delta = requests
        .iter()
        .find(|request| {
            request
                .target
                .contains("cal-main/calendarView/delta?startDateTime")
        })
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
            moved["@odata.etag"] = json!("W/\"evt-1-2\"");
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
                    "@odata.deltaLink": format!("{base}/me/calendars/cal-main/calendarView/delta?$deltatoken=main-2"),
                }),
            );
        }
        if target.ends_with("$deltatoken=team-1") {
            return reply(
                200,
                json!({
                    "value": [],
                    "@odata.deltaLink": format!("{base}/me/calendars/cal-team/calendarView/delta?$deltatoken=team-2"),
                }),
            );
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
        .map(|row| (row.title, row.starts_at))
        .collect();
    assert_eq!(
        titles,
        [
            (
                "Planning (moved)".to_string(),
                "2026-07-14T16:00:00Z".to_string()
            ),
            ("Standup".to_string(), "2026-07-15T08:00:00Z".to_string()),
            ("Review".to_string(), "2026-07-17T09:00:00Z".to_string()),
            ("Lunch".to_string(), "2026-07-18T11:00:00Z".to_string()),
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
    // No pass started from scratch.
    assert!(graph
        .requests()
        .iter()
        .all(|request| !request.target.contains("startDateTime")));
}

#[tokio::test]
async fn an_expired_delta_link_starts_the_calendar_over() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(initial_route).await;
    sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();
    graph.take_requests();

    graph.reroute(|request, base| {
        let target = request.target.as_str();
        if target.starts_with("/v1.0/me/calendars?") {
            return reply(200, calendars());
        }
        if target.ends_with("$deltatoken=main-1") {
            return reply(
                410,
                json!({ "error": { "code": "SyncStateNotFound", "message": "expired" } }),
            );
        }
        if target.starts_with("/v1.0/me/calendars/cal-main/calendarView/delta?startDateTime=") {
            return reply(
                200,
                json!({
                    "value": [event("evt-1", "Planning", "2026-07-14T14:00:00.0000000", "2026-07-14T15:00:00.0000000")],
                    "@odata.deltaLink": format!("{base}/me/calendars/cal-main/calendarView/delta?$deltatoken=main-fresh"),
                }),
            );
        }
        if target.ends_with("$deltatoken=team-1") {
            return reply(
                400,
                json!({ "error": { "code": "ErrorInvalidSyncStateData", "message": "SyncStateInvalid; resyncRequired" } }),
            );
        }
        if target.starts_with("/v1.0/me/calendars/cal-team/calendarView/delta?startDateTime=") {
            return reply(
                200,
                json!({
                    "value": [],
                    "@odata.deltaLink": format!("{base}/me/calendars/cal-team/calendarView/delta?$deltatoken=team-fresh"),
                }),
            );
        }
        reply(404, json!({ "error": { "code": "unexpected", "message": target } }))
    });

    let count = sync_with(&pool, "account", &graph.graph(), now())
        .await
        .unwrap();

    // Everything the fresh enumeration no longer lists is gone.
    assert_eq!(count, 1);
    assert_eq!(rows(&pool).await[0].title, "Planning");
    assert!(cursor_of(&pool, "cal-main")
        .await
        .unwrap()
        .delta_link
        .ends_with("main-fresh"));
    assert!(cursor_of(&pool, "cal-team")
        .await
        .unwrap()
        .delta_link
        .ends_with("team-fresh"));
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
                json!({ "value": [{ "id": "cal-main", "name": "Calendar", "isDefaultCalendar": true }] }),
            );
        }
        if target.starts_with("/v1.0/me/calendars/cal-main/calendarView/delta?startDateTime=") {
            return reply(
                200,
                json!({
                    "value": [],
                    "@odata.deltaLink": format!("{base}/me/calendars/cal-main/calendarView/delta?$deltatoken=main-later"),
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
    assert!(cursor_of(&pool, "cal-team").await.is_none());
}

#[tokio::test]
async fn paging_links_off_the_graph_host_are_refused() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(|request, base| {
        if request.target.starts_with("/v1.0/me/calendars?") {
            return reply(
                200,
                json!({ "value": [{ "id": "cal-main", "name": "Calendar" }],
                        "@odata.nextLink": "https://attacker.example/steal" }),
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

#[tokio::test]
async fn new_events_go_to_the_default_calendar_and_invite_only_when_asked() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph =
        FakeGraph::start(
            |request, _| match (request.method.as_str(), request.target.as_str()) {
                ("GET", target) if target.starts_with("/v1.0/me/calendar?") => {
                    reply(200, json!({ "id": "cal-main" }))
                }
                ("POST", "/v1.0/me/calendars/cal-main/events") => {
                    let mut created = request.body.clone();
                    created["id"] = json!("evt-created");
                    created["@odata.etag"] = json!("W/\"created-1\"");
                    created["iCalUId"] = json!("uid-created");
                    reply(201, created)
                }
                _ => reply(404, json!({})),
            },
        )
        .await;
    let fields = EventFields {
        title: "Lunch",
        description: Some("Bring snacks"),
        location: Some("Canteen"),
        starts_at: now(),
        ends_at: now() + chrono::Duration::hours(1),
        all_day: false,
    };

    let created = create_event(&pool, "account", &fields, &[]).await.unwrap();

    assert_eq!(created.calendar_id, "cal-main");
    assert_eq!(created.event.remote_id, "evt-created");
    assert_eq!(created.event.id, row_id("account", "evt-created"));
    assert_eq!(created.event.starts_at, "2026-07-01T12:00:00Z");
    let post = graph
        .requests()
        .into_iter()
        .find(|request| request.method == "POST")
        .unwrap();
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
        ..fields
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
async fn updates_send_only_changed_fields_under_the_last_seen_etag() {
    let pool = database(CALENDAR_SCOPES).await;
    let _graph = FakeGraph::start(|request, _| {
        if request.method == "PATCH" && request.target == "/v1.0/me/events/evt-1" {
            if request.header("If-Match") != Some("W/\"evt-1-1\"") {
                return reply(
                    412,
                    json!({ "error": { "code": "ErrorIrresolvableConflict" } }),
                );
            }
            let mut updated = event(
                "evt-1",
                "Planning v2",
                "2026-07-14T14:00:00.0000000",
                "2026-07-14T15:00:00.0000000",
            );
            updated["@odata.etag"] = json!("W/\"evt-1-2\"");
            return reply(200, updated);
        }
        reply(404, json!({}))
    })
    .await;
    let previous = MirroredEventView {
        title: "Planning",
        description: Some("Agenda\nPoints"),
        location: Some("Room 1"),
        starts_at: "2026-07-14T14:00:00Z",
        ends_at: "2026-07-14T15:00:00Z",
        all_day: false,
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
    let patch = patch_body(&previous, &fields);
    assert_eq!(patch, json!({ "subject": "Planning v2" }));

    let updated = update_event(
        &pool,
        "account",
        "cal-main",
        "evt-1",
        Some("W/\"evt-1-1\""),
        &patch,
    )
    .await
    .unwrap();
    assert_eq!(updated.event.title, "Planning v2");
    assert_eq!(updated.event.etag.as_deref(), Some("W/\"evt-1-2\""));

    let stale = update_event(
        &pool,
        "account",
        "cal-main",
        "evt-1",
        Some("W/\"evt-1-0\""),
        &patch,
    )
    .await;
    assert!(matches!(stale, Err(WriteError::Changed)));

    let moved = EventFields {
        starts_at: start + chrono::Duration::hours(2),
        ends_at: start + chrono::Duration::hours(3),
        ..fields
    };
    let patch = patch_body(&previous, &moved);
    assert_eq!(
        patch["start"],
        json!({ "dateTime": "2026-07-14T16:00:00", "timeZone": "UTC" })
    );
    assert_eq!(patch["isAllDay"], json!(false));
    assert!(patch.get("body").is_none());
}

#[tokio::test]
async fn deletion_is_guarded_and_an_event_already_gone_counts_as_deleted() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph =
        FakeGraph::start(
            |request, _| match (request.method.as_str(), request.target.as_str()) {
                ("DELETE", "/v1.0/me/events/evt-1") => reply(204, Value::Null),
                ("DELETE", "/v1.0/me/events/evt-gone") => {
                    reply(404, json!({ "error": { "code": "ErrorItemNotFound" } }))
                }
                ("DELETE", "/v1.0/me/events/evt-changed") => reply(
                    412,
                    json!({ "error": { "code": "ErrorIrresolvableConflict" } }),
                ),
                _ => reply(500, json!({})),
            },
        )
        .await;

    delete_event(&pool, "account", "evt-1", Some("W/\"evt-1-1\""))
        .await
        .unwrap();
    delete_event(&pool, "account", "evt-gone", None)
        .await
        .unwrap();
    assert!(matches!(
        delete_event(&pool, "account", "evt-changed", Some("W/\"x\"")).await,
        Err(WriteError::Changed)
    ));
    let first = &graph.requests()[0];
    assert_eq!(first.header("If-Match"), Some("W/\"evt-1-1\""));
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
fn expired_delta_links_are_recognised_by_status_and_code() {
    assert!(delta_link_expired(410, ""));
    assert!(delta_link_expired(
        400,
        r#"{"error":{"code":"SyncStateNotFound"}}"#
    ));
    assert!(delta_link_expired(
        400,
        r#"{"error":{"message":"resyncRequired"}}"#
    ));
    assert!(!delta_link_expired(
        400,
        r#"{"error":{"code":"BadRequest"}}"#
    ));
    assert!(!delta_link_expired(403, "SyncStateNotFound"));
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

/// A Graph that keeps the events it is given, for the editor's round trip.
fn stateful_graph(store: Arc<Mutex<Vec<Value>>>) -> impl Fn(&Request, &str) -> Reply + Send + Sync {
    move |request, _| {
        let mut events = store.lock().unwrap();
        let target = request.target.as_str();
        match request.method.as_str() {
            "GET" if target.starts_with("/v1.0/me/calendar?") => {
                reply(200, json!({ "id": "cal-main" }))
            }
            "POST" if target == "/v1.0/me/calendars/cal-main/events" => {
                let mut created = request.body.clone();
                created["id"] = json!(format!("evt-{}", events.len() + 1));
                created["@odata.etag"] = json!("W/\"1\"");
                events.push(created.clone());
                reply(201, created)
            }
            "PATCH" => {
                let id = target.trim_start_matches("/v1.0/me/events/");
                let Some(event) = events.iter_mut().find(|event| event["id"] == id) else {
                    return reply(404, json!({}));
                };
                if request.header("If-Match") != event["@odata.etag"].as_str() {
                    return reply(
                        412,
                        json!({ "error": { "code": "ErrorIrresolvableConflict" } }),
                    );
                }
                for (key, value) in request.body.as_object().unwrap() {
                    event[key] = value.clone();
                }
                let version = event["@odata.etag"].as_str().unwrap().len();
                event["@odata.etag"] = json!(format!("W/\"{version}\""));
                reply(200, event.clone())
            }
            "DELETE" => {
                let id = target.trim_start_matches("/v1.0/me/events/");
                events.retain(|event| event["id"] != id);
                reply(204, Value::Null)
            }
            _ => reply(404, json!({})),
        }
    }
}

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
    assert_eq!(patch.header("If-Match"), Some("W/\"1\""));
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
    assert!(matches!(
        guests,
        Err(crate::calendar::SaveError::Validation(
            "backend.errors.calendarEditUnsupported"
        ))
    ));

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

    graph.reroute(|_, _| reply(403, json!({ "error": { "code": "ErrorAccessDenied" } })));
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

/// Graph v1.0 documents the delta of the default calendar's view only. Where
/// the per-calendar delta is refused, the default calendar uses that, and any
/// other calendar is listed whole on every pass.
fn without_per_calendar_delta(
    team_events: Value,
) -> impl Fn(&Request, &str) -> Reply + Send + Sync {
    move |request, base| {
        let target = request.target.as_str();
        if target.starts_with("/v1.0/me/calendars?") {
            return reply(200, calendars());
        }
        if target.contains("/calendarView/delta?startDateTime=")
            && target.starts_with("/v1.0/me/calendars/")
        {
            return reply(
                400,
                json!({ "error": { "code": "BadRequest", "message": "Unsupported segment" } }),
            );
        }
        if target.starts_with("/v1.0/me/calendarView/delta?startDateTime=") {
            return reply(
                200,
                json!({
                    "value": [event("evt-1", "Planning", "2026-07-14T14:00:00.0000000", "2026-07-14T15:00:00.0000000")],
                    "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=default-1"),
                }),
            );
        }
        if target.ends_with("$deltatoken=default-1") {
            return reply(
                200,
                json!({ "value": [], "@odata.deltaLink": format!("{base}/me/calendarView/delta?$deltatoken=default-2") }),
            );
        }
        if target.starts_with("/v1.0/me/calendars/cal-team/calendarView?startDateTime=") {
            return reply(200, json!({ "value": team_events.clone() }));
        }
        reply(
            404,
            json!({ "error": { "code": "unexpected", "message": target } }),
        )
    }
}

#[tokio::test]
async fn calendars_without_a_per_calendar_delta_fall_back_to_documented_forms() {
    let pool = database(CALENDAR_SCOPES).await;
    let graph = FakeGraph::start(without_per_calendar_delta(json!([
        event(
            "team-1",
            "Review",
            "2026-07-17T09:00:00.0000000",
            "2026-07-17T10:00:00.0000000"
        ),
        event(
            "team-2",
            "Retro",
            "2026-07-18T09:00:00.0000000",
            "2026-07-18T10:00:00.0000000"
        ),
    ])))
    .await;

    assert_eq!(
        sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap(),
        3
    );
    assert!(cursor_of(&pool, "cal-main")
        .await
        .unwrap()
        .delta_link
        .ends_with("default-1"));
    assert!(cursor_of(&pool, "cal-team").await.is_none());

    // The listed calendar is listed again; what it no longer holds is gone.
    graph.reroute(without_per_calendar_delta(json!([event(
        "team-1",
        "Review",
        "2026-07-17T09:00:00.0000000",
        "2026-07-17T10:00:00.0000000"
    )])));
    assert_eq!(
        sync_with(&pool, "account", &graph.graph(), now())
            .await
            .unwrap(),
        2
    );
    assert!(cursor_of(&pool, "cal-main")
        .await
        .unwrap()
        .delta_link
        .ends_with("default-2"));
    let titles: Vec<_> = rows(&pool).await.into_iter().map(|row| row.title).collect();
    assert_eq!(titles, ["Planning", "Review"]);
}
