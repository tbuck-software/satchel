//! CalDAV/CardDAV transport, mapping, and local persistence.
//!
//! This provider is Tauri-free so the same DAV integration can be reused by
//! the desktop app, mobile clients, and the future headless hub.
use reqwest::{header, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db::contacts::ContactRepository;
use crate::email::calendar::parse_ics_duration;

const MAX_DAV_RESPONSE_BYTES: usize = 50 * 1024 * 1024;
const CARDDAV_ADDRESSBOOK_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<a:addressbook-query xmlns:d="DAV:" xmlns:a="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:getetag/><a:address-data/></d:prop>
  <a:filter/>
</a:addressbook-query>"#;

const CARDDAV_SYNC_COLLECTION_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:sync-collection xmlns:d="DAV:" xmlns:a="urn:ietf:params:xml:ns:carddav">
  <d:sync-token/>
  <d:prop><d:getetag/><a:address-data/></d:prop>
</d:sync-collection>"#;

const CARDDAV_LISTING_PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:getetag/></d:prop>
</d:propfind>"#;

const MAX_MULTIGET_HREFS: usize = 150;
const GROUPWARE_CALENDAR_SYNC_ERROR_KEY: &str = "backend.errors.groupwareCalendarSyncFailed";
const GROUPWARE_CONTACT_SYNC_ERROR_KEY: &str = "backend.errors.groupwareContactSyncFailed";
const GROUPWARE_TASK_SYNC_ERROR_KEY: &str = "backend.errors.groupwareTaskSyncFailed";

pub mod tasks;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/providers/dav/"))]
pub struct DavConfig {
    pub server: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Default, Deserialize)]
struct GroupwareConfigSnapshot {
    caldav: Option<DavConfig>,
    carddav: Option<DavConfig>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/providers/dav/"))]
pub struct GroupwareSyncSummary {
    pub accounts_checked: usize,
    pub calendars_found: usize,
    pub events_synced: usize,
    pub address_books_found: usize,
    pub contacts_synced: usize,
    pub task_lists_found: usize,
    pub tasks_synced: usize,
    /// What the task sync did, for the frontend's notices.
    pub task_summary: crate::tasks::sync::TaskSyncSummary,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum DavKind {
    Calendar,
    AddressBook,
}

const fn groupware_sync_error_key(kind: DavKind) -> &'static str {
    match kind {
        DavKind::Calendar => GROUPWARE_CALENDAR_SYNC_ERROR_KEY,
        DavKind::AddressBook => GROUPWARE_CONTACT_SYNC_ERROR_KEY,
    }
}

/// The summary entry for one failed service, naming the account so a person
/// with two groupware accounts can tell which one stopped syncing. The
/// account id stands in when its address cannot be read.
fn groupware_sync_error(kind: DavKind, account: &str) -> String {
    serde_json::json!({
        "key": groupware_sync_error_key(kind),
        "params": { "account": account },
    })
    .to_string()
}

async fn account_label(pool: &SqlitePool, account_id: &str) -> String {
    crate::db::accounts::AccountRepository::new(pool)
        .identity(account_id)
        .await
        .map_or_else(|_| account_id.to_string(), |identity| identity.email)
}

pub async fn probe_dav_server(
    server: &str,
    username: &str,
    password: &str,
    calendar: bool,
) -> Result<(String, usize), String> {
    let config = optional_dav_config(Some(server), username, password)?
        .ok_or_else(|| "DAV server is required".to_string())?;
    let collections = probe_dav_config(&config, calendar).await?;
    Ok((config.server, collections))
}

/// Verify one already-normalized DAV config and return its collection count.
pub async fn probe_dav_config(config: &DavConfig, calendar: bool) -> Result<usize, String> {
    let kind = if calendar {
        DavKind::Calendar
    } else {
        DavKind::AddressBook
    };
    let collections = discover_collections(&dav_client()?, config, kind).await?;
    Ok(collections.len())
}

impl DavKind {
    fn home_property(self) -> &'static str {
        match self {
            Self::Calendar => "calendar-home-set",
            Self::AddressBook => "addressbook-home-set",
        }
    }

    fn collection_type(self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::AddressBook => "addressbook",
        }
    }
}

#[derive(Debug, Clone)]
struct DavCollection {
    href: Url,
    name: Option<String>,
    color: Option<String>,
    /// `supported-calendar-component-set`; None when the server does not say,
    /// which RFC 4791 §5.2.3 reads as "any component".
    components: Option<Vec<String>>,
    /// From `current-user-privilege-set`; None when the server does not say.
    writable: Option<bool>,
}

impl DavCollection {
    fn supports(&self, component: &str) -> bool {
        self.components
            .as_ref()
            .is_none_or(|components| components.iter().any(|value| value == component))
    }

    /// VTODO only when the server says so: a server that does not advertise
    /// components would otherwise turn every calendar into a task list.
    fn advertises(&self, component: &str) -> bool {
        self.components
            .as_ref()
            .is_some_and(|components| components.iter().any(|value| value == component))
    }
}

#[derive(Debug, Clone)]
pub struct RemoteCalendarWrite {
    pub href: String,
    pub etag: Option<String>,
    pub collection_href: String,
}

pub async fn sync_groupware_account(
    pool: &SqlitePool,
    account_id: &str,
    config_json: &crate::accounts::AccountConfig,
) -> Result<GroupwareSyncSummary, String> {
    let snapshot = parse_groupware_config(config_json);
    let client = dav_client()?;
    let mut summary = GroupwareSyncSummary {
        accounts_checked: 1,
        ..Default::default()
    };

    if let Some(config) = snapshot.caldav.as_ref() {
        // One discovery serves events and tasks.
        let discovered = discover_collections(&client, config, DavKind::Calendar).await;
        let events = match &discovered {
            Ok(collections) => {
                sync_caldav_events(pool, &client, account_id, config, collections).await
            }
            Err(error) => Err(error.clone()),
        };
        match events {
            Ok((collections, events)) => {
                summary.calendars_found += collections;
                summary.events_synced += events;
            }
            Err(error) => {
                tracing::warn!(error = %error, account_id, "CalDAV synchronization failed");
                let account = account_label(pool, account_id).await;
                summary
                    .errors
                    .push(groupware_sync_error(DavKind::Calendar, &account));
            }
        }
        // A failed discovery was reported once, as the calendar's.
        if let Ok(collections) = discovered {
            let remote = tasks::CalDavTaskRemote::with_collections(
                client.clone(),
                config.clone(),
                collections,
            );
            match crate::tasks::sync::sync_account(pool, account_id, &remote).await {
                Ok(tasks) => {
                    summary.task_lists_found += tasks.lists;
                    summary.tasks_synced += tasks.pulled + tasks.pushed;
                    summary.task_summary.merge(&tasks);
                }
                Err(error) => {
                    tracing::warn!(error = %error, account_id, "CalDAV task synchronization failed");
                    let account = account_label(pool, account_id).await;
                    summary.errors.push(
                        serde_json::json!({
                            "key": GROUPWARE_TASK_SYNC_ERROR_KEY,
                            "params": { "account": account },
                        })
                        .to_string(),
                    );
                }
            }
        }
    }
    if let Some(config) = snapshot.carddav.as_ref() {
        match sync_carddav(pool, &client, account_id, config).await {
            Ok((collections, contacts)) => {
                summary.address_books_found += collections;
                summary.contacts_synced += contacts;
            }
            Err(error) => {
                tracing::warn!(error = %error, account_id, "CardDAV synchronization failed");
                let account = account_label(pool, account_id).await;
                summary
                    .errors
                    .push(groupware_sync_error(DavKind::AddressBook, &account));
            }
        }
    }
    Ok(summary)
}

