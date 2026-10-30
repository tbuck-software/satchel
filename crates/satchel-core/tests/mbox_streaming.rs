use satchel_core::email::mbox::{write_message, MboxEntries};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
struct Allocations {
    largest: usize,
    total: usize,
}

thread_local! {
    // Tokio's default test runtime stays on this thread. Other tests need no lock.
    static ALLOCATIONS: Cell<Option<Allocations>> = const { Cell::new(None) };
}

struct TrackingAllocator;

fn record_allocation(size: usize) {
    ALLOCATIONS.with(|allocations| {
        if let Some(mut stats) = allocations.get() {
            stats.largest = stats.largest.max(size);
            stats.total += size;
            allocations.set(Some(stats));
        }
    });
}

// SAFETY: All allocations and deallocations are forwarded unchanged to System.
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation(size);
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

struct AllocationProbe;

impl AllocationProbe {
    fn start() -> Self {
        ALLOCATIONS.with(|stats| assert!(stats.replace(Some(Allocations::default())).is_none()));
        Self
    }

    fn finish(self) -> Allocations {
        ALLOCATIONS.with(|stats| stats.take().unwrap())
    }
}

impl Drop for AllocationProbe {
    fn drop(&mut self) {
        ALLOCATIONS.with(|stats| stats.set(None));
    }
}

const CEILING: usize = 1024;
const ENVELOPE: &[u8] = b"From sender@example.test Thu Jan  1 00:00:00 2026\r\n";
const RECOVERED: &[u8] = b"Subject: Recovered\r\n\r\nok";

async fn assert_bounded_recovery(raw: &[u8]) {
    // Allocate the reader and fixture outside the measurement. The measurement
    // includes every parser allocation, not just the size of returned messages.
    for capacity in [3, 127, 8192, raw.len()] {
        let reader = tokio::io::BufReader::with_capacity(capacity, raw);
        let mut entries = MboxEntries::with_ceiling(reader, CEILING);
        let probe = AllocationProbe::start();
        let recovered = entries.next_message_bytes().await.unwrap().unwrap();
        assert!(entries.next_message_bytes().await.unwrap().is_none());
        let stats = probe.finish();
        assert_eq!(recovered, RECOVERED);
        assert_eq!(entries.dropped, 1);
        assert_eq!(entries.bytes_read, raw.len() as u64);
        assert!(
            stats.largest <= 2 * CEILING,
            "parser allocated {} bytes at once with a {CEILING}-byte ceiling",
            stats.largest
        );
        assert!(
            stats.total <= 8 * CEILING,
            "parser allocated {} bytes cumulatively while discarding an oversized entry",
            stats.total
        );
    }
}

#[tokio::test]
async fn oversized_single_lines_use_bounded_allocations_and_recover() {
    for length in [128 * 1024, 2 * 1024 * 1024] {
        let mut raw = ENVELOPE.to_vec();
        raw.extend_from_slice(b"Subject: Too big\r\n\r\n");
        raw.resize(raw.len() + length, b'x');
        raw.extend_from_slice(b"\r\n");
        raw.extend_from_slice(ENVELOPE);
        raw.extend_from_slice(RECOVERED);
        assert_bounded_recovery(&raw).await;
    }
}

#[tokio::test]
async fn oversized_continued_bodies_do_not_allocate_per_line() {
    let mut raw = ENVELOPE.to_vec();
    raw.extend_from_slice(b"Subject: Too big\n\n");
    for _ in 0..32 * 1024 {
        // Includes quoted From lines, which must never cause recovery.
        raw.extend_from_slice(b">From quoted\r\nsmall body line\n");
    }
    // A huge line after the entry is already oversized must also be discarded.
    raw.resize(raw.len() + 2 * 1024 * 1024, b'x');
    raw.extend_from_slice(b"\n");
    raw.extend_from_slice(ENVELOPE);
    raw.extend_from_slice(RECOVERED);
    assert_bounded_recovery(&raw).await;
}

#[tokio::test]
async fn oversized_unterminated_lines_are_discarded_at_eof() {
    for prefix in [ENVELOPE, b"Subject: Bare EML\r\n\r\n".as_slice()] {
        let mut raw = prefix.to_vec();
        raw.resize(raw.len() + 2 * 1024 * 1024, b'x');
        let reader = tokio::io::BufReader::with_capacity(31, raw.as_slice());
        let mut entries = MboxEntries::with_ceiling(reader, CEILING);
        let probe = AllocationProbe::start();
        assert!(entries.next_entry().await.unwrap().is_none());
        let stats = probe.finish();
        assert_eq!(entries.dropped, 1);
        assert_eq!(entries.bytes_read, raw.len() as u64);
        assert!(stats.largest <= 2 * CEILING);
        assert!(stats.total <= 8 * CEILING);
    }
}

