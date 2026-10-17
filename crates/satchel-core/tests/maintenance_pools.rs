use satchel_core::db::{
    self,
    threads::{ThreadListRequest, ThreadRepository},
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::time::{Duration, Instant};

#[tokio::test]
async fn every_connection_has_the_maintenance_pragmas() {
    let dir = tempfile::tempdir().unwrap();
    let pools = db::connect_pools(dir.path().join("satchel.db"))
        .await
        .unwrap();
    for pool in [pools.read.as_ref(), &pools.write, &pools.checkpoint] {
        let mut held = Vec::new();
        for _ in 0..pool.options().get_max_connections() {
            let mut connection = pool.acquire().await.unwrap();
            for (pragma, expected) in [
                ("wal_autocheckpoint", 400),
                ("cache_size", -8192),
                ("mmap_size", 67108864),
                ("temp_store", 2),
            ] {
                let value: i64 = sqlx::query_scalar(&format!("PRAGMA {pragma}"))
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap();
                assert_eq!(value, expected, "{pragma}");
            }
            held.push(connection);
        }
        // Replacement connections inherit the same settings.
        for connection in held {
            connection.close().await.unwrap();
        }
        let threshold: i64 = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(threshold, 400);
    }
    assert!(
        sqlx::query("DELETE FROM app_settings")
            .execute(pools.read.as_ref())
            .await
            .is_err(),
        "read pool must reject writes"
    );
}

async fn seed(pool: &SqlitePool, messages: usize) {
    sqlx::query("INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, created_at, updated_at) VALUES ('a', 'A', 'a@b.test', 'imap', '{}', '', '{}', '', '')").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at) VALUES ('f', 'a', 'INBOX', 'INBOX', 'inbox', '', '')").execute(pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2000) INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, workflow_state, created_at, updated_at) SELECT printf('t%06d', x), 'a', 'Benchmark', '2026-01-01', 10, 0, 'inbox', '', '' FROM n").execute(pool).await.unwrap();
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<?) INSERT INTO messages (id, account_id, folder_id, thread_id, from_address, to_addresses, subject, body_text, date, created_at, updated_at) SELECT printf('m%06d', x), 'a', 'f', printf('t%06d', 1+(x-1)%2000), 'a@b.test', '[]', 'Benchmark', printf('%01000d', x), '2026-01-01', '', '' FROM n").bind(messages as i64).execute(pool).await.unwrap();
}

#[tokio::test]
async fn inbox_reads_do_not_wait_for_writers_or_snapshot_storage() {
    let dir = tempfile::tempdir().unwrap();
    let pools = db::connect_pools(dir.path().join("satchel.db"))
        .await
        .unwrap();
    seed(&pools.write, 1).await;
    let first = pools.write.acquire().await.unwrap();
    let second = pools.write.acquire().await.unwrap();
    let page = tokio::time::timeout(
        Duration::from_secs(1),
        ThreadRepository::with_pools(pools.read.as_ref(), &pools.write).list(ThreadListRequest {
            limit: Some(200),
            ..Default::default()
        }),
    )
    .await
    .expect("Inbox read must use its own connections")
    .unwrap();
    assert_eq!(page.threads.len(), 1);
    drop((first, second));
}

async fn contended_read(
    read: &SqlitePool,
    write: &SqlitePool,
    blocker: &SqlitePool,
    split: bool,
) -> Duration {
    let tx = blocker.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let mut writers = Vec::new();
    for _ in 0..5 {
        let pool = write.clone();
        writers.push(tokio::spawn(async move {
            sqlx::query(
                "UPDATE app_settings SET updated_at = 'bench' WHERE key = 'data_repair.cc_index'",
            )
            .execute(&pool)
            .await
            .unwrap();
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while write.num_idle() != 0 || write.size() < write.options().get_max_connections() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        tx.rollback().await.unwrap();
    });
    let started = Instant::now();
    let repository = if split {
        ThreadRepository::with_pools(read, write)
    } else {
        ThreadRepository::new(read)
    };
    let page = repository
        .list(ThreadListRequest {
            limit: Some(200),
            ..Default::default()
        })
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(page.threads.len(), 200);
    release.await.unwrap();
    for writer in writers {
        writer.await.unwrap();
    }
    elapsed
}

#[tokio::test]
#[ignore = "contention benchmark; run explicitly"]
async fn pool_split_read_latency_under_waiting_writers() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("satchel.db");
    let pools = db::connect_pools(&file).await.unwrap();
    seed(&pools.write, 20_000).await;
    let old_options = SqliteConnectOptions::new()
        .filename(&file)
        .foreign_keys(true)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(15));
    let legacy = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(old_options.clone())
        .await
        .unwrap();
    let blocker = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(old_options)
        .await
        .unwrap();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for _ in 0..5 {
        before.push(
            contended_read(&legacy, &legacy, &blocker, false)
                .await
                .as_secs_f64()
                * 1000.0,
        );
        after.push(
            contended_read(pools.read.as_ref(), &pools.write, &blocker, true)
                .await
                .as_secs_f64()
                * 1000.0,
        );
    }
    before.sort_by(f64::total_cmp);
    after.sort_by(f64::total_cmp);
    println!("20,000 messages / 2,000 threads / 200-row Inbox / 5 waiting writers / 250 ms lock: before median {:.2} ms (range {:.2}..{:.2}); after median {:.2} ms (range {:.2}..{:.2})", before[2], before[0], before[4], after[2], after[0], after[4]);
    assert!(before[2] >= 250.0);
    assert!(
        after[2] < 250.0,
        "read must finish before the writer lock is released"
    );
}