impl GroupwareSyncSummary {
    pub fn merge(&mut self, other: Self) {
        self.calendars_found += other.calendars_found;
        self.events_synced += other.events_synced;
        self.address_books_found += other.address_books_found;
        self.contacts_synced += other.contacts_synced;
        self.task_lists_found += other.task_lists_found;
        self.tasks_synced += other.tasks_synced;
        self.task_summary.merge(&other.task_summary);
        self.errors.extend(other.errors);
    }
}
pub fn optional_dav_config(
    server: Option<&str>,
    username: &str,
    password: &str,
) -> Result<Option<DavConfig>, String> {
    let Some(server) = server.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    Ok(Some(DavConfig {
        server: normalize_server_url(server)?,
        username: username.to_string(),
        password: password.to_string(),
    }))
}

fn normalize_server_url(server: &str) -> Result<String, String> {
    let candidate = if server.contains("://") {
        server.to_string()
    } else {
        format!("https://{server}")
    };
    let mut url = Url::parse(&candidate).map_err(|err| format!("invalid DAV server URL: {err}"))?;
    if url.scheme() != "https" {
        return Err("DAV server must use HTTPS".to_string());
    }
    if url.host_str().is_none() {
        return Err("DAV server URL is missing a host".to_string());
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url.to_string())
}

fn parse_groupware_config(config_json: &crate::accounts::AccountConfig) -> GroupwareConfigSnapshot {
    serde_json::from_str(config_json).unwrap_or_default()
}

/// Return whether the account config contains at least one DAV service.
///
/// Malformed JSON is treated as an unconfigured account.
pub fn has_configured_service(config_json: &crate::accounts::AccountConfig) -> bool {
    let snapshot = parse_groupware_config(config_json);
    snapshot.caldav.is_some() || snapshot.carddav.is_some()
}

fn dav_client() -> Result<reqwest::Client, String> {
    crate::utils::http::client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| format!("DAV client setup failed: {err}"))
}

async fn discover_collections(
    client: &reqwest::Client,
    config: &DavConfig,
    kind: DavKind,
) -> Result<Vec<DavCollection>, String> {
    let server = Url::parse(&config.server).map_err(|err| format!("invalid DAV URL: {err}"))?;
    let root_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:current-user-principal/><c:calendar-home-set/><a:addressbook-home-set/></d:prop>
</d:propfind>"#.to_string();
    let root_xml = dav_xml_request(
        client,
        config,
        Method::from_bytes(b"PROPFIND").unwrap(),
        &server,
        Some("0"),
        &root_body,
    )
    .await?;

    let direct_home = nested_href(&root_xml, kind.home_property());
    let principal = nested_href(&root_xml, "current-user-principal");
    let home = if let Some(home) = direct_home {
        resolve_dav_href(&server, &home)?
    } else if let Some(principal) = principal {
        let principal_url = resolve_dav_href(&server, &principal)?;
        let principal_xml = dav_xml_request(
            client,
            config,
            Method::from_bytes(b"PROPFIND").unwrap(),
            &principal_url,
            Some("0"),
            &root_body,
        )
        .await?;
        let href = nested_href(&principal_xml, kind.home_property())
            .ok_or_else(|| format!("DAV server did not advertise {}", kind.home_property()))?;
        resolve_dav_href(&server, &href)?
    } else {
        server.clone()
    };

    let list_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="urn:ietf:params:xml:ns:carddav" xmlns:x="http://apple.com/ns/ical/">
  <d:prop><d:resourcetype/><d:displayname/><c:supported-calendar-component-set/><x:calendar-color/><d:current-user-privilege-set/></d:prop>
</d:propfind>"#;
    let xml = dav_xml_request(
        client,
        config,
        Method::from_bytes(b"PROPFIND").unwrap(),
        &home,
        Some("1"),
        list_body,
    )
    .await?;

    let mut collections = Vec::new();
    for response in xml_elements(&xml, "response") {
        let Some(resource_type) = xml_elements(&response, "resourcetype").into_iter().next() else {
            continue;
        };
        if !xml_has_element(&resource_type, kind.collection_type()) {
            continue;
        }
        let Some(href) = xml_text(&response, "href") else {
            continue;
        };
        let href = resolve_dav_href(&server, &href)?;
        let components = xml_elements(&response, "supported-calendar-component-set")
            .into_iter()
            .next()
            .map(|set| {
                xml_attribute_values(&set, "comp", "name")
                    .into_iter()
                    .map(|name| name.to_ascii_uppercase())
                    .collect::<Vec<_>>()
            })
            .filter(|components| !components.is_empty());
        collections.push(DavCollection {
            href,
            name: xml_text(&response, "displayname").filter(|name| !name.trim().is_empty()),
            color: xml_text(&response, "calendar-color").and_then(|color| normalize_color(&color)),
            components,
            writable: privileges_writable(&response),
        });
    }

    if collections.is_empty() {
        return Err(format!(
            "no {} collections found at {}",
            kind.collection_type(),
            home
        ));
    }
    Ok(collections)
}

