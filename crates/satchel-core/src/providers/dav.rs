//! CalDAV/CardDAV transport, mapping, and local persistence.
//!
//! This provider is Tauri-free so the same DAV integration can be reused by
//! the desktop app, mobile clients, and the future headless hub.
use reqwest::{header, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db::contacts::ContactRepository;

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
        match sync_caldav(pool, &client, account_id, config).await {
            Ok((collections, events)) => {
                summary.calendars_found += collections;
                summary.events_synced += events;
            }
            Err(error) => {
                tracing::warn!(error = %error, account_id, "CalDAV synchronization failed");
                summary
                    .errors
                    .push(groupware_sync_error_key(DavKind::Calendar).to_string());
            }
        }
    }
    if let Some(config) = snapshot.carddav.as_ref() {
        match sync_carddav(pool, &client, config).await {
            Ok((collections, contacts)) => {
                summary.address_books_found += collections;
                summary.contacts_synced += contacts;
            }
            Err(error) => {
                tracing::warn!(error = %error, account_id, "CardDAV synchronization failed");
                summary
                    .errors
                    .push(groupware_sync_error_key(DavKind::AddressBook).to_string());
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
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:resourcetype/><d:displayname/></d:prop>
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
        collections.push(DavCollection { href });
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

async fn sync_caldav(
    pool: &SqlitePool,
    client: &reqwest::Client,
    account_id: &str,
    config: &DavConfig,
) -> Result<(usize, usize), String> {
    let collections = discover_collections(client, config, DavKind::Calendar).await?;
    let mut synced = 0;
    let start = (chrono::Utc::now() - chrono::Duration::days(365)).format("%Y%m%dT%H%M%SZ");
    let end = (chrono::Utc::now() + chrono::Duration::days(730)).format("%Y%m%dT%H%M%SZ");

    for collection in &collections {
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
    config: &DavConfig,
) -> Result<(usize, usize), String> {
    let collections = discover_collections(client, config, DavKind::AddressBook).await?;
    let mut synced = 0;
    for collection in &collections {
        let vcards = fetch_addressbook_vcards(client, config, collection).await?;
        for vcard in &vcards {
            let parsed = parse_vcard(vcard);
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
        let collection = collections
            .into_iter()
            .next()
            .ok_or_else(|| "no writable CalDAV calendar found".to_string())?;
        let filename = format!("{}.ics", sanitize_path_component(uid));
        let url = collection
            .href
            .join(&filename)
            .map_err(|err| err.to_string())?;
        (url, collection.href.to_string())
    };

    let mut request = client
        .request(Method::PUT, url.clone())
        .basic_auth(&config.username, Some(&config.password))
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .body(as_caldav_resource(raw_ics));
    request = if let Some(etag) = existing_etag.filter(|value| !value.trim().is_empty()) {
        request.header(header::IF_MATCH, etag)
    } else if existing_href.is_none() {
        request.header(header::IF_NONE_MATCH, "*")
    } else {
        request
    };
    let response = request
        .send()
        .await
        .map_err(|err| format!("CalDAV write failed: {err}"))?;
    if !response.status().is_success() {
        return Err(dav_status_error("CalDAV write", response).await);
    }
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    Ok(Some(RemoteCalendarWrite {
        href: url.to_string(),
        etag,
        collection_href,
    }))
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

async fn dav_status_error(context: &str, response: reqwest::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = body.chars().take(500).collect::<String>();
    if detail.trim().is_empty() {
        format!("{context} failed with status {status}")
    } else {
        format!("{context} failed with status {status}: {detail}")
    }
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
    fn strips_itip_method_from_caldav_resources() {
        let raw = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:one\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let resource = as_caldav_resource(raw);
        assert!(!resource.contains("METHOD:"));
        assert!(resource.contains("UID:one\r\n"));
    }
}
