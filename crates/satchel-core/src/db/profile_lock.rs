//! Locks between processes that open the same profile database.
//!
//! On Android WorkManager runs in a process of its own and opens the database
//! itself. Every process holds [`DatabaseOpenLock`] while its pool is open, a
//! restore swap needs that lock exclusively, and migrations take turns on a
//! second lock file. The OS releases a lock with its process, so a crash
//! leaves nothing stale.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
    time::Duration,
};

/// Shared while a pool is open, exclusive for a restore swap.
pub const OPEN_LOCK_FILE: &str = "satchel.db.lock";
const MIGRATION_LOCK_SUFFIX: &str = ".migrate.lock";
const POLL: Duration = Duration::from_millis(25);

/// Held for as long as a pool on the profile database is open.
#[derive(Debug)]
pub struct DatabaseOpenLock {
    _file: File,
}

fn lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn try_file_lock(file: &File, shared: bool) -> Result<(), TryLockError> {
    #[cfg(target_os = "android")]
    {
        try_android_lock(file, shared)
    }
    #[cfg(not(target_os = "android"))]
    {
        if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        }
    }
}

/// The pinned Android standard library reports file locks as unsupported.
/// Use the OS lock so WorkManager and restore still exclude each other.
#[cfg(any(target_os = "android", all(test, unix)))]
fn try_android_lock(file: &File, shared: bool) -> Result<(), TryLockError> {
    use std::os::fd::AsRawFd;

    let operation = if shared { libc::LOCK_SH } else { libc::LOCK_EX };
    // SAFETY: the borrowed File owns a live descriptor for the entire call.
    if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Err(TryLockError::WouldBlock)
    } else {
        Err(TryLockError::Error(error))
    }
}

/// Waits only while another process is swapping in a restore.
pub async fn hold_open(profile: &Path) -> Result<DatabaseOpenLock, String> {
    let file = lock_file(&profile.join(OPEN_LOCK_FILE)).map_err(|err| err.to_string())?;
    loop {
        match try_file_lock(&file, true) {
            Ok(()) => return Ok(DatabaseOpenLock { _file: file }),
            Err(TryLockError::WouldBlock) => tokio::time::sleep(POLL).await,
            Err(TryLockError::Error(err)) => return Err(err.to_string()),
        }
    }
}

/// The exclusive lock, or `None` while any process has the database open.
pub fn try_exclusive(profile: &Path) -> Result<Option<File>, String> {
    let file = lock_file(&profile.join(OPEN_LOCK_FILE)).map_err(|err| err.to_string())?;
    match try_file_lock(&file, false) {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(err)) => Err(err.to_string()),
    }
}

/// Serialises migration runs on `database`. sqlx reads the applied versions
/// once and then applies, with nothing in between to stop a second process.
pub(crate) async fn lock_migrations(database: &Path) -> io::Result<File> {
    let mut name = database.as_os_str().to_owned();
    name.push(MIGRATION_LOCK_SUFFIX);
    let file = lock_file(Path::new(&name))?;
    loop {
        match try_file_lock(&file, false) {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => tokio::time::sleep(POLL).await,
            Err(TryLockError::Error(err)) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn android_locks_share_exclude_restore_and_release_on_close() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join(OPEN_LOCK_FILE);
        let first = lock_file(&path).unwrap();
        let second = lock_file(&path).unwrap();
        let exclusive = lock_file(&path).unwrap();

        try_android_lock(&first, true).unwrap();
        try_android_lock(&second, true).unwrap();
        assert!(matches!(
            try_android_lock(&exclusive, false),
            Err(TryLockError::WouldBlock)
        ));
        drop(first);
        assert!(matches!(
            try_android_lock(&exclusive, false),
            Err(TryLockError::WouldBlock)
        ));
        drop(second);
        try_android_lock(&exclusive, false).unwrap();

        let waiting = lock_file(&path).unwrap();
        assert!(matches!(
            try_android_lock(&waiting, true),
            Err(TryLockError::WouldBlock)
        ));
        assert!(matches!(
            try_android_lock(&waiting, false),
            Err(TryLockError::WouldBlock)
        ));
        drop(exclusive);
        try_android_lock(&waiting, false).unwrap();
    }

    #[tokio::test]
    async fn an_open_database_blocks_the_exclusive_lock_until_it_closes() {
        let profile = tempfile::tempdir().unwrap();

        let open = hold_open(profile.path()).await.unwrap();
        assert!(try_exclusive(profile.path()).unwrap().is_none());

        drop(open);
        assert!(try_exclusive(profile.path()).unwrap().is_some());
    }

    #[tokio::test]
    async fn two_openers_share_the_lock() {
        let profile = tempfile::tempdir().unwrap();

        let _first = hold_open(profile.path()).await.unwrap();
        let second = tokio::time::timeout(Duration::from_secs(1), hold_open(profile.path())).await;

        assert!(second.is_ok());
    }
}
