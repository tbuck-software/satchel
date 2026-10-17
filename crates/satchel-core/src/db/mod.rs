pub mod accounts;
pub mod aliases;
pub mod blocked_senders;
pub mod body_window;
pub mod calendar;
pub mod connection_state;
pub mod contacts;
pub mod data_repairs;
pub mod folders;
pub mod followups;
pub mod mailboxes;
pub mod maintenance;
pub mod message_links;
pub mod messages;
pub mod oauth;
pub mod plugin_permissions;
pub mod profile_lock;
pub mod queries;
pub mod sender_preferences;
pub mod signatures;
pub mod smart_folders;
pub mod smime;
pub mod sync_quarantine;
pub mod templates;
pub mod threads;
pub mod topics;
pub mod trash;
pub mod vacation;
pub mod webhooks;

use std::{path::PathBuf, time::Duration};

use sqlx::{
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

/// Portable backup key filenames and legacy profile fallback locations.
/// Export reads the active OS keychain key even when neither file exists.
pub const MASTER_KEY_FILES: &[&str] = &include!("master-key-files.json");

/// Suffix for a staged restore. Applied by `apply_pending_restore` on the next
/// launch, before the pool opens.
pub const RESTORE_PENDING_SUFFIX: &str = ".restore-pending";

fn migrator() -> Migrator {
    let mut migrator = sqlx::migrate!("./src/db/migrations");

    // Versions 23-26 shipped in daily builds and were later removed from the
    // source tree. Existing databases still record them as applied, so SQLx
    // must tolerate those historical entries while continuing to validate
    // every migration that remains in the resolved set.
    migrator.set_ignore_missing(true);
    migrator
}

/// Highest migration version this binary can resolve. A backup above it came
/// from a newer build; `set_ignore_missing(true)` would wave it through and
/// leave the schema ahead of the code reading it.
pub fn latest_migration_version() -> i64 {
    migrator()
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0)
}

pub async fn init(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    let _migration_lock = match main_database_file(pool).await? {
        Some(file) => Some(profile_lock::lock_migrations(&file).await?),
        None => None,
    };
    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(pool)
        .await?;
    migrator().run(pool).await?;
    Ok(())
}

/// `None` for an in-memory database, which no other process can see.
async fn main_database_file(pool: &SqlitePool) -> Result<Option<PathBuf>, sqlx::Error> {
    let file: Option<String> =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name = 'main'")
            .fetch_optional(pool)
            .await?;
    Ok(file.filter(|file| !file.is_empty()).map(PathBuf::from))
}

/// No `messages_fts` means no index to be wrong about — an import fixture or a
/// foreign file, both of which other checks judge.
async fn has_search_index(pool: &SqlitePool) -> Result<bool, String> {
    let present: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'messages_fts'")
            .fetch_one(pool)
            .await
            .map_err(|err| err.to_string())?;
    Ok(present > 0)
}

/// Whether the search index still agrees with the messages it indexes.
///
/// `messages_fts` is external-content FTS5 held in step by triggers, which
/// `PRAGMA integrity_check` never looks at. FTS5's own `integrity-check` needs
/// a writable connection and, in the SQLite linked here, does not compare
/// index against content table — hence the second, row-count half.
pub async fn message_index_intact(pool: &SqlitePool) -> Result<bool, String> {
    if !has_search_index(pool).await? {
        return Ok(true);
    }
    let structure_ok =
        match sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('integrity-check')")
            .execute(pool)
            .await
        {
            Ok(_) => true,
            // A damaged index is reported as a query error, not as a row.
            Err(sqlx::Error::Database(_)) => false,
            Err(err) => return Err(err.to_string()),
        };
    Ok(structure_ok && message_index_matches_row_count(pool).await?)
}

/// Indexed documents against message rows. Coarser than
/// [`message_index_intact`] — no corrupt term list is seen — but it needs no
/// write access, which is the only kind a backup file gets.
pub async fn message_index_matches_row_count(pool: &SqlitePool) -> Result<bool, String> {
    if !has_search_index(pool).await? {
        return Ok(true);
    }
    let shadow: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name = 'messages_fts_docsize'",
    )
    .fetch_one(pool)
    .await
    .map_err(|err| err.to_string())?;
    if shadow == 0 {
        return Ok(true);
    }
    let indexed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages_fts_docsize")
        .fetch_one(pool)
        .await
        .map_err(|err| err.to_string())?;
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await
        .map_err(|err| err.to_string())?;
    Ok(indexed == rows)
}

