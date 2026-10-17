//! Why every writing transaction in `db/` and `sync/` opens with
//! `BEGIN IMMEDIATE`. The source-level guard lives in
//! `src-tauri/tests/db_transaction_scan.rs`; this pins the behaviour it guards.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn a_deferred_transaction_that_reads_first_loses_its_write() {
    let dir = tempfile::tempdir().expect("temp dir");
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("satchel.db"))
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("pool");
    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await
        .expect("wal");
    sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .expect("table");
    sqlx::query("INSERT INTO t (id, v) VALUES (1, 0)")
        .execute(&pool)
        .await
        .expect("seed row");

    // Deferred: the SELECT pins a read snapshot, another connection commits,
    // and the upgrade to a writer is refused with SQLITE_BUSY_SNAPSHOT — the
    // one busy code SQLite never runs the busy handler for, so the pool's
    // 15 s `busy_timeout` does nothing here.
    let mut deferred = pool.begin().await.expect("deferred transaction");
    let _: i64 = sqlx::query_scalar("SELECT v FROM t WHERE id = 1")
        .fetch_one(&mut *deferred)
        .await
        .expect("snapshot read");
    sqlx::query("INSERT INTO t (id, v) VALUES (2, 0)")
        .execute(&pool)
        .await
        .expect("concurrent commit");
    let refused = sqlx::query("UPDATE t SET v = 1 WHERE id = 1")
        .execute(&mut *deferred)
        .await;
    assert!(
        refused.is_err(),
        "a deferred read-then-write transaction unexpectedly survived a \
         concurrent commit — the guard scan would be pointless"
    );
    drop(deferred);

    // Immediate: the write lock is held from the first statement, so the same
    // sequence goes through and the other writer waits instead.
    let mut immediate = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("immediate transaction");
    let _: i64 = sqlx::query_scalar("SELECT v FROM t WHERE id = 1")
        .fetch_one(&mut *immediate)
        .await
        .expect("snapshot read");
    sqlx::query("UPDATE t SET v = 1 WHERE id = 1")
        .execute(&mut *immediate)
        .await
        .expect("write under the held lock");
    immediate.commit().await.expect("commit");

    let value: i64 = sqlx::query_scalar("SELECT v FROM t WHERE id = 1")
        .fetch_one(&pool)
        .await
        .expect("final read");
    assert_eq!(value, 1);
}
