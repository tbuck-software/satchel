use satchel_lib::commands::signatures::{
    detect_signature_candidates, extract_signature_html, resolve_signature_for_compose,
    ResolveSignatureInput, SignatureDetectionMessage, SignaturePreference, SignatureVariant,
};
use satchel_lib::email::build::build_rfc822;
use satchel_lib::providers::traits::OutgoingMessage;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory db");
    sqlx::migrate!("../crates/satchel-core/src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_signature(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO accounts
         (id, display_name, email, provider, config, credentials_ref, color, is_active, created_at, updated_at)
         VALUES ('acc-1', 'Max', 'max@example.com', 'imap', '{}', '', '#000', 1, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_aliases
         (id, account_id, email, display_name, is_default, created_at, updated_at)
         VALUES ('alias-1', 'acc-1', 'consulting@example.com', 'Consulting', 0, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO folders
         (id, account_id, name, full_path, folder_type, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
         VALUES ('sent-1', 'acc-1', 'Sent', 'Sent', 'sent', 0, 0, 1, 0, '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO signatures
         (id, account_id, name, body_text, body_html, short_body_text, short_body_html, is_default, created_at, updated_at)
         VALUES ('sig-1', 'acc-1', 'Arbeit', 'Freundliche Grüße
Max Mustermann',
                 '<p>Freundliche Grüße<br>Max Mustermann</p><img src="cid:brand-logo"><script>bad()</script>',
                 'Viele Grüße
Max', '<p>Viele Grüße<br>Max</p>', 1, '2026-01-01', '2026-01-01')"#,
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO signature_assignments
         (account_id, identity_email, signature_id, first_contact_mode, created_at, updated_at)
         VALUES ('acc-1', 'consulting@example.com', 'sig-1', 'full_then_short', '2026-01-01', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO signature_assets
         (id, signature_id, filename, content_type, content_id, content, created_at)
         VALUES ('asset-1', 'sig-1', 'logo.png', 'image/png', 'brand-logo', X'89504E47', '2026-01-01')",
    )
    .execute(pool)
    .await
    .unwrap();
}

fn automatic(thread_id: Option<&str>) -> ResolveSignatureInput {
    ResolveSignatureInput {
        account_id: "acc-1".into(),
        identity_email: "consulting@example.com".into(),
        thread_id: thread_id.map(str::to_string),
        signature_id: None,
        preference: SignaturePreference::Auto,
    }
}

#[tokio::test]
async fn automatic_variant_becomes_short_only_after_this_identity_sent_in_the_same_thread() {
    let pool = pool().await;
    seed_signature(&pool).await;

    let new_message = resolve_signature_for_compose(&pool, automatic(None))
        .await
        .unwrap()
        .expect("assigned signature");
    assert_eq!(new_message.variant, SignatureVariant::Long);

    let first = resolve_signature_for_compose(&pool, automatic(Some("thread-a")))
        .await
        .unwrap()
        .expect("assigned signature");
    assert_eq!(first.variant, SignatureVariant::Long);
    assert_eq!(first.body_text, "Freundliche Grüße\nMax Mustermann");
    assert_eq!(first.attachments.len(), 1);
    assert_eq!(
        first.attachments[0].content_id.as_deref(),
        Some("brand-logo")
    );
    assert!(first.attachments[0].inline);
    let first_html = first.body_html.expect("long html");
    assert!(first_html.contains("cid:brand-logo"));
    assert!(!first_html.contains("<script"));
    let rfc822 = build_rfc822(
        &OutgoingMessage {
            from_address: Some("consulting@example.com".into()),
            to: vec!["client@example.com".into()],
            subject: "Status".into(),
            body: format!("Hallo\n\n-- \n{}", first.body_text),
            body_html: Some(format!("<p>Hallo</p><div>{first_html}</div>")),
            attachments: first.attachments.clone(),
            ..OutgoingMessage::default()
        },
        Some("max@example.com"),
        Some("Max"),
    )
    .expect("signature image is valid base64");
    assert!(rfc822.contains("Content-Disposition: inline"));
    assert!(rfc822.contains("Content-ID: <brand-logo>"));
    assert!(rfc822.contains("cid:brand-logo"));
    // RFC 2387: the logo must sit in the same multipart/related as the HTML
    // that names it, or Outlook shows it a second time in the attachment bar.
    let related_at = rfc822
        .find("multipart/related")
        .expect("a multipart/related layer around the inline logo");
    let alternative_at = rfc822
        .find("multipart/alternative")
        .expect("a multipart/alternative layer");
    assert!(related_at < alternative_at, "{rfc822}");
    assert!(
        !rfc822.contains("multipart/mixed"),
        "no real attachment here"
    );

    sqlx::query(
        r#"INSERT INTO messages
         (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
          body_text, is_read, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES ('sent-primary', 'acc-1', 'sent-1', 'thread-a', 'max@example.com',
                 '["client@example.com"]', 'Status', '2026-01-02', 'Hello', 1, 0, 1, 0,
                 '2026-01-02', '2026-01-02')"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let still_first_for_alias = resolve_signature_for_compose(&pool, automatic(Some("thread-a")))
        .await
        .unwrap()
        .expect("assigned signature");
    assert_eq!(still_first_for_alias.variant, SignatureVariant::Long);

    sqlx::query(
        r#"INSERT INTO messages
         (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
          body_text, is_read, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES ('sent-alias', 'acc-1', 'sent-1', 'thread-a', 'consulting@example.com',
                 '["client@example.com"]', 'Status', '2026-01-03', 'Hello', 1, 0, 1, 0,
                 '2026-01-03', '2026-01-03')"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let follow_up = resolve_signature_for_compose(&pool, automatic(Some("thread-a")))
        .await
        .unwrap()
        .expect("assigned signature");
    assert_eq!(follow_up.variant, SignatureVariant::Short);
    assert_eq!(follow_up.body_text, "Viele Grüße\nMax");

    let new_thread_same_recipient =
        resolve_signature_for_compose(&pool, automatic(Some("thread-b")))
            .await
            .unwrap()
            .expect("assigned signature");
    assert_eq!(new_thread_same_recipient.variant, SignatureVariant::Long);
}

#[tokio::test]
async fn unconfigured_identity_uses_the_same_full_signature_for_every_message() {
    let pool = pool().await;
    seed_signature(&pool).await;
    sqlx::query("DELETE FROM signature_assignments")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        r#"INSERT INTO messages
         (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, date,
          body_text, is_read, is_draft, is_sent, has_attachments, created_at, updated_at)
         VALUES ('sent-alias-default', 'acc-1', 'sent-1', 'thread-default', 'consulting@example.com',
                 '["client@example.com"]', 'Status', '2026-01-03', 'Hello', 1, 0, 1, 0,
                 '2026-01-03', '2026-01-03')"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let resolved = resolve_signature_for_compose(&pool, automatic(Some("thread-default")))
        .await
        .unwrap()
        .expect("default signature");

    assert_eq!(resolved.variant, SignatureVariant::Long);
    assert_eq!(resolved.body_text, "Freundliche Grüße\nMax Mustermann");
}

#[test]
fn discovery_finds_multiple_repeated_end_blocks_without_copying_quoted_signatures() {
    let long = "Freundliche Grüße\nMax Mustermann\nMuster Software";
    let short = "Viele Grüße\nMax";
    let messages = vec![
        SignatureDetectionMessage::text("thread-1", format!("Projekt A\n\n{long}")),
        SignatureDetectionMessage::text("thread-2", format!("Projekt B\n\n{long}")),
        SignatureDetectionMessage::text(
            "thread-3",
            format!("Projekt C\n\n{long}\n\nAm 1. Juli schrieb jemand:\n> Fremde Grüße\n> Alice"),
        ),
        SignatureDetectionMessage::text("thread-4", format!("Kurze Antwort A\n\n{short}")),
        SignatureDetectionMessage::text("thread-5", format!("Kurze Antwort B\n\n{short}")),
        SignatureDetectionMessage::text("thread-6", format!("Kurze Antwort C\n\n{short}")),
    ];

    let candidates = detect_signature_candidates(&messages, 3);

    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().any(|candidate| {
        candidate.body_text.contains("Freundliche Grüße") && candidate.evidence_count == 3
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.body_text.contains("Viele Grüße") && candidate.evidence_count == 3
    }));
    assert!(candidates
        .iter()
        .all(|candidate| !candidate.body_text.contains("Alice")));
}

#[test]
fn discovery_stops_at_spark_style_quote_intro_lines() {
    let signature = "Max Mustermann\nmail@example.de";
    let messages = vec![
        SignatureDetectionMessage::text(
            "thread-1",
            format!("Antwort A\n\n{signature}\nNachricht von alice@example.de am 18. Mai 2025, 22:12 +0200:\n> Hallo"),
        ),
        SignatureDetectionMessage::text(
            "thread-2",
            format!("Antwort B\n\n{signature}\nNachricht von bob@example.de am 19. Mai 2025, 09:03 +0200:\n> Moin"),
        ),
        SignatureDetectionMessage::text(
            "thread-3",
            format!("Antwort C\n\n{signature}\n-----Ursprüngliche Nachricht-----\nVon: carol@example.de"),
        ),
    ];

    let candidates = detect_signature_candidates(&messages, 3);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].body_text, signature);
    assert_eq!(candidates[0].evidence_count, 3);
}

#[test]
fn discovery_prefers_the_full_multi_block_signature_over_its_fragments() {
    let block = "Muster Software\nPostfach 12 34, 12345 Musterstadt\nEinzelunternehmen | DE123456789\n\nmail@example.de | 0176 000 000 00";
    let full = format!("Freundliche Grüße\nMax Mustermann\n\n{block}");
    let messages = (1..=5)
        .map(|index| {
            SignatureDetectionMessage::text(
                format!("thread-{index}"),
                format!("Antwort {index}\n\n{full}"),
            )
        })
        .collect::<Vec<_>>();

    let candidates = detect_signature_candidates(&messages, 3);

    // The contact line and the company block repeat just as often as the full
    // signature — they are fragments of it, not suggestions of their own.
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].body_text, full);
    assert_eq!(candidates[0].evidence_count, 5);
}

#[test]
fn extraction_finds_the_signature_block_between_body_and_quote() {
    let html = r#"<html><body><div>
        <div>Hi,</div>
        <div>kannst du machen.</div>
        <div>&#160;</div>
        <div><b>Freundliche Grüße</b></div>
        <div>Max <a href="https://example.de">Mustermann</a></div>
        <div>&#160;</div>
        <div>Nachricht von alice@example.de am 18. Mai 2025:</div>
        <blockquote><div>Altes Zitat mit viel Text</div></blockquote>
    </div></body></html>"#;

    let extracted = extract_signature_html(html, "Freundliche Grüße\nMax Mustermann")
        .expect("signature html found");

    assert!(extracted.contains("<b>Freundliche Grüße</b>"));
    assert!(extracted.contains("https://example.de"));
    assert!(!extracted.contains("kannst du machen"));
    assert!(!extracted.contains("Altes Zitat"));
    assert!(!extracted.contains("Nachricht von"));
}

#[test]
fn extraction_returns_none_when_the_text_never_appears() {
    let html = "<html><body><div>Nur Fließtext ohne die gesuchte Signatur.</div></body></html>";
    assert!(extract_signature_html(html, "Freundliche Grüße\nMax Mustermann").is_none());
}

#[test]
fn discovery_accepts_single_contact_lines_but_not_bare_closings() {
    let contact = "mail@example.de | 0170 000 000 00";
    let closing = "Viele liebe Grüße";
    let messages = vec![
        SignatureDetectionMessage::text("thread-1", format!("Antwort A\n\n{contact}")),
        SignatureDetectionMessage::text("thread-2", format!("Antwort B\n\n{contact}")),
        SignatureDetectionMessage::text("thread-3", format!("Antwort C\n\n{contact}")),
        SignatureDetectionMessage::text("thread-4", format!("Antwort D\n\n{closing}")),
        SignatureDetectionMessage::text("thread-5", format!("Antwort E\n\n{closing}")),
        SignatureDetectionMessage::text("thread-6", format!("Antwort F\n\n{closing}")),
    ];

    let candidates = detect_signature_candidates(&messages, 3);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].body_text, contact);
}