/// Apply only a completely published restore before opening the profile pool.
pub fn apply_pending_restore(profile_dir: &std::path::Path) -> Result<bool, String> {
    if !crate::backup::restore_pending(profile_dir) {
        return Ok(false);
    }
    let key = crate::utils::crypto::load_or_create_master_key(profile_dir).ok();
    match crate::backup::apply_pending_restore(profile_dir, key.as_ref()) {
        Err(error)
            if !crate::backup::recovery_in_progress(profile_dir)
                && profile_dir.join("satchel.db").is_file() =>
        {
            // Preparation failed before touching the live database. Keep the
            // app available so the user can cancel or stage another restore.
            tracing::error!(%error, "staged restore skipped; live database retained");
            Ok(false)
        }
        result => result,
    }
}

static HELD_OPEN_LOCKS: std::sync::Mutex<Vec<profile_lock::DatabaseOpenLock>> =
    std::sync::Mutex::new(Vec::new());

/// Open the profile database at `db_path`, applying a staged restore first.
///
/// The caller resolves the path: on the desktop and on mobile that is the
/// Tauri app-data directory, and only the app crate knows how to ask for it.
pub async fn connect(db_path: impl Into<PathBuf>) -> Result<SqlitePool, String> {
    let mut db_path = db_path.into();
    let mut restored = false;

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        restored = apply_pending_restore(parent)?;
        let open = profile_lock::hold_open(parent).await?;
        // The app's pool lives as long as the process; so does its lock.
        HELD_OPEN_LOCKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(open);
    } else {
        db_path = PathBuf::from("satchel.db");
    }

    let options = connection_options(&db_path);

    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .map_err(|err| err.to_string())?;

    init(&pool).await.map_err(|err| err.to_string())?;
    seed_if_empty(&pool).await.map_err(|err| err.to_string())?;
    if restored {
        maintenance::request_fts_check(&pool)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(pool)
}

/// Connection-local settings apply to every connection, including replacements.
pub fn connection_options(path: &std::path::Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(15))
        .pragma("wal_autocheckpoint", "400")
        .pragma("cache_size", "-8192")
        .pragma("mmap_size", "67108864")
        .pragma("temp_store", "MEMORY")
}

/// The write pool remains the default for mixed operations and background work.
/// Read commands receive a separate, read-only pool through Tauri state.
#[derive(Clone)]
pub struct ReadPool(SqlitePool);

impl AsRef<SqlitePool> for ReadPool {
    fn as_ref(&self) -> &SqlitePool {
        &self.0
    }
}

pub struct DatabasePools {
    pub read: ReadPool,
    pub write: SqlitePool,
    pub checkpoint: SqlitePool,
}

pub async fn connect_pools(path: impl Into<PathBuf>) -> Result<DatabasePools, String> {
    let path = path.into();
    let write = connect(&path).await?;
    let read = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(
            connection_options(&path)
                .create_if_missing(false)
                .read_only(true),
        )
        .await
        .map_err(|err| err.to_string())?;
    let checkpoint = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connection_options(&path).busy_timeout(Duration::from_secs(1)))
        .await
        .map_err(|err| err.to_string())?;
    Ok(DatabasePools {
        read: ReadPool(read),
        write,
        checkpoint,
    })
}

