use base64::Engine;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use md5::Md5;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use sqlx::SqlitePool;
#[cfg(target_os = "macos")]
use std::collections::HashSet;

use satchel_core::db::accounts::{AccountError, AccountRepository};
#[cfg(target_os = "macos")]
use satchel_core::db::contacts::{clean_optional, ContactAvatarTarget};
use satchel_core::db::contacts::{
    normalize_email, AvatarSource, ContactAvatarOverrideRequest, ContactError, ContactRecord,
    ContactRepository, ContactUpsertRequest,
};

use crate::utils::oauth::{access_token_from_account_config, OAuthProvider};

use super::error::{CommandError, CommandResult};

const LOCAL_AVATAR_SYNC_ERROR_KEY: &str = "backend.errors.contactAvatarLocalSyncFailed";
const PROVIDER_AVATAR_SYNC_ERROR_KEY: &str = "backend.errors.contactAvatarProviderSyncFailed";
const FALLBACK_AVATAR_SYNC_ERROR_KEY: &str = "backend.errors.contactAvatarFallbackSyncFailed";

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/contacts/")
)]
pub struct ContactDto {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_source: Option<String>,
    pub avatar_locked: i64,
    pub avatar_updated_at: Option<String>,
    pub frequency: i64,
    pub last_contacted: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ContactRecord> for ContactDto {
    fn from(record: ContactRecord) -> Self {
        Self {
            id: record.id,
            email: record.email,
            display_name: record.display_name,
            avatar_url: record.avatar_url,
            avatar_source: record.avatar_source,
            avatar_locked: record.avatar_locked,
            avatar_updated_at: record.avatar_updated_at,
            frequency: record.frequency,
            last_contacted: record.last_contacted,
            notes: record.notes,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/contacts/", optional_fields = nullable))]
pub struct ContactInput {
    #[serde(default)]
    pub id: Option<String>,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub avatar_source: Option<String>,
    #[serde(default)]
    pub avatar_locked: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/contacts/", optional_fields = nullable))]
pub struct ContactAvatarOverrideInput {
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub locked: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/contacts/")
)]
pub struct ContactAvatarSyncSummary {
    pub accounts_checked: i64,
    pub avatars_updated: i64,
    pub fallback_avatars_updated: i64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProviderContactPhoto {
    emails: Vec<String>,
    display_name: Option<String>,
    photo_url: String,
}

#[derive(Debug, Deserialize)]
struct GooglePeopleResponse {
    #[serde(default)]
    connections: Vec<GooglePerson>,
    #[serde(default, rename = "otherContacts")]
    other_contacts: Vec<GooglePerson>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GooglePerson {
    #[serde(default, rename = "emailAddresses")]
    email_addresses: Vec<GoogleEmail>,
    #[serde(default)]
    names: Vec<GoogleName>,
    #[serde(default)]
    photos: Vec<GooglePhoto>,
}

#[derive(Debug, Deserialize)]
struct GoogleEmail {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleName {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GooglePhoto {
    url: Option<String>,
    #[serde(default)]
    default: bool,
}

#[derive(Debug, Deserialize)]
struct GraphContactsResponse {
    #[serde(default)]
    value: Vec<GraphContact>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphContact {
    id: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "emailAddresses")]
    email_addresses: Vec<GraphEmailAddress>,
}

#[derive(Debug, Deserialize)]
struct GraphEmailAddress {
    address: Option<String>,
}

#[tauri::command]
pub async fn list_contacts(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<ContactDto>> {
    ContactRepository::new(pool.inner().as_ref())
        .list()
        .await
        .map(|contacts| contacts.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn sync_contact_avatars(
    pool: tauri::State<'_, SqlitePool>,
    allow_third_party: Option<bool>,
) -> CommandResult<ContactAvatarSyncSummary> {
    sync_contact_avatars_inner(pool.inner(), allow_third_party.unwrap_or(false))
        .await
        .map_err(CommandError::from)
}

/// Refresh already-authorized local photos without contacting remote services
/// or presenting a system permission prompt during application startup.
#[tauri::command]
pub async fn sync_local_contact_avatars(pool: tauri::State<'_, SqlitePool>) -> CommandResult<i64> {
    #[cfg(target_os = "macos")]
    {
        use objc2_contacts::{CNAuthorizationStatus, CNContactStore, CNEntityType};
        let status =
            unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
        if status != CNAuthorizationStatus::Authorized && status != CNAuthorizationStatus::Limited {
            return Ok(0);
        }
    }
    sync_macos_contact_avatars(pool.inner())
        .await
        .map_err(|error| {
            tracing::warn!(error = %error, "local contact avatar refresh failed");
            CommandError::new(LOCAL_AVATAR_SYNC_ERROR_KEY)
        })
}

pub async fn sync_contact_avatars_inner(
    pool: &SqlitePool,
    allow_third_party: bool,
) -> Result<ContactAvatarSyncSummary, AccountError> {
    let accounts = AccountRepository::new(pool)
        .list_active_oauth_configs()
        .await?;

    let client = satchel_core::utils::http::client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("avatar HTTP client initialization failed");
    let mut summary = ContactAvatarSyncSummary {
        accounts_checked: accounts.len() as i64,
        ..Default::default()
    };

    match sync_macos_contact_avatars(pool).await {
        Ok(updated) => summary.avatars_updated += updated,
        Err(error) => {
            tracing::warn!(error = %error, "local contact avatar synchronization failed");
            summary.errors.push(LOCAL_AVATAR_SYNC_ERROR_KEY.to_string());
        }
    }

    for account in accounts {
        let provider = match OAuthProvider::parse(&account.provider) {
            Ok(provider) => provider,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    account_id = %account.id,
                    "contact avatar provider selection failed"
                );
                summary
                    .errors
                    .push(PROVIDER_AVATAR_SYNC_ERROR_KEY.to_string());
                continue;
            }
        };

        let result = fetch_current_contact_photos(
            &client,
            provider,
            crate::utils::oauth::fresh_account_config_for_provider(pool, &account.id, provider),
            "https://people.googleapis.com/v1",
        )
        .await;
        let (token, contacts) = match result {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(error = %error, account_id = %account.id,
                    "contact avatar credential access failed");
                summary
                    .errors
                    .push(PROVIDER_AVATAR_SYNC_ERROR_KEY.to_string());
                continue;
            }
        };
        for contact in contacts.into_iter().take(500) {
            let Some(data_url) = download_avatar_data_url(&contact.photo_url, Some(&token)).await
            else {
                continue;
            };
            for email in contact.emails {
                if upsert_contact_avatar(
                    pool,
                    &email,
                    contact.display_name.as_deref(),
                    &data_url,
                    AvatarSource::ProviderContact,
                    false,
                )
                .await
                .map(|_| true)
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        error = ?error,
                        operation = "provider_contact_avatar",
                        "contact avatar persistence failed"
                    );
                    false
                }) {
                    summary.avatars_updated += 1;
                }
            }
        }
    }

    match sync_fallback_sender_avatars(pool, allow_third_party).await {
        Ok(updated) => {
            summary.fallback_avatars_updated += updated;
            summary.avatars_updated += updated;
        }
        Err(error) => {
            tracing::warn!(error = %error, "fallback contact avatar synchronization failed");
            summary
                .errors
                .push(FALLBACK_AVATAR_SYNC_ERROR_KEY.to_string());
        }
    }

    Ok(summary)
}

#[cfg(not(target_os = "macos"))]
async fn sync_macos_contact_avatars(_pool: &SqlitePool) -> Result<i64, String> {
    Ok(0)
}

#[cfg(target_os = "macos")]
async fn sync_macos_contact_avatars(pool: &SqlitePool) -> Result<i64, String> {
    let targets = contact_avatar_targets(pool, 500).await?;
    if targets.is_empty() {
        return Ok(0);
    }

    let target_emails: HashSet<String> =
        targets.iter().map(|target| target.email.clone()).collect();
    let mut updated = 0_i64;

    for photo in fetch_native_macos_contact_photos(&target_emails)? {
        if upsert_contact_avatar(
            pool,
            &photo.email,
            photo.display_name.as_deref(),
            &photo.data_url,
            AvatarSource::LocalContact,
            false,
        )
        .await
        .map(|_| true)
        .unwrap_or_else(|error| {
            tracing::warn!(
                error = ?error,
                operation = "local_contact_avatar",
                "contact avatar persistence failed"
            );
            false
        }) {
            updated += 1;
        }
    }

    Ok(updated)
}

#[cfg(target_os = "macos")]
async fn contact_avatar_targets(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<ContactAvatarTarget>, String> {
    ContactRepository::new(pool)
        .avatar_targets(limit)
        .await
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "macos")]
struct LocalContactPhoto {
    email: String,
    display_name: Option<String>,
    data_url: String,
}

#[cfg(target_os = "macos")]
fn fetch_native_macos_contact_photos(
    target_emails: &HashSet<String>,
) -> Result<Vec<LocalContactPhoto>, String> {
    use objc2::rc::autoreleasepool;
    use objc2::AnyThread;
    use objc2_contacts::{
        CNAuthorizationStatus, CNContact, CNContactEmailAddressesKey, CNContactFamilyNameKey,
        CNContactFetchRequest, CNContactGivenNameKey, CNContactImageDataAvailableKey,
        CNContactImageDataKey, CNContactStore, CNContactThumbnailImageDataKey, CNEntityType,
        CNKeyDescriptor, CNLabeledValue,
    };
    use objc2_foundation::{NSArray, NSString};

    autoreleasepool(|pool| unsafe {
        let store = CNContactStore::new();
        let status = CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts);
        if status == CNAuthorizationStatus::Denied || status == CNAuthorizationStatus::Restricted {
            return Err("permission denied".to_string());
        }

        let keys: &NSArray<objc2::runtime::ProtocolObject<dyn CNKeyDescriptor>> =
            std::mem::transmute(NSArray::from_slice(&[
                CNContactEmailAddressesKey,
                CNContactGivenNameKey,
                CNContactFamilyNameKey,
                CNContactThumbnailImageDataKey,
                CNContactImageDataAvailableKey,
                CNContactImageDataKey,
            ]));
        let request =
            CNContactFetchRequest::initWithKeysToFetch(CNContactFetchRequest::alloc(), keys);
        request.setUnifyResults(true);
        let result = store
            .enumeratorForContactFetchRequest_error(&request)
            .map_err(|err| err.to_string())?;
        let enumerator = result.value();
        let mut photos = Vec::new();

        for contact in enumerator {
            let contact: objc2::rc::Retained<CNContact> = contact;
            if !contact.imageDataAvailable() {
                continue;
            }
            let display_name = {
                let given = contact.givenName().to_str(pool).trim().to_string();
                let family = contact.familyName().to_str(pool).trim().to_string();
                clean_optional(Some(format!("{given} {family}")))
            };
            let thumbnail = contact.thumbnailImageData().map(|data| data.to_vec());
            let Some(data_url) = contact_photo_data_url(thumbnail.as_deref(), || {
                contact.imageData().map(|data| data.to_vec())
            }) else {
                continue;
            };
            for labeled in contact.emailAddresses() {
                let labeled: objc2::rc::Retained<CNLabeledValue<NSString>> = labeled;
                let email = labeled.value().to_str(pool).trim().to_ascii_lowercase();
                if target_emails.contains(&email) {
                    photos.push(LocalContactPhoto {
                        email,
                        display_name: display_name.clone(),
                        data_url: data_url.clone(),
                    });
                }
            }
        }
        Ok(photos)
    })
}

#[tauri::command]
pub async fn search_contacts(
    pool: tauri::State<'_, crate::db::ReadPool>,
    query: String,
    limit: Option<i64>,
) -> CommandResult<Vec<ContactDto>> {
    ContactRepository::new(pool.inner().as_ref())
        .search(&query, limit)
        .await
        .map(|contacts| contacts.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_contact(
    pool: tauri::State<'_, SqlitePool>,
    input: ContactInput,
) -> CommandResult<ContactDto> {
    ContactRepository::new(pool.inner())
        .upsert(ContactUpsertRequest {
            id: input.id,
            email: input.email,
            display_name: input.display_name,
            avatar_url: input.avatar_url,
            avatar_source: input.avatar_source,
            avatar_locked: input.avatar_locked,
            notes: input.notes,
        })
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_contact_avatar(
    pool: tauri::State<'_, SqlitePool>,
    input: ContactAvatarOverrideInput,
) -> CommandResult<ContactDto> {
    let request = prepare_contact_avatar_override(input).await?;
    ContactRepository::new(pool.inner())
        .set_avatar(request)
        .await
        .map(Into::into)
        .map_err(Into::into)
}

async fn prepare_contact_avatar_override(
    input: ContactAvatarOverrideInput,
) -> CommandResult<ContactAvatarOverrideRequest> {
    let email = normalize_email(&input.email)
        .ok_or_else(|| CommandError::from(ContactError::InvalidEmail))?;
    let avatar_url = normalize_avatar_override(input.avatar_url.as_deref())
        .await
        .map_err(CommandError::unexpected)?;
    Ok(ContactAvatarOverrideRequest {
        email,
        display_name: input.display_name,
        avatar_url,
        locked: input.locked,
    })
}

#[tauri::command]
pub async fn delete_contact(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    ContactRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

pub async fn touch_contact(
    pool: &SqlitePool,
    email: &str,
    display_name: Option<&str>,
) -> Result<(), ContactError> {
    ContactRepository::new(pool)
        .touch(email, display_name)
        .await
}

async fn upsert_contact_avatar(
    pool: &SqlitePool,
    email: &str,
    display_name: Option<&str>,
    avatar_url: &str,
    source: AvatarSource,
    force: bool,
) -> Result<(), ContactError> {
    ContactRepository::new(pool)
        .upsert_avatar(email, display_name, avatar_url, source, force)
        .await
}

// The config future may refresh or observe a reconnect. Both permission checks
// and the token must come from its result, never the pre-await account snapshot.
async fn fetch_current_contact_photos(
    client: &reqwest::Client,
    provider: OAuthProvider,
    current: impl std::future::Future<
        Output = crate::sync::errors::SyncResult<satchel_core::accounts::AccountConfig>,
    >,
    google_base: &str,
) -> Result<(String, Vec<ProviderContactPhoto>), String> {
    let config = current.await.map_err(|err| err.encoded())?;
    if matches!(provider, OAuthProvider::Gmail)
        && crate::utils::oauth::google_scope_granted(
            &config,
            crate::utils::oauth::GOOGLE_CONTACTS_SCOPE,
        ) == Some(false)
    {
        return Ok((String::new(), Vec::new()));
    }
    let token = access_token_from_account_config(provider, &config)
        .await
        .map_err(|err| err.encoded())?;
    let contacts = match provider {
        OAuthProvider::Gmail => {
            fetch_google_contact_photos(
                client,
                &token,
                crate::utils::oauth::google_scope_granted(
                    &config,
                    crate::utils::oauth::GOOGLE_OTHER_CONTACTS_SCOPE,
                ) == Some(true),
                google_base,
            )
            .await?
        }
        OAuthProvider::Outlook => fetch_graph_contact_photos(client, &token).await?,
    };
    Ok((token, contacts))
}

async fn fetch_google_contact_photos(
    client: &reqwest::Client,
    token: &str,
    include_other_contacts: bool,
    base: &str,
) -> Result<Vec<ProviderContactPhoto>, String> {
    let mut contacts = Vec::new();
    fetch_google_people_page(
        client,
        token,
        &format!("{base}/people/me/connections"),
        "connections",
        &mut contacts,
    )
    .await?;

    if include_other_contacts {
        fetch_google_people_page(
            client,
            token,
            &format!("{base}/otherContacts"),
            "otherContacts",
            &mut contacts,
        )
        .await?;
    }

    Ok(contacts)
}

async fn fetch_google_people_page(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    list_key: &str,
    out: &mut Vec<ProviderContactPhoto>,
) -> Result<(), String> {
    let mut next_page_token: Option<String> = None;
    loop {
        let mut request = client
            .get(url)
            .bearer_auth(token)
            .query(&[("pageSize", "1000")]);

        if list_key == "connections" {
            request = request.query(&[("personFields", "names,emailAddresses,photos")]);
        } else {
            request = request
                .query(&[("readMask", "names,emailAddresses,photos")])
                .query(&[
                    ("sources", "READ_SOURCE_TYPE_CONTACT"),
                    ("sources", "READ_SOURCE_TYPE_PROFILE"),
                ]);
        }

        if let Some(token) = next_page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }

        let response = request
            .send()
            .await
            .map_err(|err| format!("google people request failed: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "google people request failed with status {status}: {body}"
            ));
        }

        let body: GooglePeopleResponse = response
            .json()
            .await
            .map_err(|err| format!("google people parse failed: {err}"))?;
        let people = if list_key == "connections" {
            body.connections
        } else {
            body.other_contacts
        };
        out.extend(people.into_iter().filter_map(google_person_photo));

        next_page_token = body.next_page_token;
        if next_page_token.is_none() {
            break;
        }
    }
    Ok(())
}

fn google_person_photo(person: GooglePerson) -> Option<ProviderContactPhoto> {
    let emails = contact_photo_emails(
        person
            .email_addresses
            .into_iter()
            .filter_map(|email| email.value),
    );
    if emails.is_empty() {
        return None;
    }
    let photo_url = person
        .photos
        .into_iter()
        .filter(|photo| !photo.default)
        .find_map(|photo| photo.url)?;
    let display_name = person.names.into_iter().find_map(|name| name.display_name);
    Some(ProviderContactPhoto {
        emails,
        display_name,
        photo_url,
    })
}

async fn fetch_graph_contact_photos(
    client: &reqwest::Client,
    token: &str,
) -> Result<Vec<ProviderContactPhoto>, String> {
    let mut contacts = Vec::new();
    let mut next_url = Some(
        "https://graph.microsoft.com/v1.0/me/contacts?$select=id,displayName,emailAddresses&$top=100"
            .to_string(),
    );

    while let Some(url) = next_url.take() {
        let response = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|err| format!("graph contacts request failed: {err}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "graph contacts request failed with status {status}: {body}"
            ));
        }

        let body: GraphContactsResponse = response
            .json()
            .await
            .map_err(|err| format!("graph contacts parse failed: {err}"))?;

        for contact in body.value {
            let emails = contact_photo_emails(
                contact
                    .email_addresses
                    .into_iter()
                    .filter_map(|email| email.address),
            );
            if emails.is_empty() {
                continue;
            }
            let photo_url = format!(
                "https://graph.microsoft.com/v1.0/me/contacts/{}/photo/$value",
                contact.id
            );
            contacts.push(ProviderContactPhoto {
                emails,
                display_name: contact.display_name,
                photo_url,
            });
        }

        next_url = body.next_link;
        if contacts.len() >= 500 {
            break;
        }
    }

    Ok(contacts)
}

async fn download_avatar_data_url(url: &str, bearer_token: Option<&str>) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let client = satchel_core::utils::public_http::client(&url, std::time::Duration::from_secs(10))
        .await
        .ok()?;
    let mut request = client.get(url);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_ascii_lowercase();
    if !content_type.starts_with("image/") {
        return None;
    }
    let bytes = satchel_core::utils::public_http::read_limited(response, 4 * 1024 * 1024)
        .await
        .ok()?;
    if content_type.starts_with("image/svg") {
        return svg_data_url(&bytes);
    }
    normalized_image_data_url(&bytes)
}

async fn normalize_avatar_override(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if raw.starts_with("data:image/svg+xml") {
        let Some((_, encoded)) = raw.split_once(',') else {
            return Err("invalid SVG data URL".to_string());
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "invalid SVG data URL".to_string())?;
        return svg_data_url(&bytes)
            .map(Some)
            .ok_or_else(|| "invalid SVG image".to_string());
    }

    if raw.starts_with("data:image/") {
        let Some((_, encoded)) = raw.split_once(',') else {
            return Err("invalid image data URL".to_string());
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "invalid image data URL".to_string())?;
        return normalized_image_data_url(&bytes)
            .map(Some)
            .ok_or_else(|| "unsupported image".to_string());
    }

    if raw.starts_with("https://") || raw.starts_with("http://") {
        return download_avatar_data_url(raw, None)
            .await
            .map(Some)
            .ok_or_else(|| "avatar URL could not be loaded".to_string());
    }

    Err("avatar must be an image URL or data URL".to_string())
}

fn svg_data_url(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > 256 * 1024 {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let lower = text.to_ascii_lowercase();
    if !lower.contains("<svg") || lower.contains("<script") {
        return None;
    }
    Some(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn normalized_image_data_url(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let png = encode_avatar_png(img)?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

fn contact_photo_emails(addresses: impl Iterator<Item = String>) -> Vec<String> {
    let mut emails = Vec::new();
    for address in addresses {
        if let Some(email) = normalize_email(&address).filter(|email| !emails.contains(email)) {
            emails.push(email);
        }
    }
    emails
}

#[cfg(any(target_os = "macos", test))]
fn contact_photo_data_url(
    thumbnail: Option<&[u8]>,
    full_image: impl FnOnce() -> Option<Vec<u8>>,
) -> Option<String> {
    thumbnail
        .and_then(normalized_image_data_url)
        .or_else(|| full_image().as_deref().and_then(normalized_image_data_url))
}

fn encode_avatar_png(img: DynamicImage) -> Option<Vec<u8>> {
    let rgba = if img.width() > 192 || img.height() > 192 {
        img.resize(192, 192, FilterType::Lanczos3)
    } else {
        img
    }
    .to_rgba8();
    let mut cursor = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(rgba)
        .write_to(&mut cursor, ImageFormat::Png)
        .ok()?;
    let out = cursor.into_inner();
    if out.is_empty() || out.len() > 512 * 1024 {
        None
    } else {
        Some(out)
    }
}

async fn sync_fallback_sender_avatars(
    pool: &SqlitePool,
    allow_third_party: bool,
) -> Result<i64, String> {
    if !allow_third_party {
        return Ok(0);
    }
    let contacts = ContactRepository::new(pool)
        .fallback_avatar_targets(300)
        .await
        .map_err(|err| err.to_string())?;

    let mut updated = 0_i64;
    for contact in contacts {
        let Some(email) = normalize_email(&contact.email) else {
            continue;
        };
        let domain = email.split('@').nth(1).unwrap_or_default();

        if let Some(data_url) = fetch_bimi_avatar(domain).await {
            if upsert_contact_avatar(
                pool,
                &email,
                contact.display_name.as_deref(),
                &data_url,
                AvatarSource::Bimi,
                false,
            )
            .await
            .map(|_| true)
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = ?error,
                    operation = "bimi_avatar",
                    "contact avatar persistence failed"
                );
                false
            }) {
                updated += 1;
                continue;
            }
        }

        if let Some(data_url) = fetch_gravatar_avatar(&email).await {
            if upsert_contact_avatar(
                pool,
                &email,
                contact.display_name.as_deref(),
                &data_url,
                AvatarSource::Gravatar,
                false,
            )
            .await
            .map(|_| true)
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = ?error,
                    operation = "gravatar_avatar",
                    "contact avatar persistence failed"
                );
                false
            }) {
                updated += 1;
                continue;
            }
        }

        if !looks_like_personal_contact(contact.display_name.as_deref().unwrap_or(&email)) {
            if let Some(data_url) = fetch_favicon_avatar(domain).await {
                if upsert_contact_avatar(
                    pool,
                    &email,
                    contact.display_name.as_deref(),
                    &data_url,
                    AvatarSource::Favicon,
                    false,
                )
                .await
                .map(|_| true)
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        error = ?error,
                        operation = "favicon_avatar",
                        "contact avatar persistence failed"
                    );
                    false
                }) {
                    updated += 1;
                }
            }
        }
    }
    Ok(updated)
}