async fn sync_caldav_events(
    pool: &SqlitePool,
    client: &reqwest::Client,
    account_id: &str,
    config: &DavConfig,
    collections: &[DavCollection],
) -> Result<(usize, usize), String> {
    let collections = collections
        .iter()
        .filter(|collection| collection.supports("VEVENT"))
        .collect::<Vec<_>>();
    let mut synced = 0;
    let start = (chrono::Utc::now() - chrono::Duration::days(365)).format("%Y%m%dT%H%M%SZ");
    let end = (chrono::Utc::now() + chrono::Duration::days(730)).format("%Y%m%dT%H%M%SZ");

    for collection in collections.iter().copied() {
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{start}" end="{end}"/></c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#,
        );
        let xml = dav_xml_request(
            client,
            config,
            Method::from_bytes(b"REPORT").unwrap(),
            &collection.href,
            Some("1"),
            &body,
        )
        .await?;
        for response in xml_elements(&xml, "response") {
            let Some(href) = xml_text(&response, "href") else {
                continue;
            };
            let Some(raw_ics) = xml_text(&response, "calendar-data") else {
                continue;
            };
            let remote_url = resolve_dav_href(&collection.href, &href)?;
            let etag = xml_text(&response, "getetag");
            if crate::db::calendar::CalendarRepository::new(pool)
                .upsert_caldav_event(
                    account_id,
                    collection.href.as_str(),
                    remote_url.as_str(),
                    etag.as_deref(),
                    &raw_ics,
                )
                .await
                .map_err(|error| error.to_string())?
            {
                synced += 1;
            }
        }
    }
    Ok((collections.len(), synced))
}

async fn sync_carddav(
    pool: &SqlitePool,
    client: &reqwest::Client,
    account_id: &str,
    config: &DavConfig,
) -> Result<(usize, usize), String> {
    let collections = discover_collections(client, config, DavKind::AddressBook).await?;
    let mut synced = 0;
    let mut address_book = Vec::new();
    for collection in &collections {
        let vcards = fetch_addressbook_vcards(client, config, collection).await?;
        for vcard in &vcards {
            let parsed = parse_vcard(vcard);
            address_book.extend(parsed.emails.iter().cloned());
            for email in parsed.emails {
                if upsert_carddav_contact(
                    pool,
                    &email,
                    parsed.display_name.as_deref(),
                    parsed.note.as_deref(),
                )
                .await?
                {
                    synced += 1;
                }
            }
        }
    }
    // Every collection was read: the list replaces the previous one, so a
    // contact deleted on the server stops making its sender known.
    let book = crate::db::known_senders::AddressBook::Account {
        kind: "carddav",
        account_id,
    };
    crate::db::known_senders::replace_address_book(pool, book, address_book)
        .await
        .map_err(|error| {
            tracing::warn!(error = ?error, "CardDAV address book persistence failed");
            groupware_sync_error_key(DavKind::AddressBook).to_string()
        })?;
    Ok((collections.len(), synced))
}

/// Some servers reject `addressbook-query` REPORTs (RFC 6352 §8.5) with
/// `403 + <supported-report/>`. Fall back to `sync-collection` and then to
/// listing the collection plus `addressbook-multiget`, which nearly every
/// CardDAV server supports.
async fn fetch_addressbook_vcards(
    client: &reqwest::Client,
    config: &DavConfig,
    collection: &DavCollection,
) -> Result<Vec<String>, String> {
    let report = Method::from_bytes(b"REPORT").unwrap();
    if let Ok(xml) = dav_xml_request(
        client,
        config,
        report.clone(),
        &collection.href,
        Some("1"),
        CARDDAV_ADDRESSBOOK_QUERY,
    )
    .await
    {
        return Ok(extract_address_data(&xml));
    }
    if let Ok(xml) = dav_xml_request(
        client,
        config,
        report.clone(),
        &collection.href,
        Some("1"),
        CARDDAV_SYNC_COLLECTION_QUERY,
    )
    .await
    {
        return Ok(extract_address_data(&xml));
    }
    fetch_vcards_via_multiget(client, config, collection).await
}

async fn fetch_vcards_via_multiget(
    client: &reqwest::Client,
    config: &DavConfig,
    collection: &DavCollection,
) -> Result<Vec<String>, String> {
    let propfind = Method::from_bytes(b"PROPFIND").unwrap();
    let listing = dav_xml_request(
        client,
        config,
        propfind,
        &collection.href,
        Some("1"),
        CARDDAV_LISTING_PROPFIND,
    )
    .await?;

    let mut hrefs = Vec::new();
    for response in xml_elements(&listing, "response") {
        let Some(href) = xml_text(&response, "href") else {
            continue;
        };
        let url = resolve_dav_href(&collection.href, &href)?;
        if url.path() != collection.href.path() {
            hrefs.push(url);
        }
    }
    if hrefs.is_empty() {
        return Ok(Vec::new());
    }

    let report = Method::from_bytes(b"REPORT").unwrap();
    let mut vcards = Vec::new();
    for chunk in hrefs.chunks(MAX_MULTIGET_HREFS) {
        let xml = dav_xml_request(
            client,
            config,
            report.clone(),
            &collection.href,
            Some("1"),
            &addressbook_multiget_body(chunk),
        )
        .await?;
        vcards.extend(extract_address_data(&xml));
    }
    Ok(vcards)
}

fn addressbook_multiget_body(hrefs: &[Url]) -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:addressbook-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:getetag/><c:address-data/></d:prop>"#,
    );
    for href in hrefs {
        body.push_str("\n  <d:href>");
        body.push_str(&xml_escape(href.path()));
        body.push_str("</d:href>");
    }
    body.push_str("\n</c:addressbook-multiget>");
    body
}

fn extract_address_data(xml: &str) -> Vec<String> {
    xml_elements(xml, "response")
        .into_iter()
        .filter_map(|response| xml_text(&response, "address-data"))
        .collect()
}

async fn upsert_carddav_contact(
    pool: &SqlitePool,
    email: &str,
    display_name: Option<&str>,
    note: Option<&str>,
) -> Result<bool, String> {
    ContactRepository::new(pool)
        .upsert_carddav(email, display_name, note)
        .await
        .map_err(|error| {
            tracing::warn!(error = ?error, "CardDAV contact persistence failed");
            groupware_sync_error_key(DavKind::AddressBook).to_string()
        })
}

