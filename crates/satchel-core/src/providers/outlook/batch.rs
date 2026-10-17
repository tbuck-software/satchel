//! Bulk mutations through Microsoft Graph `$batch`, with throttling honoured.
//!
//! One request per message meant 200 round trips to archive 200 messages, and
//! the first failure or 429 abandoned the rest. Backoff parameters are Gmail's.

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::providers::gmail::{
    rate_limit_delay, MAX_BLOCK_DOUBLINGS, MAX_RATE_LIMIT_WAIT, RATE_LIMIT_ATTEMPTS,
};
use crate::sync::errors::{http_failure, SyncError, SyncResult};

/// Graph's ceiling for requests in one batch.
pub(super) const BATCH_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphOperation {
    pub remote_id: String,
    pub method: &'static str,
    /// Relative to the API version root: `/me/messages/{id}`.
    pub url: String,
    pub body: Option<Value>,
    pub immutable_ids: bool,
    /// A 404 means the goal is already reached (deleting a deleted message).
    pub missing_is_done: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct OperationFailure {
    pub remote_id: String,
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct BatchOutcome {
    /// Positions in the operations sent that were throttled.
    pub throttled: Vec<usize>,
    pub retry_after: Option<Duration>,
    pub failed: Vec<OperationFailure>,
}

pub(super) fn batch_payload(operations: &[&GraphOperation]) -> Value {
    let requests: Vec<Value> = operations
        .iter()
        .enumerate()
        .map(|(index, operation)| {
            let mut request = json!({
                "id": index.to_string(),
                "method": operation.method,
                "url": operation.url,
            });
            let mut headers = Map::new();
            if operation.immutable_ids {
                headers.insert("Prefer".into(), json!("IdType=\"ImmutableId\""));
            }
            if let Some(body) = &operation.body {
                request["body"] = body.clone();
                headers.insert("Content-Type".into(), json!("application/json"));
            }
            if !headers.is_empty() {
                request["headers"] = Value::Object(headers);
            }
            request
        })
        .collect();
    json!({ "requests": requests })
}

fn header<'a>(response: &'a Value, name: &str) -> Option<&'a str> {
    response
        .get("headers")?
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))?
        .1
        .as_str()
}

/// Maps a `$batch` response back onto the operations that were sent.
pub(super) fn classify(operations: &[&GraphOperation], payload: &Value) -> BatchOutcome {
    let responses = payload
        .get("responses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut outcome = BatchOutcome::default();
    for (index, operation) in operations.iter().enumerate() {
        let id = index.to_string();
        let Some(response) = responses
            .iter()
            .find(|response| response.get("id").and_then(Value::as_str) == Some(id.as_str()))
        else {
            outcome.failed.push(OperationFailure {
                remote_id: operation.remote_id.clone(),
                status: 500,
                body: "no response in batch".into(),
            });
            continue;
        };
        let status = response
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .unwrap_or(500);
        if (200..300).contains(&status) || (status == 404 && operation.missing_is_done) {
            continue;
        }
        let body = response
            .get("body")
            .map(Value::to_string)
            .unwrap_or_default();
        if matches!(status, 429 | 503) {
            outcome.throttled.push(index);
            let delay = rate_limit_delay(header(response, "Retry-After"), &body, 0);
            if header(response, "Retry-After").is_some() {
                outcome.retry_after = Some(outcome.retry_after.map_or(delay, |d| d.max(delay)));
            }
            continue;
        }
        outcome.failed.push(OperationFailure {
            remote_id: operation.remote_id.clone(),
            status,
            body,
        });
    }
    outcome
}

/// Without a server hint: 1 s, 2 s, 4 s, capped like Gmail's doubling.
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.saturating_sub(1).min(MAX_BLOCK_DOUBLINGS))
}

