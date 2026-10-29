use satchel_core::email::mbox::{write_message, MboxEntries};

#[tokio::test]
async fn mbox_round_trip_keeps_binary_octets_from_lines_and_trailing_newlines() {
    let mut mbox = Vec::new();
    let messages = [
        b"Subject: Original\r\n\r\nFrom sender\r\n>From quoted\r\n>>From twice\r\n\xff\0\r\n\r\n"
            .as_slice(),
        b"Subject: No terminal newline\n\nFrom last".as_slice(),
        b"Subject: Empty body\r\n\r\n".as_slice(),
    ];
    for raw in messages {
        mbox.extend_from_slice(b"From sender@example.test Thu Jan  1 00:00:00 2026\n");
        write_message(&mut mbox, raw).await.unwrap();
    }
    let mut entries = MboxEntries::new(tokio::io::BufReader::new(mbox.as_slice()));
    for expected in messages {
        assert_eq!(
            entries.next_message_bytes().await.unwrap().unwrap(),
            expected
        );
    }
    assert!(entries.next_message_bytes().await.unwrap().is_none());
}

#[tokio::test]
async fn a_bare_crlf_eml_is_not_unquoted_or_trimmed() {
    let raw = b"Subject: Original\r\n\r\nFrom literal\r\n>From quoted\r\n\xff\r\n\r\n";
    let mut entries = MboxEntries::new(tokio::io::BufReader::new(raw.as_slice()));
    assert_eq!(entries.next_message_bytes().await.unwrap().unwrap(), raw);
}