pub async fn put_calendar_event(
    config_json: &crate::accounts::AccountConfig,
    uid: &str,
    raw_ics: &str,
    existing_href: Option<&str>,
    existing_etag: Option<&str>,
) -> Result<Option<RemoteCalendarWrite>, String> {
    let Some(config) = parse_groupware_config(config_json).caldav else {
        return Ok(None);
    };
    let client = dav_client()?;
    let (url, collection_href) = if let Some(existing_href) = existing_href {
        let server = Url::parse(&config.server).map_err(|err| err.to_string())?;
        let url = resolve_dav_href(&server, existing_href)?;
        let collection = url.join("./").map_err(|err| err.to_string())?;
        (url, collection.to_string())
    } else {
        let collections = discover_collections(&client, &config, DavKind::Calendar).await?;
        // A VTODO-only collection (iCloud Reminders, a Nextcloud task list)
        // refuses events.
        let collection = collections
            .into_iter()
            .find(|collection| collection.supports("VEVENT"))
            .ok_or_else(|| "no writable CalDAV calendar found".to_string())?;
        let filename = format!("{}.ics", sanitize_path_component(uid));
        let url = collection
            .href
            .join(&filename)
            .map_err(|err| err.to_string())?;
        (url, collection.href.to_string())
    };

    let etag = dav_put(
        &client,
        &config,
        &url,
        raw_ics,
        existing_href.is_none(),
        existing_etag,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(Some(RemoteCalendarWrite {
        href: url.to_string(),
        etag,
        collection_href,
    }))
}

/// Why a DAV write was refused, as far as the caller can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DavWriteError {
    /// 412: the ETag (or `If-None-Match: *`) precondition failed.
    PreconditionFailed,
    /// 404/410: the resource is gone.
    NotFound,
    /// 403: no right to write here.
    Forbidden,
    Other(String),
}

impl std::fmt::Display for DavWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreconditionFailed => f.write_str("DAV precondition failed"),
            Self::NotFound => f.write_str("DAV resource not found"),
            Self::Forbidden => f.write_str("DAV write forbidden"),
            Self::Other(detail) => f.write_str(detail),
        }
    }
}

/// PUT one iCalendar resource. A creation sends `If-None-Match: *`, a
/// replacement `If-Match` with the ETag it was edited against.
async fn dav_put(
    client: &reqwest::Client,
    config: &DavConfig,
    url: &Url,
    raw_ics: &str,
    create: bool,
    etag: Option<&str>,
) -> Result<Option<String>, DavWriteError> {
    let server = Url::parse(&config.server).map_err(|err| DavWriteError::Other(err.to_string()))?;
    ensure_same_origin(&server, url).map_err(DavWriteError::Other)?;
    let mut request = client
        .request(Method::PUT, url.clone())
        .basic_auth(&config.username, Some(&config.password))
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .body(as_caldav_resource(raw_ics));
    request = if let Some(etag) = etag.filter(|value| !value.trim().is_empty()) {
        request.header(header::IF_MATCH, etag)
    } else if create {
        request.header(header::IF_NONE_MATCH, "*")
    } else {
        request
    };
    let response = request
        .send()
        .await
        .map_err(|err| DavWriteError::Other(format!("CalDAV write failed: {err}")))?;
    match response.status() {
        StatusCode::PRECONDITION_FAILED => return Err(DavWriteError::PreconditionFailed),
        StatusCode::FORBIDDEN => return Err(DavWriteError::Forbidden),
        StatusCode::NOT_FOUND | StatusCode::GONE if !create => return Err(DavWriteError::NotFound),
        status if !status.is_success() => {
            return Err(DavWriteError::Other(
                dav_status_error("CalDAV write", response).await,
            ))
        }
        _ => {}
    }
    Ok(response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string))
}