/// The error for operations that failed, naming them, or `Ok` for none.
///
/// Retryable if any failure is, so the queue runs the mutation again; the
/// operations are idempotent, so repeating the ones that worked is harmless.
pub(super) fn failures_result(label: &str, failures: &[OperationFailure]) -> SyncResult<()> {
    let Some(worst) = failures
        .iter()
        .find(|failure| failure.status == 429 || failure.status >= 500)
        .or(failures.first())
    else {
        return Ok(());
    };
    let ids = failures
        .iter()
        .map(|failure| failure.remote_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(http_failure(
        "outlook",
        &format!("{label} for {} message(s) [{ids}]", failures.len()),
        worst.status,
        &worst.body,
    ))
}

/// Sends a request again after a 429/503 instead of failing the run.
pub(super) async fn send_throttled<F>(build: F, label: &str) -> SyncResult<reqwest::Response>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 1u32;
    loop {
        let response = build()
            .send()
            .await
            .map_err(|err| format!("outlook {label} failed: {err}"))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.text().await.unwrap_or_default();
        if !matches!(status.as_u16(), 429 | 503) {
            return Err(http_failure("outlook", label, status.as_u16(), &body));
        }
        let delay = match retry_after.as_deref() {
            Some(value) => rate_limit_delay(Some(value), &body, 0),
            None => backoff(attempt),
        };
        if attempt >= RATE_LIMIT_ATTEMPTS as u32 || delay > MAX_RATE_LIMIT_WAIT {
            return Err(SyncError::rate_limited(
                format!(
                    "outlook {label} throttled ({status}); Graph asks for {}s",
                    delay.as_secs()
                ),
                Some(delay),
            ));
        }
        tracing::warn!("outlook {label} throttled ({status}); waiting {delay:?}");
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

/// Runs `operations` in batches of [`BATCH_LIMIT`], retrying throttled ones.
pub(super) async fn run(
    client: &reqwest::Client,
    token: &str,
    label: &str,
    operations: &[GraphOperation],
) -> SyncResult<()> {
    let endpoint = format!("{}/$batch", super::graph_api_base());
    let mut failures = Vec::new();
    for chunk in operations.chunks(BATCH_LIMIT) {
        let mut pending: Vec<&GraphOperation> = chunk.iter().collect();
        let mut attempt = 1u32;
        loop {
            let payload = batch_payload(&pending);
            let response = send_throttled(
                || client.post(&endpoint).bearer_auth(token).json(&payload),
                label,
            )
            .await?;
            let value: Value = response
                .json()
                .await
                .map_err(|err| format!("outlook {label} batch parse failed: {err}"))?;
            let outcome = classify(&pending, &value);
            failures.extend(outcome.failed);
            if outcome.throttled.is_empty() {
                break;
            }
            let delay = outcome.retry_after.unwrap_or_else(|| backoff(attempt));
            if attempt >= RATE_LIMIT_ATTEMPTS as u32 || delay > MAX_RATE_LIMIT_WAIT {
                return Err(SyncError::rate_limited(
                    format!(
                        "outlook {label} throttled for {} message(s); Graph asks for {}s",
                        outcome.throttled.len(),
                        delay.as_secs()
                    ),
                    Some(delay),
                ));
            }
            tracing::warn!(
                "outlook {label}: {} operation(s) throttled; waiting {delay:?}",
                outcome.throttled.len()
            );
            tokio::time::sleep(delay).await;
            pending = outcome
                .throttled
                .iter()
                .map(|index| pending[*index])
                .collect();
            attempt += 1;
        }
    }
    failures_result(label, &failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(remote_id: &str) -> GraphOperation {
        GraphOperation {
            remote_id: remote_id.into(),
            method: "DELETE",
            url: format!("/me/messages/{remote_id}"),
            body: None,
            immutable_ids: false,
            missing_is_done: false,
        }
    }

    #[test]
    fn operations_split_into_batches_of_twenty() {
        let two_hundred: Vec<_> = (0..200).map(|i| operation(&i.to_string())).collect();
        let two_hundred_one: Vec<_> = (0..201).map(|i| operation(&i.to_string())).collect();

        assert_eq!(two_hundred.chunks(BATCH_LIMIT).count(), 10);
        assert_eq!(two_hundred_one.chunks(BATCH_LIMIT).count(), 11);
    }

    #[test]
    fn a_batch_request_carries_method_url_body_and_headers() {
        let mut moving = operation("a");
        moving.method = "POST";
        moving.url = "/me/messages/a/move".into();
        moving.body = Some(json!({ "destinationId": "archive" }));
        moving.immutable_ids = true;

        let payload = batch_payload(&[&moving, &operation("b")]);

        assert_eq!(
            payload,
            json!({ "requests": [
                {
                    "id": "0", "method": "POST", "url": "/me/messages/a/move",
                    "body": { "destinationId": "archive" },
                    "headers": {
                        "Prefer": "IdType=\"ImmutableId\"",
                        "Content-Type": "application/json"
                    }
                },
                { "id": "1", "method": "DELETE", "url": "/me/messages/b" }
            ]})
        );
    }

    #[test]
    fn responses_map_back_to_their_remote_ids() {
        let mut gone = operation("gone");
        gone.missing_is_done = true;
        let sent = [
            operation("ok"),
            operation("refused"),
            operation("slow"),
            gone,
            operation("lost"),
        ];
        let sent: Vec<&GraphOperation> = sent.iter().collect();
        // Graph does not keep the order of the requests.
        let payload = json!({ "responses": [
            { "id": "2", "status": 429, "headers": { "Retry-After": "2" } },
            { "id": "0", "status": 204 },
            { "id": "3", "status": 404 },
            { "id": "1", "status": 403, "body": { "error": { "code": "ErrorAccessDenied" } } }
        ]});

        let outcome = classify(&sent, &payload);

        assert_eq!(outcome.throttled, [2]);
        assert_eq!(outcome.retry_after, Some(Duration::from_secs(2)));
        let failed: Vec<_> = outcome
            .failed
            .iter()
            .map(|failure| (failure.remote_id.as_str(), failure.status))
            .collect();
        assert_eq!(failed, [("refused", 403), ("lost", 500)]);
    }

    #[test]
    fn failures_name_their_messages_and_stay_retryable_if_any_is() {
        let failures = [
            OperationFailure {
                remote_id: "a".into(),
                status: 403,
                body: String::new(),
            },
            OperationFailure {
                remote_id: "b".into(),
                status: 503,
                body: String::new(),
            },
        ];

        let error = failures_result("move_messages", &failures).unwrap_err();

        assert!(!error.is_permanent());
        assert!(error.to_string().contains("[a, b]"), "{error}");
        assert!(failures_result("move_messages", &[]).is_ok());
    }

    #[test]
    fn backoff_doubles_up_to_the_shared_ceiling() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(4));
        assert_eq!(backoff(40), Duration::from_secs(1 << MAX_BLOCK_DOUBLINGS));
    }
}
