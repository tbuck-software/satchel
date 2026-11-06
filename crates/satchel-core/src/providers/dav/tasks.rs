//! CalDAV VTODO collections as a [`TaskRemote`].
//!
//! The same client, discovery and XML handling as calendar events: a task
//! list is a calendar collection whose `supported-calendar-component-set`
//! names VTODO. Changes are found by ETag (a `calendar-query` for VTODO that
//! returns only `getetag`), fetched with `calendar-multiget`, and written with
//! `If-Match` / `If-None-Match: *`.
use async_trait::async_trait;
use reqwest::{Method, Url};

use super::{
    dav_client, dav_delete, dav_put, dav_xml_request, discover_collections, parse_groupware_config,
    resolve_dav_href, sanitize_path_component, xml_elements, xml_escape, xml_text, DavCollection,
    DavConfig, DavKind, DavWriteError, MAX_MULTIGET_HREFS,
};
use crate::tasks::sync::{RemoteError, RemoteItem, RemoteList, TaskRemote, Written};
use crate::tasks::vtodo::{self, TaskFields};

const VTODO_ETAG_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO"/></c:comp-filter></c:filter>
</c:calendar-query>"#;

const VTODO_DATA_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO"/></c:comp-filter></c:filter>
</c:calendar-query>"#;

const ETAG_PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:getetag/></d:prop></d:propfind>"#;

pub struct CalDavTaskRemote {
    client: reqwest::Client,
    config: DavConfig,
    /// Collections a groupware sync already discovered.
    known: Option<Vec<DavCollection>>,
}

impl CalDavTaskRemote {
    /// The account's CalDAV service, or None when it has none.
    pub fn from_account_config(
        config_json: &crate::accounts::AccountConfig,
    ) -> Result<Option<Self>, String> {
        let Some(config) = parse_groupware_config(config_json).caldav else {
            return Ok(None);
        };
        Ok(Some(Self {
            client: dav_client()?,
            config,
            known: None,
        }))
    }

    pub(super) fn with_collections(
        client: reqwest::Client,
        config: DavConfig,
        collections: Vec<DavCollection>,
    ) -> Self {
        Self {
            client,
            config,
            known: Some(collections),
        }
    }

    fn url(&self, href: &str) -> Result<Url, RemoteError> {
        let server = Url::parse(&self.config.server).map_err(|err| other(err.to_string()))?;
        resolve_dav_href(&server, href).map_err(other)
    }

    async fn report(&self, collection: &Url, body: &str) -> Result<String, RemoteError> {
        dav_xml_request(
            &self.client,
            &self.config,
            Method::from_bytes(b"REPORT").expect("valid method"),
            collection,
            Some("1"),
            body,
        )
        .await
        .map_err(other)
    }

    /// The current ETag of one resource (PROPFIND, depth 0).
    async fn etag_of(&self, url: &Url) -> Option<String> {
        let xml = dav_xml_request(
            &self.client,
            &self.config,
            Method::from_bytes(b"PROPFIND").expect("valid method"),
            url,
            Some("0"),
            ETAG_PROPFIND,
        )
        .await
        .ok()?;
        xml_text(&xml, "getetag").filter(|etag| !etag.is_empty())
    }

    fn items(&self, collection: &Url, xml: &str) -> Result<Vec<RemoteItem>, RemoteError> {
        let mut items = Vec::new();
        for response in xml_elements(xml, "response") {
            let (Some(href), Some(payload)) = (
                xml_text(&response, "href"),
                xml_text(&response, "calendar-data"),
            ) else {
                continue;
            };
            let url = resolve_dav_href(collection, &href).map_err(other)?;
            items.push(RemoteItem {
                remote_id: url.to_string(),
                version: xml_text(&response, "getetag").filter(|etag| !etag.is_empty()),
                payload,
            });
        }
        Ok(items)
    }
}

fn other(detail: impl Into<String>) -> RemoteError {
    RemoteError::Other(detail.into())
}

fn remote_error(error: DavWriteError) -> RemoteError {
    match error {
        DavWriteError::PreconditionFailed => RemoteError::Conflict,
        DavWriteError::NotFound => RemoteError::Gone,
        DavWriteError::Forbidden => RemoteError::Forbidden,
        DavWriteError::Other(detail) => RemoteError::Other(detail),
    }
}

fn calendar_multiget_body(hrefs: &[Url]) -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>"#,
    );
    for href in hrefs {
        body.push_str("\n  <d:href>");
        body.push_str(&xml_escape(href.path()));
        body.push_str("</d:href>");
    }
    body.push_str("\n</c:calendar-multiget>");
    body
}

/// The VTODO collections among the discovered calendars.
fn task_lists(collections: &[DavCollection]) -> Vec<RemoteList> {
    collections
        .iter()
        .filter(|collection| collection.advertises("VTODO"))
        .map(|collection| RemoteList {
            remote_id: collection.href.to_string(),
            name: collection.name.clone().unwrap_or_default(),
            color: collection.color.clone(),
            read_only: collection.writable.map(|writable| !writable),
            mixed: collection.supports("VEVENT"),
        })
        .collect()
}

