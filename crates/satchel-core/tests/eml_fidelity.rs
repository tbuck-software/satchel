use satchel_core::providers::imap::parse_raw_rfc822;

#[test]
fn quoted_display_names_do_not_become_part_of_recipient_addresses() {
    let raw = b"From: sender@example.test\r\n\
To: \"Torben Buck <mail@example.test>\" <mail@example.test>, \"Doe, Jane\" <jane@example.test>\r\n\
Cc: \"Other <old@example.test>\" <other@example.test>\r\n\
Subject: Address fixture\r\n\r\nHello";
    let parsed = parse_raw_rfc822(raw).unwrap();
    assert_eq!(
        parsed.to_addresses,
        ["mail@example.test", "jane@example.test"]
    );
    assert_eq!(parsed.cc_addresses, ["other@example.test"]);
}

#[test]
fn escaped_quotes_keep_address_shaped_display_names_out_of_recipients() {
    let raw = b"To: \"Buck, \\\"Torben <old@example.test>\\\"\" <mail@example.test>\r\n\r\nHello";
    let parsed = parse_raw_rfc822(raw).unwrap();
    assert_eq!(parsed.to_addresses, ["mail@example.test"]);
}

#[test]
fn an_explicit_image_attachment_stays_downloadable_when_it_has_a_content_id() {
    let raw = b"From: sender@example.test\r\n\
Content-Type: multipart/mixed; boundary=parts\r\n\r\n\
--parts\r\nContent-Type: text/plain\r\n\r\nPhoto attached.\r\n\
--parts\r\nContent-Type: image/jpeg\r\n\
Content-Disposition: attachment; filename=photo.jpg\r\n\
Content-ID: <photo@example.test>\r\n\
Content-Transfer-Encoding: base64\r\n\r\nAQIDBA==\r\n\
--parts--\r\n";
    let parsed = parse_raw_rfc822(raw).unwrap();
    let attachment = &parsed.attachments[0];
    assert_eq!(attachment.bytes, [1, 2, 3, 4]);
    assert_eq!(attachment.content_id.as_deref(), Some("photo@example.test"));
    assert!(!attachment.is_inline);
}