async fn seed_if_empty(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let existing_accounts: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM accounts")
        .fetch_one(pool)
        .await?;

    if existing_accounts > 0 {
        return Ok(());
    }

    if !should_seed_demo_data() {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let account_id = "acc-work";
    let inbox_folder = "fld-inbox";
    let sent_folder = "fld-sent";

    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, sync_state, color, is_active, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(account_id)
    .bind("Work")
    .bind("tom@satchel.dev")
    .bind("imap")
    .bind("{\"imap\":{\"host\":\"imap.example.com\",\"port\":993,\"tls\":true}}")
    // Vestigial: nothing reads this column. It is kept because dropping a column
    // needs a migration, but the value must not claim a keychain location — the
    // credential lives inside the encrypted `config` blob, and a reader auditing
    // the schema would otherwise conclude the keychain holds it.
    .bind("encrypted-in-config")
    .bind("{}")
    .bind("#55a6ff")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    for (folder_id, folder_name, folder_type) in [
        (inbox_folder, "Inbox", "inbox"),
        (sent_folder, "Sent", "sent"),
    ] {
        sqlx::query(
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, remote_id, uidvalidity, message_count, unread_count, sync_enabled, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, 0, 0, 1, 0, ?, ?)",
        )
        .bind(folder_id)
        .bind(account_id)
        .bind(folder_name)
        .bind(folder_name)
        .bind(folder_type)
        .bind(folder_name)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
    }

    for (
        thread_id,
        subject,
        state,
        unread,
        snippet,
        from_name,
        from_address,
        folder_id,
        category,
        is_starred,
        priority,
    ) in [
        (
            "th-1",
            "Staging login 500 after deployment",
            "todo",
            1_i64,
            "Users cannot sign in on staging since this morning...",
            "Sarah Kim",
            "sarah@app.io",
            inbox_folder,
            None,
            0_i64,
            3_i64,
        ),
        (
            "th-2",
            "Invoice March (waiting for vendor correction)",
            "waiting",
            0_i64,
            "Please confirm corrected line items in attached PDF.",
            "Vendor Ops",
            "billing@vendor.example",
            inbox_folder,
            None,
            0_i64,
            2_i64,
        ),
        (
            "th-6",
            "Google Cloud billing threshold reached",
            "inbox",
            1_i64,
            "Your configured billing threshold for Project Atlas has been reached.",
            "Google Cloud",
            "billing-noreply@cloud.google.com",
            inbox_folder,
            Some("updates"),
            1_i64,
            2_i64,
        ),
        (
            "th-7",
            "Google AI Studio quota warning",
            "inbox",
            0_i64,
            "Your usage is approaching the daily quota for Gemini API calls.",
            "Google AI Studio",
            "no-reply@aistudio.google.com",
            inbox_folder,
            Some("updates"),
            1_i64,
            2_i64,
        ),
        (
            "th-8",
            "Google Maps Platform key restriction reminder",
            "inbox",
            0_i64,
            "Protect production keys by adding HTTP referrer restrictions.",
            "Google Maps Platform",
            "maps-noreply@google.com",
            inbox_folder,
            Some("updates"),
            1_i64,
            1_i64,
        ),
        (
            "th-9",
            "App Store Connect: Build 42 finished processing",
            "inbox",
            1_i64,
            "Version 0.3.0 build 42 is ready for TestFlight review.",
            "App Store Connect",
            "no_reply@email.apple.com",
            inbox_folder,
            Some("updates"),
            0_i64,
            1_i64,
        ),
        (
            "th-10",
            "GitHub Actions: deploy.yml failed",
            "inbox",
            2_i64,
            "The deploy workflow failed on main after 3m 12s.",
            "GitHub",
            "notifications@github.com",
            inbox_folder,
            Some("updates"),
            0_i64,
            2_i64,
        ),
        (
            "th-11",
            "Sentry alert: API error rate above threshold",
            "inbox",
            1_i64,
            "Error rate for api.satchel.dev crossed the configured threshold.",
            "Sentry",
            "alerts@sentry.io",
            inbox_folder,
            Some("updates"),
            0_i64,
            2_i64,
        ),
        (
            "th-12",
            "Linear: Inbox triage milestone moved",
            "inbox",
            0_i64,
            "The Inbox triage milestone was moved to next Friday.",
            "Linear",
            "notifications@linear.app",
            inbox_folder,
            Some("updates"),
            0_i64,
            1_i64,
        ),
        (
            "th-13",
            "Stripe payout is on the way",
            "inbox",
            0_i64,
            "A payout of EUR 4,218.90 is expected to arrive tomorrow.",
            "Stripe",
            "support@stripe.com",
            inbox_folder,
            Some("updates"),
            0_i64,
            1_i64,
        ),
        (
            "th-14",
            "DHL Paket wurde zugestellt",
            "inbox",
            1_i64,
            "Ihre Sendung wurde an den gewuenschten Ablageort geliefert.",
            "DHL Paket",
            "paket@dhl.de",
            inbox_folder,
            Some("updates"),
            0_i64,
            0_i64,
        ),
        (
            "th-15",
            "Amazon Spar-Abo wird bald versendet",
            "inbox",
            0_i64,
            "Pruefen Sie Ihre Spar-Abo Lieferung vor dem Versand.",
            "Amazon.de",
            "auto-confirm@amazon.de",
            inbox_folder,
            Some("updates"),
            0_i64,
            0_i64,
        ),
        (
            "th-25",
            "Expedia Group Lodging Team: Booking update",
            "inbox",
            1_i64,
            "A partner message requires review before the end of the day.",
            "Expedia Group Lodging Team",
            "lodging@expedia.com",
            inbox_folder,
            Some("updates"),
            0_i64,
            0_i64,
        ),
        (
            "th-26",
            "Google Business Profile: Neue Rezension",
            "inbox",
            0_i64,
            "Sie haben eine neue Rezension erhalten, die beantwortet werden kann.",
            "Google Business Profile",
            "businessprofile-noreply@google.com",
            inbox_folder,
            Some("updates"),
            0_i64,
            0_i64,
        ),
        (
            "th-27",
            "Vercel deployment succeeded",
            "inbox",
            0_i64,
            "Production deployment for satchel-web completed successfully.",
            "Vercel",
            "notifications@vercel.com",
            inbox_folder,
            Some("updates"),
            0_i64,
            0_i64,
        ),
        (
            "th-16",
            "METRO Wochenangebote",
            "inbox",
            1_i64,
            "Neue Angebote fuer Buero, Kueche und Vorrat sind verfuegbar.",
            "METRO Wochenangebote",
            "newsletter@metro.de",
            inbox_folder,
            Some("promotions"),
            0_i64,
            0_i64,
        ),
        (
            "th-17",
            "TechCrunch Daily: Funding rounds and AI launches",
            "inbox",
            0_i64,
            "Today in startups: new AI launches, acquisitions and funding rounds.",
            "TechCrunch via LinkedIn",
            "news@linkedin.com",
            inbox_folder,
            Some("promotions"),
            0_i64,
            0_i64,
        ),
        (
            "th-18",
            "Kickstarter News Track: Projects ending soon",
            "inbox",
            0_i64,
            "Three saved projects are ending within the next 48 hours.",
            "Kickstarter News Track",
            "news@kickstarter.com",
            inbox_folder,
            Some("promotions"),
            0_i64,
            0_i64,
        ),
        (
            "th-19",
            "Product Hunt Daily Digest",
            "inbox",
            0_i64,
            "The most discussed tools from today, including new developer workflows.",
            "Product Hunt",
            "hello@producthunt.com",
            inbox_folder,
            Some("promotions"),
            0_i64,
            0_i64,
        ),
        (
            "th-20",
            "The Pragmatic Engineer: Platform teams",
            "inbox",
            0_i64,
            "A deep dive on platform teams and developer productivity.",
            "The Pragmatic Engineer",
            "newsletter@pragmaticengineer.com",
            inbox_folder,
            Some("promotions"),
            0_i64,
            0_i64,
        ),
        (
            "th-21",
            "Hacker News Weekly",
            "inbox",
            0_i64,
            "Top engineering discussions and releases from this week.",
            "Hacker News Weekly",
            "weekly@hackernewsletter.com",
            inbox_folder,
            Some("promotions"),
            0_i64,
            0_i64,
        ),
        (
            "th-22",
            "LinkedIn: 4 updates from your network",
            "inbox",
            1_i64,
            "New comments, profile views and messages from your network.",
            "LinkedIn",
            "messages-noreply@linkedin.com",
            inbox_folder,
            Some("social"),
            0_i64,
            0_i64,
        ),
        (
            "th-23",
            "X: Login from a new device",
            "inbox",
            1_i64,
            "We noticed a login from a new device. Review this activity.",
            "X",
            "info@x.com",
            inbox_folder,
            Some("social"),
            0_i64,
            0_i64,
        ),
        (
            "th-24",
            "Coffee with design systems group",
            "inbox",
            0_i64,
            "Could we move coffee to Thursday? I want to bring the new component audit.",
            "Mina Patel",
            "mina@design.example",
            inbox_folder,
            Some("personal"),
            0_i64,
            1_i64,
        ),
    ] {
        let message_id = format!("m-{thread_id}");

        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, last_message_date, message_count, unread_count, has_attachments, participants, snippet, workflow_state, workflow_changed_at, snooze_until, due_date, priority, board_column, board_position, category, is_starred, created_at, updated_at)
             VALUES (?, ?, ?, ?, 1, ?, 0, ?, ?, ?, ?, NULL, NULL, ?, ?, 1, ?, ?, ?, ?)",
        )
        .bind(thread_id)
        .bind(account_id)
        .bind(subject)
        .bind(&now)
        .bind(unread)
        .bind(format!("[\"{from_address}\",\"tom@satchel.dev\"]"))
        .bind(snippet)
        .bind(state)
        .bind(&now)
        .bind(priority)
        .bind(state)
        .bind(category)
        .bind(is_starred)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, thread_id, message_id_header, in_reply_to, references_header, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, date, body_text, body_html, snippet, is_read, is_starred, is_draft, is_sent, has_attachments, remote_uid, remote_id, raw_size, raw_headers, category, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, NULL, NULL, NULL, ?, ?, ?, NULL, ?, ?, ?, 0, 0, 0, NULL, NULL, NULL, NULL, ?, ?, ?)",
        )
        .bind(&message_id)
        .bind(account_id)
        .bind(folder_id)
        .bind(thread_id)
        .bind(format!("<{message_id}@satchel.local>"))
        .bind(from_address)
        .bind(from_name)
        .bind("[\"tom@satchel.dev\"]")
        .bind(subject)
        .bind(&now)
        .bind(snippet)
        .bind(snippet)
        .bind(if unread == 0 { 1 } else { 0 })
        .bind(is_starred)
        .bind(category)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
    }

    sqlx::query(
        "INSERT INTO rules (id, name, description, is_active, priority, trigger_type, trigger_config, actions, last_triggered, trigger_count, created_at, updated_at)
         VALUES (?, ?, ?, 1, 10, 'on_receive', ?, ?, NULL, 0, ?, ?)",
    )
    .bind("rule-staging-to-todo")
    .bind("Staging Bugs to Todo")
    .bind("Moves staging incidents into todo with high priority")
    .bind("{\"subject_contains\":\"staging\"}")
    .bind("[\"set_state:todo\",\"set_priority:3\"]")
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(())
}

