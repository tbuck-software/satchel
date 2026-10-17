//! A large, deeply nested message has to parse in bounded time.
//!
//! The boundary search used to walk byte by byte and restart at every
//! near-match, and the body pass decoded every inline image only to drop it.

use std::time::{Duration, Instant};

use base64::Engine;
use satchel_core::providers::imap::parse_message_body;

/// Three levels, a 70-byte boundary per level, about 25 MB of attachments.
fn large_message() -> (String, Vec<u8>) {
    let boundary = |level: usize| format!("{:=<70}", format!("=_satchel_level{level}_"));
    let (outer, middle, inner) = (boundary(1), boundary(2), boundary(3));
    let attachment = base64::engine::general_purpose::STANDARD
        .encode(vec![0xA5u8; 6 * 1024 * 1024])
        .as_bytes()
        .chunks(76)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n");

    let mut body = String::new();
    body.push_str(&format!(
        "--{outer}\r\nContent-Type: multipart/related; boundary=\"{middle}\"\r\n\r\n"
    ));
    body.push_str(&format!(
        "--{middle}\r\nContent-Type: multipart/alternative; boundary=\"{inner}\"\r\n\r\n"
    ));
    body.push_str(&format!(
        "--{inner}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHallo Welt\r\n"
    ));
    body.push_str(&format!(
        "--{inner}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hallo Welt</p>\r\n"
    ));
    body.push_str(&format!("--{inner}--\r\n"));
    for index in 0..2 {
        body.push_str(&format!(
            "--{middle}\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <img{index}>\r\nContent-Disposition: inline; filename=\"img{index}.png\"\r\n\r\n{attachment}\r\n"
        ));
    }
    body.push_str(&format!("--{middle}--\r\n"));
    for index in 0..2 {
        body.push_str(&format!(
            "--{outer}\r\nContent-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"file{index}.pdf\"\r\n\r\n{attachment}\r\n"
        ));
    }
    body.push_str(&format!("--{outer}--\r\n"));
    let header = format!("Content-Type: multipart/mixed; boundary=\"{outer}\"\r\n");
    (header, body.into_bytes())
}

#[test]
fn a_25_mb_message_with_three_levels_parses_in_bounded_time() {
    let (header, body) = large_message();
    assert!(body.len() > 25 * 1024 * 1024, "{} bytes", body.len());

    let started = Instant::now();
    let parsed = parse_message_body(&header, Some(&body));
    let elapsed = started.elapsed();

    assert_eq!(parsed.text.as_deref().map(str::trim), Some("Hallo Welt"));
    assert_eq!(parsed.attachments.len(), 4);
    // About 0.9 s unoptimised and 50 ms optimised on an M-series laptop; the
    // ceilings leave room for a shared CI runner.
    let ceiling = if cfg!(debug_assertions) {
        Duration::from_secs(4)
    } else {
        Duration::from_millis(250)
    };
    eprintln!("parsed {} MB in {elapsed:?}", body.len() / (1024 * 1024));
    assert!(elapsed < ceiling, "took {elapsed:?}");
}