#[tokio::test]
async fn fragmented_envelopes_crlf_and_quoting_round_trip() {
    let messages = [
        b"Subject: Original\r\n\r\nFrom literal\r\n>From quoted\n>>From twice\n\xff\0\r\n"
            .as_slice(),
        b"Subject: Last\n\nNo final newline".as_slice(),
    ];
    let mut raw = b"preamble\r\n".to_vec();
    for message in messages {
        raw.extend_from_slice(ENVELOPE);
        write_message(&mut raw, message).await.unwrap();
    }
    for capacity in 1..=32 {
        let reader = tokio::io::BufReader::with_capacity(capacity, raw.as_slice());
        let mut entries = MboxEntries::new(reader);
        for expected in messages {
            assert_eq!(
                entries.next_message_bytes().await.unwrap().unwrap(),
                expected
            );
        }
        assert!(entries.next_message_bytes().await.unwrap().is_none());
        assert_eq!(entries.dropped, 0);
        assert_eq!(entries.bytes_read, raw.len() as u64);
    }
}

#[tokio::test]
async fn fragmented_bare_eml_header_detection_preserves_literal_from_lines() {
    for header in ["Subject", "X", "X-Very-Long-Header"] {
        let raw = format!("{header}: Bare\r\n\r\nFrom literal\r\n>From quoted\r\n");
        for capacity in 1..=8 {
            let reader = tokio::io::BufReader::with_capacity(capacity, raw.as_bytes());
            let mut entries = MboxEntries::new(reader);
            assert_eq!(
                entries.next_message_bytes().await.unwrap().unwrap(),
                raw.as_bytes()
            );
            assert!(entries.next_message_bytes().await.unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn message_ceiling_is_inclusive_with_fragmented_input() {
    let mut raw = ENVELOPE.to_vec();
    raw.extend_from_slice(b"Subject: At limit\n\n");
    raw.resize(CEILING, b'x');
    for capacity in [1, 7, CEILING * 2] {
        for ceiling in [CEILING - 1, CEILING] {
            let reader = tokio::io::BufReader::with_capacity(capacity, raw.as_slice());
            let mut entries = MboxEntries::with_ceiling(reader, ceiling);
            let entry = entries.next_entry().await.unwrap();
            assert_eq!(entry.is_some(), ceiling == CEILING);
            assert_eq!(entries.dropped, usize::from(ceiling < CEILING));
            assert!(entries.next_entry().await.unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn an_oversized_first_header_still_identifies_bare_eml() {
    let mut raw = vec![b'X'; 2 * 1024 * 1024];
    raw.extend_from_slice(b": Bare\r\n\r\n");
    raw.extend_from_slice(ENVELOPE);
    raw.extend_from_slice(RECOVERED);
    let reader = tokio::io::BufReader::with_capacity(7, raw.as_slice());
    let mut entries = MboxEntries::with_ceiling(reader, CEILING);
    let probe = AllocationProbe::start();
    assert!(entries.next_message_bytes().await.unwrap().is_none());
    let stats = probe.finish();
    assert_eq!(entries.dropped, 1);
    assert_eq!(entries.bytes_read, raw.len() as u64);
    assert!(stats.largest <= 2 * CEILING);
    assert!(stats.total <= 8 * CEILING);
}

#[tokio::test]
async fn oversized_preamble_is_discarded_without_counting_a_dropped_message() {
    let mut raw = vec![b'x'; 2 * 1024 * 1024];
    raw.extend_from_slice(b"\r\n");
    raw.extend_from_slice(ENVELOPE);
    raw.extend_from_slice(RECOVERED);
    let reader = tokio::io::BufReader::with_capacity(3, raw.as_slice());
    let mut entries = MboxEntries::with_ceiling(reader, CEILING);
    let probe = AllocationProbe::start();
    let recovered = entries.next_message_bytes().await.unwrap().unwrap();
    assert!(entries.next_message_bytes().await.unwrap().is_none());
    let stats = probe.finish();
    assert_eq!(recovered, RECOVERED);
    assert_eq!(entries.dropped, 0);
    assert_eq!(entries.bytes_read, raw.len() as u64);
    assert!(stats.largest <= 2 * CEILING);
    assert!(stats.total <= 8 * CEILING);
}