fn should_seed_demo_data() -> bool {
    demo_seed_enabled(std::env::var("SATCHEL_SEED_DEMO_DATA").ok().as_deref())
}

fn demo_seed_enabled(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{demo_seed_enabled, init, migrator, profile_lock};

    #[tokio::test]
    async fn migration_lock_precedes_the_wal_switch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("satchel.db");
        let pool = SqlitePoolOptions::new()
            // PRAGMA journal_mode reports connection-local cached state; use
            // the same connection before and after initialization.
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let mode_before: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        let lock = profile_lock::lock_migrations(&path).await.unwrap();

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let init_pool = pool.clone();
        let mut initializing = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            init(&init_pool).await
        });
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut initializing)
                .await
                .is_err(),
            "initialization must wait for the migration lock"
        );
        let mode_while_locked: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode_while_locked, mode_before);

        drop(lock);
        initializing.await.unwrap().unwrap();
        let mode_after: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode_after, "wal");
    }

    /// Two processes starting on one file after an update: WorkManager and the
    /// app. Separate pools stand in for them; the file lock does not care.
    #[tokio::test]
    async fn two_concurrent_migrators_apply_each_migration_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("satchel.db");
        let open = || {
            SqlitePoolOptions::new().max_connections(2).connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .busy_timeout(std::time::Duration::from_secs(15)),
            )
        };
        let (first, second) = tokio::join!(open(), open());
        let (first, second) = (first.unwrap(), second.unwrap());

        let (first_run, second_run) = tokio::join!(init(&first), init(&second));

        first_run.unwrap();
        second_run.unwrap();
        let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&first)
            .await
            .unwrap();
        assert_eq!(applied, migrator().iter().count() as i64);
    }

    #[test]
    fn demo_seed_is_strictly_opt_in() {
        assert!(!demo_seed_enabled(None));
        assert!(!demo_seed_enabled(Some("0")));
        assert!(demo_seed_enabled(Some("true")));
        assert!(demo_seed_enabled(Some("1")));
    }

    #[tokio::test]
    async fn init_accepts_an_applied_historical_migration() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");

        init(&pool).await.expect("initial migrations");
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, success, checksum, execution_time) \
             VALUES (23, 'historical migration', 1, X'00', 0)",
        )
        .execute(&pool)
        .await
        .expect("historical migration record");

        init(&pool)
            .await
            .expect("historical migration should be ignored");
    }

    #[tokio::test]
    async fn init_adds_covering_indexes_for_the_thread_list() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");

        init(&pool).await.expect("initial migrations");
        let index_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM sqlite_master
             WHERE type = 'index'
               AND name IN ('idx_messages_thread_latest', 'idx_threads_date_id')",
        )
        .fetch_one(&pool)
        .await
        .expect("thread-list indexes");

        assert_eq!(index_count, 2);
    }

    #[tokio::test]
    async fn init_adds_thread_list_snapshot_storage() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");

        init(&pool).await.expect("initial migrations");
        let table_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM sqlite_master
             WHERE type = 'table' AND name = 'thread_list_snapshots'",
        )
        .fetch_one(&pool)
        .await
        .expect("thread-list snapshot table");

        assert_eq!(table_count, 1);
    }

    #[tokio::test]
    async fn init_adds_pgp_signed_message_storage() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");

        init(&pool).await.expect("initial migrations");
        let column_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM pragma_table_info('messages')
             WHERE name = 'pgp_signed' AND type = 'BLOB'",
        )
        .fetch_one(&pool)
        .await
        .expect("PGP signed message column");

        assert_eq!(column_count, 1);
    }
}