#[async_trait]
impl TaskRemote for CalDavTaskRemote {
    fn provider(&self) -> &'static str {
        "caldav"
    }

    async fn lists(&self) -> Result<Vec<RemoteList>, RemoteError> {
        if let Some(known) = &self.known {
            return Ok(task_lists(known));
        }
        let collections = discover_collections(&self.client, &self.config, DavKind::Calendar)
            .await
            .map_err(other)?;
        Ok(task_lists(&collections))
    }

    async fn versions(&self, list: &str) -> Result<Vec<(String, Option<String>)>, RemoteError> {
        let collection = self.url(list)?;
        let xml = self.report(&collection, VTODO_ETAG_QUERY).await?;
        let mut versions = Vec::new();
        for response in xml_elements(&xml, "response") {
            let Some(href) = xml_text(&response, "href") else {
                continue;
            };
            let url = resolve_dav_href(&collection, &href).map_err(other)?;
            if url.path() == collection.path() {
                continue;
            }
            versions.push((
                url.to_string(),
                xml_text(&response, "getetag").filter(|etag| !etag.is_empty()),
            ));
        }
        Ok(versions)
    }

    async fn fetch(&self, list: &str, ids: &[String]) -> Result<Vec<RemoteItem>, RemoteError> {
        let collection = self.url(list)?;
        let hrefs = ids
            .iter()
            .map(|id| self.url(id))
            .collect::<Result<Vec<_>, _>>()?;
        let mut items = Vec::with_capacity(hrefs.len());
        for chunk in hrefs.chunks(MAX_MULTIGET_HREFS) {
            match self
                .report(&collection, &calendar_multiget_body(chunk))
                .await
            {
                Ok(xml) => items.extend(self.items(&collection, &xml)?),
                Err(_) => {
                    // A server without multiget: one query for the whole list,
                    // narrowed to what was asked for.
                    let xml = self.report(&collection, VTODO_DATA_QUERY).await?;
                    return Ok(self
                        .items(&collection, &xml)?
                        .into_iter()
                        .filter(|item| ids.contains(&item.remote_id))
                        .collect());
                }
            }
        }
        Ok(items)
    }

    async fn put(
        &self,
        list: &str,
        uid: &str,
        remote_id: Option<&str>,
        version: Option<&str>,
        payload: &str,
    ) -> Result<Written, RemoteError> {
        let url = match remote_id {
            Some(remote_id) => self.url(remote_id)?,
            None => self
                .url(list)?
                .join(&format!("{}.ics", sanitize_path_component(uid)))
                .map_err(|err| other(err.to_string()))?,
        };
        let create = remote_id.is_none();
        match dav_put(&self.client, &self.config, &url, payload, create, version).await {
            Ok(Some(etag)) => Ok(Written {
                remote_id: url.to_string(),
                version: Some(etag),
                adopted: false,
            }),
            // Some servers answer a PUT without an ETag (they rewrote the
            // resource). Ask for it now, so the next edit has a precondition.
            Ok(None) => Ok(Written {
                remote_id: url.to_string(),
                version: self.etag_of(&url).await,
                adopted: false,
            }),
            // The name comes from the task's UID: if the resource there is
            // this task, an earlier PUT landed before the app recorded it, and
            // it is taken over with its ETag. Anything else is not ours.
            Err(DavWriteError::PreconditionFailed) if create => {
                let collection = self.url(list)?;
                let existing = self
                    .fetch(collection.as_str(), &[url.to_string()])
                    .await?
                    .into_iter()
                    .next()
                    .ok_or(RemoteError::Conflict)?;
                if vtodo::parse(&existing.payload).is_some_and(|fields| fields.uid == uid) {
                    Ok(Written {
                        remote_id: url.to_string(),
                        version: existing.version,
                        adopted: true,
                    })
                } else {
                    Err(other(format!("{url} already holds another resource")))
                }
            }
            Err(error) => Err(remote_error(error)),
        }
    }

    async fn delete(&self, remote_id: &str, version: Option<&str>) -> Result<(), RemoteError> {
        let url = self.url(remote_id)?;
        dav_delete(&self.client, &self.config, &url, version)
            .await
            .map_err(remote_error)
    }

    fn decode(&self, payload: &str) -> Option<TaskFields> {
        vtodo::parse(payload)
    }

    fn encode(&self, task: &TaskFields, previous: Option<&str>) -> String {
        match previous {
            Some(previous) => vtodo::patch(previous, task),
            None => vtodo::build(task),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collection(href: &str, components: Option<&[&str]>) -> DavCollection {
        DavCollection {
            href: Url::parse(href).unwrap(),
            name: Some("List".into()),
            color: Some("#00aaff".into()),
            components: components.map(|list| list.iter().map(|value| value.to_string()).collect()),
            writable: None,
        }
    }

    #[test]
    fn only_advertised_vtodo_collections_become_lists() {
        let lists = task_lists(&[
            collection("https://dav.example.test/cal/work/", Some(&["VEVENT"])),
            collection("https://dav.example.test/cal/todo/", Some(&["VTODO"])),
            collection(
                "https://dav.example.test/cal/both/",
                Some(&["VEVENT", "VTODO"]),
            ),
            collection("https://dav.example.test/cal/unknown/", None),
        ]);
        let ids = lists
            .iter()
            .map(|list| list.remote_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "https://dav.example.test/cal/todo/",
                "https://dav.example.test/cal/both/"
            ]
        );
        assert_eq!(lists[0].color.as_deref(), Some("#00aaff"));
    }

    #[test]
    fn multiget_lists_escaped_paths() {
        let body = calendar_multiget_body(&[
            Url::parse("https://dav.example.test/t/a%20b.ics").unwrap(),
            Url::parse("https://dav.example.test/t/c&d.ics").unwrap(),
        ]);
        assert!(body.contains("<d:href>/t/a%20b.ics</d:href>"));
        assert!(body.contains("c&amp;d.ics"));
        assert!(body.contains("calendar-multiget"));
    }
}
