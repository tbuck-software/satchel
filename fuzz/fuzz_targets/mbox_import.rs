#![no_main]
use libfuzzer_sys::fuzz_target;
use satchel_core::{
    email::mbox::{strip_mbox_from_line, MboxEntries},
    providers::imap::parse_raw_rfc822,
};
fuzz_target!(|data: &[u8]| {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut entries = MboxEntries::with_ceiling(data, 64 * 1024);
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let _ = parse_raw_rfc822(strip_mbox_from_line(&entry).as_bytes());
        }
        assert_eq!(entries.bytes_read, data.len() as u64);
    });
});
