#![no_main]
use libfuzzer_sys::fuzz_target;
use satchel_core::email::calendar::{extract_ics_block, parse_invite};
fuzz_target!(|data: &[u8]| {
    let input = String::from_utf8_lossy(data);
    let _ = parse_invite(&input);
    if let Some(block) = extract_ics_block(Some(&input), Some(&input)) {
        let _ = parse_invite(&block);
    }
});
