//! Repeatable query timings on 300k messages / 150k threads across five accounts.
//! Run explicitly with `cargo test -p satchel-core --test query_benchmark -- --ignored --nocapture`.
use satchel_core::db::{
    messages::{MessageSearchRepository, SearchQuery},
    threads::{ThreadListRequest, ThreadRepository},
};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::time::Instant;

async fn seed(pool: &SqlitePool) {
    sqlx::raw_sql(
        "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<4)
         INSERT INTO accounts(id,display_name,email,provider,config,credentials_ref,created_at,updated_at)
         SELECT 'a'||x,'Account','me'||x||'@test.local','imap','{}','','2026-01-01','2026-01-01' FROM n;
         INSERT INTO folders(id,account_id,name,full_path,folder_type,created_at,updated_at)
         SELECT 'inbox'||id,id,'Inbox','INBOX','inbox','2026-01-01','2026-01-01' FROM accounts;
         INSERT INTO folders(id,account_id,name,full_path,folder_type,created_at,updated_at)
         SELECT 'sparse'||id,id,'Sparse','Sparse','drafts','2026-01-01','2026-01-01' FROM accounts;
         WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<149999)
         INSERT INTO threads(id,account_id,subject,last_message_date,message_count,unread_count,workflow_state,created_at,updated_at)
         SELECT printf('t%06d',x),'a'||(x%5),'Mail '||x,datetime('2026-01-01','+'||x||' seconds'),2,1,'inbox','2026-01-01','2026-01-01' FROM n;
         WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<299999)
         INSERT INTO messages(id,account_id,folder_id,thread_id,from_address,to_addresses,cc_addresses,bcc_addresses,subject,body_text,snippet,date,is_read,remote_id,raw_size,created_at,updated_at)
         SELECT printf('m%06d',x),'a'||((x/2)%5),
                CASE WHEN x<80 THEN 'sparse' ELSE 'inbox' END||'a'||((x/2)%5),printf('t%06d',x/2),
                CASE WHEN x<80 THEN 'contact@test.local' ELSE 'sender'||x||'@test.local' END,
                '[\"me@test.local\"]','[]','[]','Mail '||x,
                'market'||x||' meeting'||x||' memo'||x||' money'||x||' '||replace(hex(zeroblob(512)),'0','body '),
                'Snippet',datetime('2026-01-01','+'||x||' seconds'),x%2,CAST(x AS TEXT),4096,'2026-01-01','2026-01-01' FROM n;",
    ).execute(pool).await.expect("seed");
}

#[tokio::test]
#[ignore = "benchmark; run explicitly"]
async fn queries_300k_messages() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("bench.db").display()
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    satchel_core::db::init(&pool).await.unwrap();
    seed(&pool).await;
    for repair in satchel_core::db::data_repairs::registered_repairs() {
        satchel_core::db::data_repairs::run_repair(&pool, repair, 1_000, None, |_| {})
            .await
            .unwrap();
    }
    sqlx::query("ANALYZE").execute(&pool).await.unwrap();
    for name in [
        "sidebar",
        "sparse folder",
        "contact",
        "dense inbox",
        "search m",
        "search ma",
    ] {
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            match name {
                "sidebar" => {
                    assert_eq!(
                        satchel_core::db::mailboxes::list_mailboxes(&pool)
                            .await
                            .unwrap()
                            .len(),
                        10
                    );
                }
                "sparse folder" | "contact" | "dense inbox" => {
                    let request = if name == "contact" {
                        ThreadListRequest {
                            contact_email: Some("contact@test.local".into()),
                            ..Default::default()
                        }
                    } else if name == "dense inbox" {
                        ThreadListRequest {
                            mailbox_kind: Some("section".into()),
                            mailbox_value: Some("inbox".into()),
                            ..Default::default()
                        }
                    } else {
                        ThreadListRequest {
                            mailbox_kind: Some("folder".into()),
                            mailbox_value: Some("Sparse".into()),
                            ..Default::default()
                        }
                    };
                    let page = ThreadRepository::new(&pool).list(request).await.unwrap();
                    assert_eq!(
                        page.threads.len(),
                        if name == "dense inbox" { 200 } else { 40 }
                    );
                }
                _ => {
                    let response = MessageSearchRepository::new(&pool)
                        .search(&SearchQuery {
                            query: name[7..].into(),
                            account_id: None,
                            limit: None,
                            cursor: None,
                            list_scope: None,
                        })
                        .await
                        .unwrap();
                    assert_eq!(response.hits.len(), if name == "search m" { 0 } else { 50 });
                }
            }
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.remove(0);
        times.sort_by(f64::total_cmp);
        println!(
            "{name}: median {:.3} ms, min {:.3} ms, max {:.3} ms",
            (times[2] + times[3]) / 2.0,
            times[0],
            times[5]
        );
    }
    pool.close().await;
}
