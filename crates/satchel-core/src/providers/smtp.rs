//! SMTP submission for the providers that cannot send on their own.
//!
//! This lived inside `imap.rs`, 3700 lines serving two unrelated protocols,
//! with a comment explaining its location by its own location. The From
//! header bug of 21092a3 sat here, and nothing tested "envelope and headers
//! for this message" as a unit because there was no unit to test.

use base64::Engine as _;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::extension::ClientId;
use lettre::{address::Envelope, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::traits::OutgoingMessage;

/// The SMTP server an IMAP or POP3 account submits through.
#[derive(Debug, Clone)]
pub struct SmtpRuntimeConfig {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
    pub from_address: String,
}

/// What the client calls itself when a sender domain cannot be derived.
///
/// A hardened submission server rejects a HELO that is not a FQDN, so the
/// fallback needs a dot. `.invalid` is reserved by RFC 2606 and therefore never
/// claims a name someone else owns.
const EHLO_FALLBACK_DOMAIN: &str = "satchel.invalid";

/// The name to announce in EHLO, derived from the sender's domain.
///
/// Without this lettre falls back to `[127.0.0.1]` — the `hostname` feature is
/// off, and the local machine name (`macbook.local`) would fail the same
/// checks anyway. The sender domain is the one name that is both a FQDN and
/// plausibly ours.
fn ehlo_name(from_address: &str) -> String {
    from_address
        .trim()
        .trim_end_matches('>')
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().trim_matches('.').to_ascii_lowercase())
        .filter(|domain| {
            domain.contains('.')
                && domain
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        })
        .unwrap_or_else(|| EHLO_FALLBACK_DOMAIN.to_string())
}

/// Submit a message over SMTP.
///
/// Shared by the IMAP and POP3 providers: neither protocol can send, so both
/// depend on an SMTP server configured alongside them.
pub async fn send_via_smtp(
    smtp: &SmtpRuntimeConfig,
    message: OutgoingMessage,
) -> crate::sync::errors::SyncResult<()> {
    send_via_smtp_returning_wire_copy(smtp, message)
        .await
        .map(|_| ())
}

/// What goes to the server: the envelope, and the exact bytes of the message.
///
/// Built without touching the network, so what a From header and an envelope
/// look like for a given message is a unit a test can hold still. The envelope
/// carries bare addresses — display names belong in the headers only.
#[derive(Debug, Clone)]
pub struct Submission {
    pub envelope: Envelope,
    pub bytes: Vec<u8>,
}

/// Build the envelope and wire form of an outgoing message.
/// The header mailbox; the envelope takes only its `email`.
fn named_mailbox(
    message: &OutgoingMessage,
    address: &str,
) -> Result<Mailbox, lettre::address::AddressError> {
    let mut mailbox: Mailbox = address.parse()?;
    if let Some(name) = message.recipient_names.get(&address.to_ascii_lowercase()) {
        mailbox.name = Some(name.clone());
    }
    Ok(mailbox)
}