/// DELETE one resource with its ETag as precondition. Gone counts as done.
async fn dav_delete(
    client: &reqwest::Client,
    config: &DavConfig,
    url: &Url,
    etag: Option<&str>,
) -> Result<(), DavWriteError> {
    let server = Url::parse(&config.server).map_err(|err| DavWriteError::Other(err.to_string()))?;
    ensure_same_origin(&server, url).map_err(DavWriteError::Other)?;
    let mut request = client
        .request(Method::DELETE, url.clone())
        .basic_auth(&config.username, Some(&config.password));
    if let Some(etag) = etag.filter(|value| !value.trim().is_empty()) {
        request = request.header(header::IF_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .map_err(|err| DavWriteError::Other(format!("CalDAV delete failed: {err}")))?;
    match response.status() {
        StatusCode::PRECONDITION_FAILED => Err(DavWriteError::PreconditionFailed),
        StatusCode::FORBIDDEN => Err(DavWriteError::Forbidden),
        StatusCode::NOT_FOUND | StatusCode::GONE => Ok(()),
        status if status.is_success() => Ok(()),
        _ => Err(DavWriteError::Other(
            dav_status_error("CalDAV delete", response).await,
        )),
    }
}

pub async fn delete_calendar_event_remote(
    config_json: &crate::accounts::AccountConfig,
    remote_href: &str,
    etag: Option<&str>,
) -> Result<(), String> {
    let config = parse_groupware_config(config_json)
        .caldav
        .ok_or_else(|| "CalDAV is not configured for this account".to_string())?;
    let server = Url::parse(&config.server).map_err(|err| err.to_string())?;
    let url = resolve_dav_href(&server, remote_href)?;
    let client = dav_client()?;
    let mut request = client
        .request(Method::DELETE, url)
        .basic_auth(&config.username, Some(&config.password));
    if let Some(etag) = etag.filter(|value| !value.trim().is_empty()) {
        request = request.header(header::IF_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .map_err(|err| format!("CalDAV delete failed: {err}"))?;
    if response.status() == StatusCode::NOT_FOUND || response.status().is_success() {
        return Ok(());
    }
    Err(dav_status_error("CalDAV delete", response).await)
}

/// Whether the account config carries a CalDAV service.
pub fn has_caldav(config_json: &crate::accounts::AccountConfig) -> bool {
    parse_groupware_config(config_json).caldav.is_some()
}

/// Where a CalDAV server takes free/busy requests (RFC 6638), and as whom.
#[derive(Debug, Clone)]
struct ScheduleOutbox {
    url: Url,
    /// The first `mailto:` of `calendar-user-address-set`.
    organizer: Option<String>,
}

/// Outbox discovery costs three requests; its answer, or that the server has
/// none, is kept this long per server and user.
const OUTBOX_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

type OutboxCache = std::collections::HashMap<String, (std::time::Instant, Option<ScheduleOutbox>)>;

static OUTBOXES: std::sync::LazyLock<std::sync::Mutex<OutboxCache>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(OutboxCache::new()));

/// Busy time of `addresses` from the server's scheduling outbox. A server
/// that does not advertise `calendar-auto-schedule`, or has no outbox,
/// answers for nobody; recipients it reports a failure for are left out.
/// Either way they are unknown.
pub async fn free_busy(
    config_json: &crate::accounts::AccountConfig,
    account_email: &str,
    addresses: &[String],
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<crate::calendar::free_busy::ProviderAnswers, String> {
    use crate::calendar::free_busy::ProviderAnswers;
    let Some(config) = parse_groupware_config(config_json).caldav else {
        return Ok(ProviderAnswers::default());
    };
    let client = dav_client()?;
    let cache_key = format!("{}\u{0}{}", config.server, config.username);
    let cached = OUTBOXES.lock().ok().and_then(|cache| {
        cache
            .get(&cache_key)
            .filter(|(at, _)| at.elapsed() < OUTBOX_TTL)
            .map(|(_, outbox)| outbox.clone())
    });
    let outbox = match cached {
        Some(outbox) => outbox,
        None => {
            let outbox = discover_schedule_outbox(&client, &config).await?;
            if let Ok(mut cache) = OUTBOXES.lock() {
                cache.insert(cache_key, (std::time::Instant::now(), outbox.clone()));
            }
            outbox
        }
    };
    let Some(outbox) = outbox else {
        return Ok(ProviderAnswers::default());
    };
    let organizer = outbox.organizer.as_deref().unwrap_or(account_email);
    let body = free_busy_request(
        &uuid::Uuid::new_v4().to_string(),
        organizer,
        addresses,
        start,
        end,
        chrono::Utc::now(),
    );
    let response = client
        .post(outbox.url.clone())
        .basic_auth(&config.username, Some(&config.password))
        .header(
            header::CONTENT_TYPE,
            "text/calendar; charset=utf-8; method=REQUEST",
        )
        .body(body)
        .send()
        .await
        .map_err(|err| format!("CalDAV free/busy request failed: {err}"))?;
    let status = response.status();
    if status.is_client_error() && status.as_u16() != 429 {
        // The server will not schedule for this user; asking every minute won't change it.
        tracing::warn!(status = status.as_u16(), "CalDAV free/busy refused");
        return Ok(ProviderAnswers {
            refused: addresses.to_vec(),
            ..ProviderAnswers::default()
        });
    }
    if !status.is_success() {
        return Err(dav_status_error("CalDAV free/busy", response).await);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("CalDAV free/busy read failed: {err}"))?;
    if bytes.len() > MAX_DAV_RESPONSE_BYTES {
        return Err("DAV response exceeds the 50 MB safety limit".to_string());
    }
    let xml = String::from_utf8(bytes.to_vec())
        .map_err(|_| "DAV response is not valid UTF-8".to_string())?;
    Ok(ProviderAnswers {
        known: parse_schedule_response(&xml, addresses),
        ..ProviderAnswers::default()
    })
}

async fn discover_schedule_outbox(
    client: &reqwest::Client,
    config: &DavConfig,
) -> Result<Option<ScheduleOutbox>, String> {
    let server = Url::parse(&config.server).map_err(|err| format!("invalid DAV URL: {err}"))?;
    let propfind = Method::from_bytes(b"PROPFIND").expect("valid method");
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:current-user-principal/><c:schedule-outbox-URL/><c:calendar-user-address-set/><c:calendar-home-set/></d:prop>
</d:propfind>"#;
    let root = dav_xml_request(client, config, propfind.clone(), &server, Some("0"), body).await?;
    let principal = match nested_href(&root, "current-user-principal") {
        Some(href) => resolve_dav_href(&server, &href)?,
        None => server.clone(),
    };
    let xml = if principal == server {
        root
    } else {
        dav_xml_request(client, config, propfind, &principal, Some("0"), body).await?
    };
    let Some(outbox) = nested_href(&xml, "schedule-outbox-URL") else {
        return Ok(None);
    };
    let outbox = resolve_dav_href(&server, &outbox)?;
    let home = nested_href(&xml, "calendar-home-set")
        .map(|href| resolve_dav_href(&server, &href))
        .transpose()?
        .unwrap_or(principal);
    // RFC 6638 §2: a server that schedules says so in the DAV header.
    let options = client
        .request(Method::OPTIONS, home)
        .basic_auth(&config.username, Some(&config.password))
        .send()
        .await
        .map_err(|err| format!("DAV OPTIONS failed: {err}"))?;
    let schedules = options
        .headers()
        .get_all("DAV")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("calendar-auto-schedule"));
    if !schedules {
        return Ok(None);
    }
    let organizer = xml_elements(&xml, "calendar-user-address-set")
        .into_iter()
        .flat_map(|set| xml_elements(&set, "href"))
        .map(|href| xml_unescape(href.trim()))
        .find_map(|href| mailto(&href));
    Ok(Some(ScheduleOutbox {
        url: outbox,
        organizer,
    }))
}

fn mailto(uri: &str) -> Option<String> {
    let uri = uri.trim();
    let address = uri
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
        .map(|_| &uri[7..])?;
    Some(address.trim().to_ascii_lowercase()).filter(|address| address.contains('@'))
}

/// The VFREEBUSY request of RFC 6638 §3.2.1 for one organizer and many attendees.
fn free_busy_request(
    uid: &str,
    organizer: &str,
    addresses: &[String],
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let stamp = |time: chrono::DateTime<chrono::Utc>| time.format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Satchel//EN".to_string(),
        "METHOD:REQUEST".to_string(),
        "BEGIN:VFREEBUSY".to_string(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", stamp(now)),
        format!("DTSTART:{}", stamp(start)),
        format!("DTEND:{}", stamp(end)),
        format!("ORGANIZER:mailto:{organizer}"),
    ];
    // Callers pass parsed addresses; a line break here would still smuggle a
    // property into the request, so anything with one is left out.
    lines.extend(
        addresses
            .iter()
            .filter(|address| {
                !address
                    .chars()
                    .any(|ch| ch.is_control() || ch.is_whitespace())
            })
            .map(|address| format!("ATTENDEE:mailto:{address}")),
    );
    lines.push("END:VFREEBUSY".to_string());
    lines.push("END:VCALENDAR".to_string());
    format!("{}\r\n", lines.join("\r\n"))
}

/// Recipients of a `schedule-response` the server answered for (request
/// status 2.x), with their busy periods.
fn parse_schedule_response(
    xml: &str,
    asked: &[String],
) -> std::collections::HashMap<String, Vec<crate::calendar::availability::Busy>> {
    let mut known = std::collections::HashMap::new();
    for response in xml_elements(xml, "response") {
        let Some(recipient) = xml_elements(&response, "recipient").into_iter().next() else {
            continue;
        };
        // RFC 6638 wraps the recipient in DAV:href; early drafts did not.
        let recipient =
            xml_text(&recipient, "href").unwrap_or_else(|| xml_unescape(recipient.trim()));
        let Some(address) = mailto(&recipient).and_then(|address| {
            asked
                .iter()
                .find(|asked| asked.eq_ignore_ascii_case(&address))
        }) else {
            continue;
        };
        let status = xml_text(&response, "request-status").unwrap_or_default();
        if !status.trim_start().starts_with('2') {
            continue;
        }
        let Some(data) = xml_text(&response, "calendar-data") else {
            continue;
        };
        let data = ["&#13;", "&#xD;", "&#xd;"]
            .iter()
            .fold(data, |data, entity| data.replace(entity, ""));
        known.insert(address.clone(), parse_free_busy_periods(&data));
    }
    known
}

/// The busy periods of a VFREEBUSY reply. FBTYPE defaults to BUSY;
/// BUSY-TENTATIVE is tentative, FREE is no busy time at all.
fn parse_free_busy_periods(ics: &str) -> Vec<crate::calendar::availability::Busy> {
    let parse_time = |value: &str| {
        chrono::NaiveDateTime::parse_from_str(value.trim().trim_end_matches('Z'), "%Y%m%dT%H%M%S")
            .ok()
            .map(|time| time.and_utc())
    };
    let mut busy = Vec::new();
    for line in unfold_lines(ics) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let mut parts = name.split(';');
        if !parts
            .next()
            .is_some_and(|property| property.trim().eq_ignore_ascii_case("FREEBUSY"))
        {
            continue;
        }
        let kind = parts
            .find_map(|param| {
                param
                    .split_once('=')
                    .filter(|(key, _)| key.eq_ignore_ascii_case("FBTYPE"))
                    .map(|(_, value)| value.trim().to_ascii_uppercase())
            })
            .unwrap_or_else(|| "BUSY".to_string());
        if kind == "FREE" {
            continue;
        }
        for period in value.split(',') {
            let Some((from, until)) = period.split_once('/') else {
                continue;
            };
            let Some(start) = parse_time(from) else {
                continue;
            };
            let end =
                parse_time(until).or_else(|| parse_ics_duration(until).map(|span| start + span));
            if let Some(end) = end.filter(|end| *end > start) {
                busy.push(crate::calendar::availability::Busy {
                    start,
                    end,
                    tentative: kind == "BUSY-TENTATIVE",
                });
            }
        }
    }
    busy.sort_by_key(|block| (block.start, block.end));
    busy
}

async fn dav_xml_request(
    client: &reqwest::Client,
    config: &DavConfig,
    method: Method,
    url: &Url,
    depth: Option<&str>,
    body: &str,
) -> Result<String, String> {
    let server = Url::parse(&config.server).map_err(|err| err.to_string())?;
    ensure_same_origin(&server, url)?;
    let mut request = client
        .request(method, url.clone())
        .basic_auth(&config.username, Some(&config.password))
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .body(body.to_string());
    if let Some(depth) = depth {
        request = request.header("Depth", depth);
    }
    let response = request.send().await.map_err(|err| {
        format!(
            "DAV request to {} failed: {err}",
            url.host_str().unwrap_or("server")
        )
    })?;
    if !response.status().is_success() {
        return Err(dav_status_error("DAV request", response).await);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("DAV response read failed: {err}"))?;
    if bytes.len() > MAX_DAV_RESPONSE_BYTES {
        return Err("DAV response exceeds the 50 MB safety limit".to_string());
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| "DAV response is not valid UTF-8".to_string())
}

/// The status and the size of the answer, never its text: these errors are
/// logged, and a server's error page can echo attendees, paths or tokens.
async fn dav_status_error(context: &str, response: reqwest::Response) -> String {
    let status = response.status();
    let length = response.bytes().await.map_or(0, |body| body.len());
    format!("{context} failed with status {status} ({length} bytes of response)")
}

fn resolve_dav_href(base: &Url, href: &str) -> Result<Url, String> {
    let resolved = Url::parse(href)
        .or_else(|_| base.join(href))
        .map_err(|err| format!("invalid DAV href: {err}"))?;
    ensure_same_origin(base, &resolved)?;
    Ok(resolved)
}

fn ensure_same_origin(base: &Url, target: &Url) -> Result<(), String> {
    if base.scheme() != target.scheme()
        || base.host_str() != target.host_str()
        || base.port_or_known_default() != target.port_or_known_default()
    {
        return Err("DAV server returned a cross-origin URL".to_string());
    }
    Ok(())
}

fn nested_href(xml: &str, property: &str) -> Option<String> {
    xml_elements(xml, property)
        .into_iter()
        .find_map(|inner| xml_text(&inner, "href"))
}

fn xml_text(xml: &str, local_name: &str) -> Option<String> {
    xml_elements(xml, local_name)
        .into_iter()
        .next()
        .map(|value| xml_unescape(value.trim()))
}

fn xml_has_element(xml: &str, local_name: &str) -> bool {
    !xml_elements(xml, local_name).is_empty()
}

/// The values of `attribute` on every `local_name` tag, e.g. the `name` of
/// each `<c:comp name="VTODO"/>`.
fn xml_attribute_values(xml: &str, local_name: &str, attribute: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(open_rel) = xml[offset..].find('<') {
        let open = offset + open_rel;
        let Some(end_rel) = xml[open..].find('>') else {
            break;
        };
        let end = open + end_rel;
        offset = end + 1;
        let raw = xml[open + 1..end].trim().trim_end_matches('/');
        let mut parts = raw.splitn(2, char::is_whitespace);
        let qualified = parts.next().unwrap_or("");
        if qualified.rsplit(':').next() != Some(local_name) {
            continue;
        }
        let rest = parts.next().unwrap_or("");
        let mut cursor = rest;
        while let Some(eq) = cursor.find('=') {
            let name = cursor[..eq].trim();
            let after = cursor[eq + 1..].trim_start();
            let Some(quote) = after.chars().next().filter(|ch| *ch == '"' || *ch == '\'') else {
                break;
            };
            let Some(close) = after[1..].find(quote) else {
                break;
            };
            if name.rsplit(':').next() == Some(attribute) {
                out.push(xml_unescape(&after[1..1 + close]));
            }
            cursor = &after[close + 2..];
        }
    }
    out
}

/// Whether `current-user-privilege-set` (RFC 3744 §5.4) lets us change
/// resources: `write`, `write-content` or `all`. None when it is not there.
fn privileges_writable(response: &str) -> Option<bool> {
    let set = xml_elements(response, "current-user-privilege-set")
        .into_iter()
        .find(|set| xml_has_element(set, "privilege"))?;
    Some(
        ["write", "write-content", "all"]
            .iter()
            .any(|privilege| xml_has_element(&set, privilege)),
    )
}

/// `#RRGGBB` from Apple's `calendar-color`, which may carry an alpha byte.
fn normalize_color(value: &str) -> Option<String> {
    let hex = value.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex[..6].to_ascii_lowercase()))
}