async fn fetch_bimi_avatar(domain: &str) -> Option<String> {
    let mut query_url = reqwest::Url::parse("https://cloudflare-dns.com/dns-query").ok()?;
    query_url
        .query_pairs_mut()
        .append_pair("name", &format!("default._bimi.{domain}"))
        .append_pair("type", "TXT");
    let dns_client =
        satchel_core::utils::public_http::client(&query_url, std::time::Duration::from_secs(10))
            .await
            .ok()?;
    let response = dns_client
        .get(query_url)
        .header(ACCEPT, "application/dns-json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = satchel_core::utils::public_http::read_limited(response, 256 * 1024)
        .await
        .ok()?;
    let body: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let answers = body.get("Answer")?.as_array()?;
    for answer in answers {
        let data = answer.get("data")?.as_str()?.replace('"', "");
        if !data.to_ascii_lowercase().starts_with("v=bimi1") {
            continue;
        }
        let logo = data
            .split(';')
            .filter_map(|part| part.trim().split_once('='))
            .find_map(|(key, value)| (key.trim().eq_ignore_ascii_case("l")).then(|| value.trim()));
        if let Some(url) = logo.filter(|url| url.starts_with("https://")) {
            if let Some(data_url) = download_avatar_data_url(url, None).await {
                return Some(data_url);
            }
        }
    }
    None
}

async fn fetch_gravatar_avatar(email: &str) -> Option<String> {
    let normalized = email.trim().to_ascii_lowercase();
    let md5_hash = format!("{:x}", Md5::digest(normalized.as_bytes()));
    let sha256_hash = format!("{:x}", sha2::Sha256::digest(normalized.as_bytes()));
    for hash in [md5_hash, sha256_hash] {
        let url = format!("https://www.gravatar.com/avatar/{hash}?d=404&s=192");
        if let Some(data_url) = download_avatar_data_url(&url, None).await {
            return Some(data_url);
        }
    }
    None
}

async fn fetch_favicon_avatar(domain: &str) -> Option<String> {
    let mut url = reqwest::Url::parse("https://www.google.com/s2/favicons").ok()?;
    url.query_pairs_mut()
        .append_pair("domain", domain)
        .append_pair("sz", "128");
    download_avatar_data_url(url.as_str(), None).await
}

fn looks_like_personal_contact(label: &str) -> bool {
    let primary = label.split('|').next().unwrap_or(label).trim();
    let tokens: Vec<&str> = primary.split_whitespace().collect();
    (2..=4).contains(&tokens.len())
        && tokens.iter().all(|token| {
            token
                .chars()
                .next()
                .map(|ch| ch.is_uppercase())
                .unwrap_or(false)
        })
}

impl From<ContactError> for CommandError {
    fn from(error: ContactError) -> Self {
        match error {
            ContactError::InvalidEmail => Self::new("backend.errors.contactEmailInvalid"),
            ContactError::Persistence(error) => {
                tracing::error!(error = ?error, "contact persistence failed");
                Self::new("backend.errors.contactPersistenceFailed")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn contact_requests_use_scopes_and_token_from_config_returned_after_reconnect() {
        use crate::utils::oauth::{
            OAuthIdentity, OAuthTokenPayload, GOOGLE_CONTACTS_SCOPE, GOOGLE_OTHER_CONTACTS_SCOPE,
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (old_regular, old_other, new_regular, new_other) in [
            (true, true, true, false),
            (true, false, true, true),
            (false, false, true, false),
            (true, true, false, true),
        ] {
            crate::utils::crypto::seed_master_key_for_tests();
            let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
            crate::db::init(&pool).await.unwrap();
            let scopes = |regular: bool, other: bool| {
                [
                    (regular, GOOGLE_CONTACTS_SCOPE),
                    (other, GOOGLE_OTHER_CONTACTS_SCOPE),
                ]
                .into_iter()
                .filter_map(|(granted, scope)| granted.then_some(scope))
                .collect::<Vec<_>>()
                .join(" ")
            };
            let old = serde_json::json!({"oauth": {"provider":"gmail", "client_id":"old-client",
                "access_token":"old-access", "refresh_token":"old-refresh",
                "scope":scopes(old_regular, old_other), "expires_at":"2000-01-01T00:00:00Z"}});
            sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
                VALUES ('contacts-race', 'Fixture', 'fixture@example.test', 'gmail', ?, 'encrypted-in-config', '2026-01-01', '2026-01-01')")
                .bind(satchel_core::accounts::encrypt_account_config("contacts-race", &old.to_string()).unwrap())
                .execute(&pool).await.unwrap();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let mut paths = Vec::new();
                loop {
                    let (mut stream, _) = tokio::select! {
                        _ = &mut stop_rx => break,
                        connection = listener.accept() => connection.unwrap(),
                    };
                    let mut request = Vec::new();
                    loop {
                        let mut buf = [0; 2048];
                        let read = stream.read(&mut buf).await.unwrap();
                        assert!(read > 0);
                        request.extend_from_slice(&buf[..read]);
                        if request.windows(4).any(|part| part == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(request).unwrap();
                    assert!(request
                        .to_ascii_lowercase()
                        .contains("authorization: bearer new-access"));
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_owned();
                    let other = path.starts_with("/otherContacts?");
                    // An erroneous stale-scope request would fail and discard
                    // valid regular contacts in the previous implementation.
                    let (status, body) = if other && !new_other {
                        ("403 Forbidden", "{}".to_string())
                    } else {
                        let list = if other {
                            "otherContacts"
                        } else {
                            "connections"
                        };
                        (
                            "200 OK",
                            serde_json::json!({(list): [{
                                "emailAddresses":[{"value":"person@example.test"}],
                                "photos":[{"url":"https://photos.example.test/person.png"}]
                            }]})
                            .to_string(),
                        )
                    };
                    paths.push(path);
                    let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
                paths
            });
            let (waiting_tx, waiting_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn({
                let pool = pool.clone();
                async move {
                    fetch_current_contact_photos(
                        &reqwest::Client::new(),
                        OAuthProvider::Gmail,
                        async {
                            waiting_tx.send(()).unwrap();
                            release_rx.await.unwrap();
                            crate::utils::oauth::fresh_account_config_for_provider(
                                &pool,
                                "contacts-race",
                                OAuthProvider::Gmail,
                            )
                            .await
                        },
                        &base,
                    )
                    .await
                    .unwrap()
                }
            });
            waiting_rx.await.unwrap();
            let client = crate::utils::oauth::OAuthClientConfig {
                provider: OAuthProvider::Gmail,
                client_id: "new-client".into(),
                client_secret: None,
                redirect_uri: "http://127.0.0.1:1".into(),
                auth_url: "https://oauth.example.test/authorize".into(),
                token_url: "https://oauth.example.test/token".into(),
                scopes: Vec::new(),
            };
            AccountRepository::new(&pool)
                .connect_oauth(
                    "contacts-race",
                    &client,
                    &OAuthTokenPayload {
                        access_token: "new-access".into(),
                        refresh_token: Some("new-refresh".into()),
                        token_type: None,
                        scope: Some(scopes(new_regular, new_other)),
                        expires_at: Some("2099-01-01T00:00:00Z".into()),
                    },
                    &OAuthIdentity {
                        email: "fixture@example.test".into(),
                        display_name: None,
                    },
                    "2026-01-01",
                )
                .await
                .unwrap();
            release_tx.send(()).unwrap();
            let (_, contacts) = task.await.unwrap();
            stop_tx.send(()).unwrap();
            let requests = server.await.unwrap();
            let expected = usize::from(new_regular) * (1 + usize::from(new_other));
            assert_eq!(contacts.len(), expected);
            assert_eq!(requests.len(), expected);
            assert_eq!(
                requests
                    .iter()
                    .any(|path| path.starts_with("/otherContacts?")),
                new_regular && new_other
            );
        }
    }

    #[tokio::test]
    async fn third_party_avatar_import_requires_explicit_consent() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // No contacts table: default-off must return before querying targets or fetching DNS.
        assert_eq!(
            super::sync_fallback_sender_avatars(&pool, false)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn avatar_urls_cannot_reach_plain_http_or_private_hosts() {
        for url in [
            "http://example.com/logo.png",
            "https://127.0.0.1/logo.png",
            "https://[::1]/logo.svg",
        ] {
            assert!(super::download_avatar_data_url(url, None).await.is_none());
        }
    }

    use super::*;

    #[test]
    fn normalizes_favicon_ico_images() {
        assert!(normalized_image_data_url(include_bytes!("../../icons/icon.ico")).is_some());
    }

    #[test]
    fn normalizes_webp_and_gif_contact_images_without_upscaling() {
        for format in [ImageFormat::WebP, ImageFormat::Gif] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            DynamicImage::new_rgba8(32, 32)
                .write_to(&mut bytes, format)
                .expect("image fixture");
            let url = normalized_image_data_url(&bytes.into_inner()).expect("normalized image");
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(url.split_once(',').expect("data URL").1)
                .expect("base64");
            let normalized = image::load_from_memory(&decoded).expect("PNG");
            assert_eq!((normalized.width(), normalized.height()), (32, 32));
        }
    }

    #[test]
    fn imports_full_contact_photo_when_thumbnail_is_missing_or_unusable() {
        let image = include_bytes!("../../icons/32x32.png");
        for thumbnail in [None, Some(b"unavailable-thumbnail".as_slice())] {
            assert!(contact_photo_data_url(thumbnail, || Some(image.to_vec())).is_some());
        }
        assert!(contact_photo_data_url(Some(image), || panic!(
            "valid thumbnail needs no full photo"
        ))
        .is_some());
    }

    #[test]
    fn provider_photo_covers_every_contact_email_alias() {
        let person: GooglePerson = serde_json::from_value(serde_json::json!({
            "emailAddresses": [{"value":"person@example.test"}, {"value":" Work@Example.test "}, {"value":"work@example.test"}],
            "photos": [{"url":"https://example.test/photo.png", "default":false}]
        })).expect("provider contact");
        assert_eq!(
            google_person_photo(person).expect("photo").emails,
            vec!["person@example.test", "work@example.test"]
        );
    }

    #[test]
    fn contact_errors_use_stable_redacted_keys() {
        assert_eq!(
            CommandError::from(ContactError::InvalidEmail).key,
            "backend.errors.contactEmailInvalid"
        );
        let detail = "near SELECT: contact secret";
        let error = CommandError::from(ContactError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_string(&error).expect("command error JSON");
        assert_eq!(error.key, "backend.errors.contactPersistenceFailed");
        assert!(!serialized.contains(detail));
    }

    #[tokio::test]
    async fn manual_avatar_rejects_invalid_email_before_avatar_processing() {
        let error = prepare_contact_avatar_override(ContactAvatarOverrideInput {
            email: "not-an-email".to_string(),
            display_name: None,
            avatar_url: Some("not-an-image".to_string()),
            locked: None,
        })
        .await
        .expect_err("invalid email must win");

        assert_eq!(error.key, "backend.errors.contactEmailInvalid");
    }
}