pub fn build_submission(
    smtp: &SmtpRuntimeConfig,
    message: &OutgoingMessage,
) -> crate::sync::errors::SyncResult<Submission> {
    // Cc or Bcc alone is a complete envelope; only no recipient at all is not.
    if message.to.is_empty() && message.cc.is_empty() && message.bcc.is_empty() {
        return Err("missing recipient for SMTP send".into());
    }

    let from_address = message
        .from_address
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&smtp.from_address);
    let from = sender_mailbox(message.from_name.as_deref(), from_address)?;
    let envelope_from = from.email.clone();

    let mut builder = Message::builder()
        .from(from)
        .subject(message.subject.clone());
    if let Some(mid) = message
        .message_id_header
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        builder = builder.message_id(Some(mid.to_string()));
    }
    let mut envelope_recipients = Vec::new();
    for to in &message.to {
        let recipient = named_mailbox(message, to)
            .map_err(|err| format!("invalid recipient address `{to}`: {err}"))?;
        envelope_recipients.push(recipient.email.clone());
        builder = builder.to(recipient);
    }
    for cc in &message.cc {
        let recipient = named_mailbox(message, cc)
            .map_err(|err| format!("invalid cc address `{cc}`: {err}"))?;
        envelope_recipients.push(recipient.email.clone());
        builder = builder.cc(recipient);
    }
    for bcc in &message.bcc {
        let recipient = named_mailbox(message, bcc)
            .map_err(|err| format!("invalid bcc address `{bcc}`: {err}"))?;
        envelope_recipients.push(recipient.email.clone());
        builder = builder.bcc(recipient);
    }
    if let Some(reply_to) = message.reply_to.as_deref().filter(|s| !s.trim().is_empty()) {
        let mailbox: Mailbox = reply_to
            .parse()
            .map_err(|err| format!("invalid Reply-To address `{reply_to}`: {err}"))?;
        builder = builder.reply_to(mailbox);
    }
    if let Some(rr) = message
        .request_read_receipt_to
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        // lettre exposes user_defined headers via the message_builder; for read
        // receipts we inject a header manually using the public header API.
        use lettre::message::header::{Header, HeaderName, HeaderValue};
        #[derive(Clone)]
        struct RawHeader {
            name: HeaderName,
            value: String,
        }
        impl Header for RawHeader {
            fn name() -> HeaderName {
                HeaderName::new_from_ascii_str("Disposition-Notification-To")
            }
            fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                unimplemented!()
            }
            fn display(&self) -> HeaderValue {
                HeaderValue::dangerous_new_pre_encoded(
                    self.name.clone(),
                    self.value.clone(),
                    self.value.clone(),
                )
            }
        }
        builder = builder.header(RawHeader {
            name: HeaderName::new_from_ascii_str("Disposition-Notification-To"),
            value: format!("<{rr}>"),
        });
    }
    if let Some(in_reply_to) = &message.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.clone());
    }
    if !message.references.is_empty() {
        builder = builder.references(message.references.join(" "));
    }

    if let Some(raw) = message
        .raw_rfc822_override
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let envelope = Envelope::new(Some(envelope_from), envelope_recipients)
            .map_err(|err| format!("smtp envelope build failed: {err}"))?;
        return Ok(Submission {
            envelope,
            bytes: raw.as_bytes().to_vec(),
        });
    }
    let mail = build_lettre_body(builder, message)
        .map_err(|err| format!("smtp message build failed: {err}"))?;
    // lettre derives this envelope from the headers and drops the Bcc header
    // from the formatted bytes, keeping those addresses in the envelope only.
    Ok(Submission {
        envelope: mail.envelope().clone(),
        bytes: mail.formatted(),
    })
}

/// The one place an SMTP transport is built — for sending and for the
/// connection test alike, so the two cannot drift apart on TLS or EHLO.
pub(crate) fn smtp_transport(
    smtp: &SmtpRuntimeConfig,
    hello_from: &str,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let creds = Credentials::new(smtp.username.clone(), smtp.password.clone());
    let hello = ClientId::Domain(ehlo_name(hello_from));
    Ok(if smtp.tls {
        // Port 465 speaks implicit TLS from the first byte; issuing STARTTLS
        // there stalls until timeout. Everything else (587/25) uses STARTTLS.
        let builder = if smtp.port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
        }
        .map_err(|err| format!("smtp relay setup failed: {err}"))?;
        builder
            .port(smtp.port)
            .hello_name(hello)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host)
            .port(smtp.port)
            .hello_name(hello)
            .credentials(creds)
            .build()
    })
}

/// Submit a message over SMTP and return the bytes handed to the transport.
///
/// Callers that file a Sent copy use those bytes verbatim: rebuilding the
/// message would file a second, different message, and for a crypto send it
/// would file the plaintext the wire form deliberately no longer carries.
pub async fn send_via_smtp_returning_wire_copy(
    smtp: &SmtpRuntimeConfig,
    message: OutgoingMessage,
) -> crate::sync::errors::SyncResult<Vec<u8>> {
    let submission = build_submission(smtp, &message)?;
    let hello_from = message
        .from_address
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&smtp.from_address);
    let transport = smtp_transport(smtp, hello_from)?;
    // `AsyncTransport::send` is `send_raw(message.envelope(), &formatted)`;
    // calling it directly makes the bytes returned the bytes sent.
    transport
        .send_raw(&submission.envelope, &submission.bytes)
        .await
        .map_err(|err| crate::sync::errors::smtp_submission_failure("smtp send failed", &err))?;
    Ok(submission.bytes)
}

/// Parses only the address and attaches the display name verbatim. Feeding
/// `Name <addr>` through lettre's parser rejects names with dots, commas,
/// parentheses or non-ASCII, which real account names routinely contain.
fn sender_mailbox(name: Option<&str>, address: &str) -> Result<Mailbox, String> {
    let email: lettre::Address = address
        .trim()
        .parse()
        .map_err(|err| format!("invalid From address `{address}`: {err}"))?;
    let name = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(Mailbox::new(name, email))
}