fn xml_elements(xml: &str, local_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(open_rel) = xml[offset..].find('<') {
        let open = offset + open_rel;
        let Some(end_rel) = xml[open..].find('>') else {
            break;
        };
        let end = open + end_rel;
        let raw = xml[open + 1..end].trim();
        if raw.starts_with('/') || raw.starts_with('?') || raw.starts_with('!') {
            offset = end + 1;
            continue;
        }
        let self_closing = raw.ends_with('/');
        let qualified = raw
            .trim_end_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("");
        let local = qualified.rsplit(':').next().unwrap_or(qualified);
        if local != local_name {
            offset = end + 1;
            continue;
        }
        if self_closing {
            out.push(String::new());
            offset = end + 1;
            continue;
        }
        let close = format!("</{qualified}>");
        let content_start = end + 1;
        let Some(close_rel) = xml[content_start..].find(&close) else {
            offset = end + 1;
            continue;
        };
        let content_end = content_start + close_rel;
        out.push(xml[content_start..content_end].to_string());
        offset = content_end + close.len();
    }
    out
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_unescape(value: &str) -> String {
    let value = value
        .strip_prefix("<![CDATA[")
        .and_then(|value| value.strip_suffix("]]>"))
        .unwrap_or(value);
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn sanitize_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn unfold_lines(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in raw.replace("\r\n", "\n").lines() {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else {
            out.push(line.trim_end_matches('\r').to_string());
        }
    }
    out
}

fn as_caldav_resource(raw: &str) -> String {
    let mut output = unfold_lines(raw)
        .into_iter()
        .filter(|line| !line.to_ascii_uppercase().starts_with("METHOD:"))
        .collect::<Vec<_>>()
        .join("\r\n");
    output.push_str("\r\n");
    output
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedVcard {
    display_name: Option<String>,
    emails: Vec<String>,
    note: Option<String>,
}

fn parse_vcard(raw: &str) -> ParsedVcard {
    let mut parsed = ParsedVcard::default();
    for line in unfold_lines(raw) {
        let Some((property, value)) = line.split_once(':') else {
            continue;
        };
        let name = property
            .split(';')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        match name.as_str() {
            "FN" if parsed.display_name.is_none() => {
                parsed.display_name = clean_vcard_value(value);
            }
            "EMAIL" => {
                if let Some(email) = clean_vcard_value(value) {
                    if email.contains('@') {
                        parsed.emails.push(email.to_ascii_lowercase());
                    }
                }
            }
            "NOTE" if parsed.note.is_none() => parsed.note = clean_vcard_value(value),
            _ => {}
        }
    }
    parsed.emails.sort();
    parsed.emails.dedup();
    parsed
}

fn clean_vcard_value(value: &str) -> Option<String> {
    let value = value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
        .trim()
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_busy_request_names_the_organizer_and_every_attendee() {
        let at = |text: &str| chrono::DateTime::parse_from_rfc3339(text).unwrap().to_utc();
        let ics = free_busy_request(
            "uid-1",
            "me@example.test",
            &["a@example.test".into(), "b@example.test".into()],
            at("2026-11-09T00:00:00Z"),
            at("2026-11-10T00:00:00Z"),
            at("2026-11-06T12:00:00Z"),
        );
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.contains("\r\nMETHOD:REQUEST\r\nBEGIN:VFREEBUSY\r\n"));
        assert!(ics.contains("\r\nDTSTART:20261109T000000Z\r\nDTEND:20261110T000000Z\r\n"));
        assert!(ics.contains("\r\nORGANIZER:mailto:me@example.test\r\n"));
        assert!(ics
            .contains("\r\nATTENDEE:mailto:a@example.test\r\nATTENDEE:mailto:b@example.test\r\n"));
        assert!(ics.ends_with("END:VFREEBUSY\r\nEND:VCALENDAR\r\n"));
    }

    #[test]
    fn schedule_response_keeps_answered_recipients_with_their_periods() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<C:schedule-response xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <C:response>
    <C:recipient><D:href>mailto:Lena@Example.test</D:href></C:recipient>
    <C:request-status>2.0;Success</C:request-status>
    <C:calendar-data>BEGIN:VCALENDAR&#13;
VERSION:2.0&#13;
METHOD:REPLY&#13;
BEGIN:VFREEBUSY&#13;
FREEBUSY;FBTYPE=BUSY:20261109T090000Z/20261109T100000Z,20261109T140000Z/PT30M&#13;
FREEBUSY;FBTYPE=BUSY-TENTATIVE:20261109T110000Z/20261109T113000Z&#13;
FREEBUSY;FBTYPE=FREE:20261109T120000Z/20261109T130000Z&#13;
FREEBUSY:20261109T150000Z/PT1H&#13;
END:VFREEBUSY&#13;
END:VCALENDAR&#13;
</C:calendar-data>
  </C:response>
  <C:response>
    <C:recipient>mailto:nobody@example.test</C:recipient>
    <C:request-status>3.7;Invalid calendar user</C:request-status>
  </C:response>
</C:schedule-response>"#;
        let asked = [
            "lena@example.test".to_string(),
            "nobody@example.test".to_string(),
        ];
        let known = parse_schedule_response(xml, &asked);
        assert_eq!(known.len(), 1);
        let lena = &known["lena@example.test"];
        let spans = lena
            .iter()
            .map(|block| {
                (
                    block.start.format("%H:%M").to_string(),
                    block.end.format("%H:%M").to_string(),
                    block.tentative,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            spans,
            vec![
                ("09:00".into(), "10:00".into(), false),
                ("11:00".into(), "11:30".into(), true),
                ("14:00".into(), "14:30".into(), false),
                ("15:00".into(), "16:00".into(), false),
            ]
        );
    }

    #[test]
    fn ics_durations_cover_weeks_days_and_times() {
        assert_eq!(
            parse_ics_duration("PT1H30M"),
            Some(chrono::Duration::minutes(90))
        );
        assert_eq!(parse_ics_duration("P1D"), Some(chrono::Duration::days(1)));
        assert_eq!(parse_ics_duration("P1W"), Some(chrono::Duration::weeks(1)));
        assert_eq!(
            parse_ics_duration("P1DT2H"),
            Some(chrono::Duration::hours(26))
        );
        assert_eq!(parse_ics_duration("1H"), None);
        assert_eq!(parse_ics_duration("PT5"), None);
    }

    #[test]
    fn mailto_accepts_any_case_and_needs_an_address() {
        assert_eq!(
            mailto("MAILTO:Lena@Example.test"),
            Some("lena@example.test".into())
        );
        assert_eq!(mailto("mailto:"), None);
        assert_eq!(mailto("/principals/lena/"), None);
    }

    #[test]
    fn normalizes_https_server_names() {
        assert_eq!(
            normalize_server_url("gwcaldav.example.test").unwrap(),
            "https://gwcaldav.example.test/"
        );
        assert!(normalize_server_url("http://gwcaldav.example.test").is_err());
    }

    #[test]
    fn parses_namespaced_dav_multistatus() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
          <d:response><d:href>/cal/tbuck/work/</d:href><d:propstat><d:prop>
            <d:displayname>Work &amp; Team</d:displayname><d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
          </d:prop></d:propstat></d:response>
        </d:multistatus>"#;
        let responses = xml_elements(xml, "response");
        assert_eq!(responses.len(), 1);
        assert_eq!(
            xml_text(&responses[0], "href").as_deref(),
            Some("/cal/tbuck/work/")
        );
        assert_eq!(
            xml_text(&responses[0], "displayname").as_deref(),
            Some("Work & Team")
        );
        let resource_type = xml_elements(&responses[0], "resourcetype").remove(0);
        assert!(xml_has_element(&resource_type, "calendar"));
    }

    #[test]
    fn parses_folded_vcard_with_multiple_emails() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Thomas Buck\r\nEMAIL;TYPE=work:TBuck@example.test\r\nEMAIL;TYPE=home:other@example.test\r\nNOTE:One\\nTwo\r\nEND:VCARD\r\n";
        assert_eq!(
            parse_vcard(card),
            ParsedVcard {
                display_name: Some("Thomas Buck".to_string()),
                emails: vec![
                    "other@example.test".to_string(),
                    "tbuck@example.test".to_string()
                ],
                note: Some("One\nTwo".to_string()),
            }
        );
    }

    #[test]
    fn carddav_query_includes_required_filter() {
        assert!(xml_has_element(CARDDAV_ADDRESSBOOK_QUERY, "filter"));
    }

    #[test]
    fn groupware_summary_errors_use_stable_message_keys() {
        assert_eq!(
            groupware_sync_error_key(DavKind::Calendar),
            "backend.errors.groupwareCalendarSyncFailed"
        );
        assert_eq!(
            groupware_sync_error_key(DavKind::AddressBook),
            "backend.errors.groupwareContactSyncFailed"
        );
    }

    #[test]
    fn groupware_summary_errors_name_the_account() {
        let encoded = groupware_sync_error(DavKind::Calendar, "me@th-koeln.example");
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["key"], "backend.errors.groupwareCalendarSyncFailed");
        assert_eq!(value["params"]["account"], "me@th-koeln.example");

        let encoded = groupware_sync_error(DavKind::AddressBook, "me@th-koeln.example");
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["key"], "backend.errors.groupwareContactSyncFailed");
    }

    #[test]
    fn multiget_body_escapes_and_lists_hrefs() {
        let hrefs = vec![
            Url::parse("https://dav.example.test/book/a%20b.vcf").unwrap(),
            Url::parse("https://dav.example.test/book/c&d.vcf").unwrap(),
        ];
        let body = addressbook_multiget_body(&hrefs);
        assert!(xml_has_element(&body, "addressbook-multiget"));
        assert_eq!(xml_elements(&body, "href").len(), 2);
        assert!(body.contains("<d:href>/book/a%20b.vcf</d:href>"));
        assert!(body.contains("&amp;"));
        assert!(!body.contains("c&d"));
    }

    #[test]
    fn reads_component_sets_and_colors() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
          <d:response><d:href>/cal/todo/</d:href><d:propstat><d:prop>
            <c:supported-calendar-component-set><c:comp name="VTODO"/><c:comp name='vevent' /></c:supported-calendar-component-set>
          </d:prop></d:propstat><d:propstat><d:prop><c:supported-calendar-component-set/></d:prop>
          <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let set = xml_elements(xml, "supported-calendar-component-set").remove(0);
        assert_eq!(
            xml_attribute_values(&set, "comp", "name"),
            ["VTODO", "vevent"]
        );
        assert_eq!(normalize_color("#FF8800CC").as_deref(), Some("#ff8800"));
        assert_eq!(normalize_color("blue"), None);
    }

    #[test]
    fn reads_whether_the_collection_takes_writes() {
        let read_only = r#"<d:response><d:propstat><d:prop><d:current-user-privilege-set>
            <d:privilege><d:read/></d:privilege><d:privilege><d:read-current-user-privilege-set/></d:privilege>
          </d:current-user-privilege-set></d:prop></d:propstat></d:response>"#;
        let writable = r#"<d:response><d:propstat><d:prop><d:current-user-privilege-set>
            <d:privilege><d:read/></d:privilege><d:privilege><d:write-content/></d:privilege>
          </d:current-user-privilege-set></d:prop></d:propstat></d:response>"#;
        let unknown = r#"<d:response><d:propstat><d:prop><d:current-user-privilege-set/></d:prop>
          <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>"#;
        assert_eq!(privileges_writable(read_only), Some(false));
        assert_eq!(privileges_writable(writable), Some(true));
        assert_eq!(privileges_writable(unknown), None);
    }

    #[test]
    fn strips_itip_method_from_caldav_resources() {
        let raw = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:one\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let resource = as_caldav_resource(raw);
        assert!(!resource.contains("METHOD:"));
        assert!(resource.contains("UID:one\r\n"));
    }
}
