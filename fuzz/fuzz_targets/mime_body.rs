#![no_main]
use libfuzzer_sys::fuzz_target;
use satchel_core::providers::imap::{parse_raw_message_body, parse_raw_rfc822};
fuzz_target!(|data: &[u8]| {
    let _ = parse_raw_message_body(data);
    let _ = parse_raw_rfc822(data);
});
