//! Bounded CPU-work caches for message bodies and signature verification.
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(60);

struct Entry<T> {
    message_id: String,
    fingerprint: [u8; 32],
    value: T,
    bytes: usize,
    created_at: Instant,
}

pub struct ProcessingCache<T> {
    entries: VecDeque<Entry<T>>,
    bytes: usize,
}

impl<T> Default for ProcessingCache<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
        }
    }
}

impl<T: Clone> ProcessingCache<T> {
    fn get(&mut self, message_id: &str, fingerprint: [u8; 32]) -> Option<T> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.message_id == message_id)?;
        let entry = self.entries.remove(index)?;
        self.bytes -= entry.bytes;
        if entry.fingerprint != fingerprint || entry.created_at.elapsed() >= MAX_AGE {
            return None;
        }
        let value = entry.value.clone();
        self.bytes += entry.bytes;
        self.entries.push_back(entry);
        Some(value)
    }

    fn insert(&mut self, message_id: String, fingerprint: [u8; 32], value: T, bytes: usize) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.message_id == message_id)
        {
            if let Some(entry) = self.entries.remove(index) {
                self.bytes -= entry.bytes;
            }
        }
        let bytes = bytes.saturating_add(message_id.len());
        if bytes > MAX_BYTES {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes -= entry.bytes;
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.entries.push_back(Entry {
            message_id,
            fingerprint,
            value,
            bytes,
            created_at: Instant::now(),
        });
    }
}

pub fn fingerprint(parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.len().to_le_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

/// Call from a blocking worker. The lock is never held while CPU work runs.
pub fn cached<T: Clone>(
    cache: &Mutex<ProcessingCache<T>>,
    message_id: String,
    fingerprint: [u8; 32],
    compute: impl FnOnce() -> (T, usize),
) -> T {
    if let Ok(mut cache) = cache.lock() {
        if let Some(value) = cache.get(&message_id, fingerprint) {
            return value;
        }
    }
    let (value, bytes) = compute();
    if let Ok(mut cache) = cache.lock() {
        cache.insert(message_id, fingerprint, value.clone(), bytes);
    }
    value
}

static HTML_CACHE: LazyLock<Mutex<ProcessingCache<String>>> =
    LazyLock::new(|| Mutex::new(ProcessingCache::default()));

pub async fn sanitize_message_html(
    message_id: String,
    html: String,
) -> Result<String, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        cached(
            &HTML_CACHE,
            message_id,
            fingerprint(&[html.as_bytes()]),
            || {
                let sanitized = crate::email::sanitizer::sanitize_stored_email_html(&html);
                let bytes = sanitized.len();
                (sanitized, bytes)
            },
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_opens_reuse_work_until_content_changes() {
        let cache = Mutex::new(ProcessingCache::default());
        let mut computations = 0;
        for body in [
            b"first".as_slice(),
            b"first".as_slice(),
            b"changed".as_slice(),
        ] {
            let value = cached(&cache, "message".into(), fingerprint(&[body]), || {
                computations += 1;
                (body.to_vec(), body.len())
            });
            assert_eq!(value, body);
        }
        assert_eq!(computations, 2);
    }

    #[test]
    fn body_and_key_changes_invalidate_cached_results() {
        let mut cache = ProcessingCache::default();
        let first = fingerprint(&[b"body", b"key"]);
        cache.insert("message".into(), first, "clean".to_string(), 5);
        assert_eq!(cache.get("message", first).as_deref(), Some("clean"));
        assert!(cache
            .get("message", fingerprint(&[b"edited", b"key"]))
            .is_none());
        cache.insert("message".into(), first, "clean".into(), 5);
        assert!(cache
            .get("message", fingerprint(&[b"body", b"new key"]))
            .is_none());
        assert_ne!(fingerprint(&[b"ab", b"c"]), fingerprint(&[b"a", b"bc"]));
    }

    #[test]
    fn evicts_by_count_bytes_and_age() {
        let mut cache = ProcessingCache::default();
        for index in 0..=MAX_ENTRIES {
            cache.insert(index.to_string(), [0; 32], index, 1);
        }
        assert!(cache.get("0", [0; 32]).is_none());
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        cache.insert("large".into(), [0; 32], 100, MAX_BYTES - 5);
        assert_eq!(cache.entries.len(), 1);
        cache.insert("oversize".into(), [0; 32], 100, MAX_BYTES + 1);
        assert_eq!(cache.entries.len(), 1);
        cache.entries[0].created_at = Instant::now() - MAX_AGE;
        assert!(cache.get("large", [0; 32]).is_none());
        assert_eq!(cache.bytes, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn output_sanitizing_uses_the_blocking_pool() {
        let html = sanitize_message_html(
            "test".into(),
            "<p onclick=bad()>Hi<script>bad()</script></p>".into(),
        )
        .await
        .unwrap();
        assert_eq!(html, "<p>Hi</p>");
    }
}