fn build_lettre_body(
    builder: lettre::message::MessageBuilder,
    message: &OutgoingMessage,
) -> Result<Message, lettre::error::Error> {
    let body_text = message.body.clone();
    let body_html = message.body_html.clone().filter(|s| !s.trim().is_empty());

    if message.attachments.is_empty() {
        return match body_html {
            Some(html) => builder.multipart(MultiPart::alternative_plain_html(body_text, html)),
            None => builder.body(body_text),
        };
    }

    let alt = match body_html {
        Some(html) => MultiPart::alternative_plain_html(body_text, html),
        None => MultiPart::alternative().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(body_text),
        ),
    };

    fn single_part(
        att: &crate::providers::traits::OutgoingAttachment,
    ) -> Result<SinglePart, lettre::error::Error> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(att.content_base64.as_bytes())
            .map_err(|_| lettre::error::Error::CannotParseFilename)?;
        let mime_type: ContentType = att
            .mime_type
            .parse()
            .unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap());
        Ok(if att.inline {
            let cid = att
                .content_id
                .clone()
                .unwrap_or_else(|| att.filename.clone());
            Attachment::new_inline(cid).body(bytes, mime_type)
        } else {
            Attachment::new(att.filename.clone()).body(bytes, mime_type)
        })
    }

    let (inline, attached): (Vec<_>, Vec<_>) =
        message.attachments.iter().partition(|att| att.inline);

    // RFC 2387: `cid:` only resolves inside the multipart/related holding both
    // the HTML and the part it names. Mirrors `email::build::build_body_section`
    // — the two builders have to produce the same structure or a mail looks
    // different depending on which account sent it.
    if inline.is_empty() {
        let mut mixed = MultiPart::mixed().multipart(alt);
        for att in attached {
            mixed = mixed.singlepart(single_part(att)?);
        }
        return builder.multipart(mixed);
    }

    let mut related = MultiPart::related().multipart(alt);
    for att in inline {
        related = related.singlepart(single_part(att)?);
    }
    if attached.is_empty() {
        return builder.multipart(related);
    }

    let mut mixed = MultiPart::mixed().multipart(related);
    for att in attached {
        mixed = mixed.singlepart(single_part(att)?);
    }
    builder.multipart(mixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smtp_test_message() -> (Message, OutgoingMessage) {
        let outgoing = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            bcc: vec!["chef@example.test".to_owned()],
            subject: "quarterly figures".to_owned(),
            body: "plain text".to_owned(),
            body_html: Some("<p>plain text</p>".to_owned()),
            message_id_header: Some("<stable@satchel.local>".to_owned()),
            ..Default::default()
        };
        let builder = Message::builder()
            .from(sender_mailbox(Some("Me"), "me@example.test").expect("from"))
            .to("her@example.test".parse().expect("to"))
            .bcc("chef@example.test".parse().expect("bcc"))
            .message_id(Some("<stable@satchel.local>".to_owned()))
            .subject(outgoing.subject.clone());
        (
            build_lettre_body(builder, &outgoing).expect("lettre body"),
            outgoing,
        )
    }

    #[test]
    fn the_smtp_wire_copy_is_the_message_the_transport_formats() {
        // `AsyncTransport::send` formats the very same `Message`, so taking the
        // Sent copy from here keeps Date, Message-ID and boundaries identical.
        let (mail, _) = smtp_test_message();
        let wire_copy = mail.formatted();

        assert_eq!(wire_copy, mail.formatted());
        let text = String::from_utf8(wire_copy).expect("utf-8 wire copy");
        assert!(text.contains("Message-ID: <stable@satchel.local>"));
        assert_eq!(text.matches("Date: ").count(), 1);
    }

    #[test]
    fn smtp_keeps_bcc_in_the_envelope_and_out_of_the_wire_headers() {
        let (mail, outgoing) = smtp_test_message();
        let text = String::from_utf8(mail.formatted()).expect("utf-8 wire copy");

        assert!(!text.to_ascii_lowercase().contains("bcc:"));
        assert!(
            !text.contains("chef@example.test"),
            "a Bcc header in SMTP DATA would show the list to every To/Cc recipient"
        );
        assert_eq!(outgoing.bcc, vec!["chef@example.test".to_owned()]);
        assert!(mail
            .envelope()
            .to()
            .iter()
            .any(|address| address.to_string() == "chef@example.test"));
    }

    fn lettre_structure(attachments: Vec<crate::providers::traits::OutgoingAttachment>) -> String {
        let outgoing = OutgoingMessage {
            to: vec!["her@example.test".to_owned()],
            subject: "Angebot".to_owned(),
            body: "Guten Tag".to_owned(),
            body_html: Some("<p>Guten Tag <img src=\"cid:logo\"></p>".to_owned()),
            attachments,
            ..Default::default()
        };
        let builder = Message::builder()
            .from(sender_mailbox(Some("Me"), "me@example.test").expect("from"))
            .to("her@example.test".parse().expect("to"))
            .subject(outgoing.subject.clone());
        String::from_utf8(
            build_lettre_body(builder, &outgoing)
                .expect("lettre body")
                .formatted(),
        )
        .expect("utf-8")
    }

    fn lettre_attachment(
        filename: &str,
        inline: bool,
    ) -> crate::providers::traits::OutgoingAttachment {
        crate::providers::traits::OutgoingAttachment {
            filename: filename.to_owned(),
            mime_type: if inline {
                "image/png"
            } else {
                "application/pdf"
            }
            .to_owned(),
            content_base64: base64::engine::general_purpose::STANDARD.encode(b"bytes"),
            content_id: inline.then(|| "logo".to_owned()),
            inline,
        }
    }

    /// Structure has to match `email::build::build_body_section`, or the same
    /// message looks different depending on which account sent it.
    #[test]
    fn the_lettre_builder_nests_inline_parts_in_multipart_related() {
        let only_inline = lettre_structure(vec![lettre_attachment("logo.png", true)]);
        assert!(only_inline.contains("multipart/related"), "{only_inline}");
        assert!(only_inline.contains("Content-ID: <logo>"));
        assert!(!only_inline.contains("multipart/mixed"));

        let both = lettre_structure(vec![
            lettre_attachment("logo.png", true),
            lettre_attachment("Angebot.pdf", false),
        ]);
        let mixed_at = both.find("multipart/mixed").expect("a mixed layer");
        let related_at = both.find("multipart/related").expect("a related layer");
        let alternative_at = both
            .find("multipart/alternative")
            .expect("an alternative layer");
        assert!(
            mixed_at < related_at && related_at < alternative_at,
            "{both}"
        );

        let no_inline = lettre_structure(vec![lettre_attachment("Angebot.pdf", false)]);
        assert!(!no_inline.contains("multipart/related"), "{no_inline}");
        assert!(no_inline.contains("multipart/mixed"));
    }

    fn smtp_config() -> SmtpRuntimeConfig {
        SmtpRuntimeConfig {
            host: "smtp.example.test".into(),
            port: 587,
            tls: true,
            username: "anna@example.com".into(),
            password: "pw".into(),
            from_address: "anna@example.com".into(),
        }
    }

    /// 21092a3 as a unit: a display name with a dot and a comma used to fail
    /// before SMTP was contacted. No transport involved here.
    #[test]
    fn builds_the_from_header_with_a_dotted_display_name() {
        let message = OutgoingMessage {
            from_name: Some("Dr. Meier, Anna".into()),
            from_address: Some("anna@example.com".into()),
            to: vec!["mara@example.test".into()],
            subject: "Status".into(),
            body: "Hallo".into(),
            ..Default::default()
        };

        let submission = build_submission(&smtp_config(), &message).expect("submission");

        let text = String::from_utf8(submission.bytes).expect("utf-8");
        let from = text
            .lines()
            .find_map(|line| line.strip_prefix("From: "))
            .expect("a From header");
        // A comma is a special in a phrase, so lettre sends the name as an
        // RFC 2047 encoded word. What matters is that it arrives whole.
        assert!(from.ends_with(" <anna@example.com>"), "{from}");
        assert_eq!(
            crate::utils::mime::decode_mime_encoded_words(from),
            "Dr. Meier, Anna <anna@example.com>"
        );
    }

    /// The envelope carries addresses, never names — `MAIL FROM:<Dr. Meier…>`
    /// is a syntax error at the server.
    #[test]
    fn the_envelope_uses_bare_addresses() {
        let message = OutgoingMessage {
            from_name: Some("Dr. Meier, Anna".into()),
            from_address: Some("anna@example.com".into()),
            to: vec!["Mara Example <mara@example.test>".into()],
            bcc: vec!["chef@example.test".into()],
            subject: "Status".into(),
            body: "Hallo".into(),
            ..Default::default()
        };

        let submission = build_submission(&smtp_config(), &message).expect("submission");

        assert_eq!(
            submission
                .envelope
                .from()
                .map(ToString::to_string)
                .as_deref(),
            Some("anna@example.com")
        );
        let recipients: Vec<String> = submission
            .envelope
            .to()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(recipients, ["mara@example.test", "chef@example.test"]);
    }

    #[test]
    fn recipient_names_reach_the_header_but_not_the_envelope() {
        let mut message = OutgoingMessage {
            from_address: Some("anna@example.com".into()),
            to: vec!["mara@example.test".into()],
            subject: "Status".into(),
            body: "Hallo".into(),
            ..Default::default()
        };
        message
            .recipient_names
            .insert("mara@example.test".into(), "Mara Müller".into());

        let submission = build_submission(&smtp_config(), &message).expect("submission");

        let text = String::from_utf8(submission.bytes).expect("utf-8");
        let to = text
            .lines()
            .find_map(|line| line.strip_prefix("To: "))
            .expect("a To header");
        assert!(to.is_ascii(), "{to}");
        assert_eq!(
            crate::utils::mime::decode_mime_encoded_words(to),
            "Mara Müller <mara@example.test>"
        );
        let envelope: Vec<String> = submission
            .envelope
            .to()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(envelope, ["mara@example.test"]);
    }

    #[test]
    fn a_message_with_only_cc_or_bcc_recipients_still_has_an_envelope() {
        let cc_only = OutgoingMessage {
            from_address: Some("anna@example.com".into()),
            cc: vec!["copy@example.test".into()],
            subject: "Status".into(),
            body: "Hallo".into(),
            ..Default::default()
        };
        let submission = build_submission(&smtp_config(), &cc_only).expect("cc-only submission");
        let envelope: Vec<String> = submission
            .envelope
            .to()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(envelope, ["copy@example.test"]);

        let bcc_only = OutgoingMessage {
            cc: Vec::new(),
            bcc: vec!["hidden@example.test".into()],
            ..cc_only
        };
        let submission = build_submission(&smtp_config(), &bcc_only).expect("bcc-only submission");
        let text = String::from_utf8(submission.bytes).expect("utf-8");
        assert!(!text.contains("hidden@example.test"), "{text}");
        let envelope: Vec<String> = submission
            .envelope
            .to()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(envelope, ["hidden@example.test"]);
    }

    #[test]
    fn a_message_without_any_recipient_is_refused() {
        let message = OutgoingMessage {
            from_address: Some("anna@example.com".into()),
            subject: "Status".into(),
            body: "Hallo".into(),
            ..Default::default()
        };
        assert!(build_submission(&smtp_config(), &message).is_err());
    }

    #[test]
    fn builds_a_multipart_body_with_an_attachment_without_a_network() {
        let message = OutgoingMessage {
            from_address: Some("anna@example.com".into()),
            to: vec!["mara@example.test".into()],
            subject: "Angebot".into(),
            body: "Anbei".into(),
            attachments: vec![lettre_attachment("Angebot.pdf", false)],
            ..Default::default()
        };

        let submission = build_submission(&smtp_config(), &message).expect("submission");

        let text = String::from_utf8(submission.bytes).expect("utf-8");
        assert!(text.contains("multipart/mixed"), "{text}");
        assert!(text.contains("filename=\"Angebot.pdf\""), "{text}");
    }

    #[test]
    fn sender_mailbox_keeps_names_lettre_cannot_parse() {
        for name in [
            "Dr. Alex Morgan",
            "Morgan, Alex",
            "Alex Mörgan",
            "Alex (Privat)",
        ] {
            let mailbox = sender_mailbox(Some(name), "alex@example.com").expect(name);
            assert_eq!(mailbox.name.as_deref(), Some(name));
            assert_eq!(mailbox.email.to_string(), "alex@example.com");
        }
    }

    #[test]
    fn sender_mailbox_drops_blank_names_and_rejects_bad_addresses() {
        let mailbox = sender_mailbox(Some("  "), " alex@example.com ").unwrap();
        assert_eq!(mailbox.name, None);
        assert_eq!(mailbox.to_string(), "alex@example.com");
        let err = sender_mailbox(Some("Alex"), "not-an-address").unwrap_err();
        assert!(
            err.starts_with("invalid From address `not-an-address`"),
            "{err}"
        );
    }

    #[test]
    fn the_ehlo_name_comes_from_the_sender_domain() {
        assert_eq!(ehlo_name("user@firma.de"), "firma.de");
        assert_eq!(ehlo_name("  User@Firma.DE  "), "firma.de");
        assert_eq!(ehlo_name("user@mail.firma.co.uk"), "mail.firma.co.uk");
    }

    #[test]
    fn an_underivable_sender_still_yields_a_fully_qualified_ehlo_name() {
        for address in [
            "",
            "   ",
            "postmaster",
            "user@localhost",
            "user@[192.0.2.1]",
        ] {
            let name = ehlo_name(address);
            assert_eq!(name, super::EHLO_FALLBACK_DOMAIN, "for {address:?}");
            assert!(
                name.contains('.') && !name.starts_with('['),
                "a hardened submission server rejects anything but a FQDN: {name:?}"
            );
        }
    }
}
