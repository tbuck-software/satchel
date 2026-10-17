//! Shared bounded CPU-work caches for message bodies and signature verification.
pub(crate) use satchel_core::email::processing::{
    cached, fingerprint, sanitize_message_html, ProcessingCache,
};
