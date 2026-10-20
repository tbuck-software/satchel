//! Shared parsing for provider sync cursors.
//!
//! Provider cursor states remain provider-owned because their fields and
//! migration rules differ. This module only owns the common envelope: empty
//! input, current JSON, and the provider's legacy fallback.

use serde::de::DeserializeOwned;

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedCursor<T> {
    pub(crate) state: T,
    pub(crate) had_checkpoint: bool,
    pub(crate) was_legacy: bool,
}

/// Parse a provider cursor while preserving its legacy representation.
///
/// Non-empty input that is not current JSON is deliberately still reported as
/// a legacy checkpoint. The provider decides how that legacy value maps to its
/// state, including whether an unrecognised value becomes the default state.
pub(crate) fn parse_state<T, F>(cursor: Option<&str>, parse_legacy: F) -> ParsedCursor<T>
where
    T: Default + DeserializeOwned,
    F: FnOnce(&str) -> T,
{
    let Some(raw) = cursor.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return ParsedCursor::default();
    };

    if let Ok(state) = serde_json::from_str::<T>(raw) {
        return ParsedCursor {
            state,
            had_checkpoint: true,
            was_legacy: false,
        };
    }

    ParsedCursor {
        state: parse_legacy(raw),
        had_checkpoint: true,
        was_legacy: true,
    }
}

/// Read backfill progress from the provider cursor formats used by mail sync.
///
/// Gmail and Outlook use message counters; IMAP uses sequence counters plus
/// the new messages a bounded incremental run left for the next one. Keep the
/// field priority and permissive JSON fallback stable because this is also
/// used by the Tauri sync orchestrator while it aggregates account progress.
pub fn backfill_stats(cursor: &str) -> Option<(usize, usize)> {
    let value = serde_json::from_str::<serde_json::Value>(cursor).ok()?;
    if let Some(remaining) = value
        .get("backfill_remaining_messages")
        .and_then(serde_json::Value::as_u64)
    {
        let remaining = remaining as usize;
        let total = value
            .get("backfill_total_messages")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(remaining as u64) as usize;
        return Some((remaining, total.max(remaining)));
    }
    let incremental = value
        .get("incremental_pending")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let backfill = value
        .get("backfill_next_seq")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let total = value
        .get("backfill_total_seq")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(backfill as u64) as usize;
    let remaining = backfill + incremental;
    Some((remaining, (total + incremental).max(remaining)))
}

pub fn backfill_pending(cursor: &str) -> bool {
    backfill_stats(cursor)
        .map(|(remaining, _)| remaining > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{backfill_pending, backfill_stats, parse_state};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
    struct TestState {
        value: u64,
    }

    #[test]
    fn empty_cursor_starts_with_default_state() {
        for cursor in [None, Some(""), Some("  \n\t ")] {
            let parsed = parse_state(cursor, |_| TestState { value: 9 });
            assert_eq!(parsed.state, TestState::default());
            assert!(!parsed.had_checkpoint);
            assert!(!parsed.was_legacy);
        }
    }

    #[test]
    fn current_json_is_returned_without_legacy_fallback() {
        let parsed = parse_state(Some(r#"{"value":7}"#), |_| TestState { value: 9 });
        assert_eq!(parsed.state, TestState { value: 7 });
        assert!(parsed.had_checkpoint);
        assert!(!parsed.was_legacy);
    }

    #[test]
    fn legacy_input_uses_the_provider_callback() {
        let parsed = parse_state(Some("legacy:7"), |raw| TestState {
            value: raw
                .strip_prefix("legacy:")
                .and_then(|value| value.parse().ok())
                .unwrap_or_default(),
        });
        assert_eq!(parsed.state, TestState { value: 7 });
        assert!(parsed.had_checkpoint);
        assert!(parsed.was_legacy);
    }

    #[test]
    fn invalid_legacy_input_keeps_the_legacy_marker() {
        let parsed = parse_state(Some("not a cursor"), |_| TestState::default());
        assert_eq!(parsed.state, TestState::default());
        assert!(parsed.had_checkpoint);
        assert!(parsed.was_legacy);
    }

    #[test]
    fn message_backfill_fields_take_priority_and_clamp_total() {
        let cursor = r#"{
            "backfill_remaining_messages": 3,
            "backfill_total_messages": 2,
            "backfill_next_seq": 99,
            "backfill_total_seq": 100
        }"#;
        assert_eq!(backfill_stats(cursor), Some((3, 3)));
        assert!(backfill_pending(cursor));
    }

    #[test]
    fn sequence_backfill_fields_are_the_fallback() {
        let cursor = r#"{"backfill_next_seq":4,"backfill_total_seq":3}"#;
        assert_eq!(backfill_stats(cursor), Some((4, 4)));
        assert!(backfill_pending(cursor));
    }

    /// New mail a bounded IMAP run left behind is pending work too: without it
    /// neither the fast reschedule nor a complete target rebuild comes back.
    #[test]
    fn deferred_incremental_messages_count_as_pending() {
        let cursor = r#"{"high_uid":40,"incremental_pending":7}"#;
        assert_eq!(backfill_stats(cursor), Some((7, 7)));
        assert!(backfill_pending(cursor));

        let both = r#"{"backfill_next_seq":4,"backfill_total_seq":10,"incremental_pending":3}"#;
        assert_eq!(backfill_stats(both), Some((7, 13)));
    }

    #[test]
    fn valid_json_without_backfill_is_zero_but_invalid_json_is_absent() {
        assert_eq!(backfill_stats(r#"{}"#), Some((0, 0)));
        assert!(!backfill_pending(r#"{}"#));
        assert_eq!(backfill_stats("not json"), None);
        assert!(!backfill_pending("not json"));
    }
}
