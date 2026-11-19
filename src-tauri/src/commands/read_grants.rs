//! What the import and restore commands may read.
//!
//! The read-side twin of [`crate::commands::write_grants`]: a path the user
//! picked in a dialog opened here, or one the operating system handed to the
//! app, is granted; a path the webview names on its own is refused.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri_plugin_dialog::DialogExt;

use crate::commands::error::{CommandError, CommandResult};

#[derive(Default)]
pub struct ReadGrants {
    files: Mutex<HashSet<PathBuf>>,
    folders: Mutex<HashSet<PathBuf>>,
    #[cfg(mobile)]
    documents: Mutex<std::collections::HashMap<String, tauri_plugin_dialog::FilePath>>,
}

fn refused(path: &Path) -> CommandError {
    CommandError::new("backend.errors.readSourceNotChosen")
        .with_param("path", path.display().to_string())
}

fn insert(set: &Mutex<HashSet<PathBuf>>, path: &Path) {
    if let (Ok(canonical), Ok(mut granted)) = (std::fs::canonicalize(path), set.lock()) {
        granted.insert(canonical);
    }
}

fn contains(set: &Mutex<HashSet<PathBuf>>, path: &Path) -> bool {
    set.lock().is_ok_and(|granted| granted.contains(path))
}

impl ReadGrants {
    pub fn grant_file(&self, path: &Path) {
        insert(&self.files, path);
    }

    pub fn grant_folder(&self, path: &Path) {
        insert(&self.folders, path);
    }

    #[cfg(mobile)]
    fn grant_document(&self, file: tauri_plugin_dialog::FilePath) -> Option<String> {
        let key = file.to_string();
        self.documents.lock().ok()?.insert(key.clone(), file);
        Some(key)
    }

    pub(super) async fn open_file(
        &self,
        app: &tauri::AppHandle,
        source: &str,
    ) -> CommandResult<(tokio::fs::File, Option<super::document_io::DocumentAccess>)> {
        #[cfg(desktop)]
        let _ = app;
        #[cfg(mobile)]
        {
            let document = self
                .documents
                .lock()
                .ok()
                .and_then(|grants| grants.get(source).cloned());
            if let Some(url) = document {
                let options = tauri_plugin_fs::OpenOptions::new().read(true).to_owned();
                let (file, access) = super::document_io::open_document(app, url, options).await?;
                return Ok((file, Some(access)));
            }
        }
        let file = tokio::fs::File::open(self.file(Path::new(source))?)
            .await
            .map_err(|error| error.to_string())?;
        Ok((file, None))
    }

    /// The canonical path of a granted file, or a refusal.
    pub fn file(&self, path: &Path) -> CommandResult<PathBuf> {
        let canonical = std::fs::canonicalize(path).map_err(|_| refused(path))?;
        if contains(&self.files, &canonical) {
            Ok(canonical)
        } else {
            Err(refused(path))
        }
    }

    /// The canonical path of a granted folder, or a refusal.
    pub fn folder(&self, path: &Path) -> CommandResult<PathBuf> {
        let canonical = std::fs::canonicalize(path).map_err(|_| refused(path))?;
        if contains(&self.folders, &canonical) {
            Ok(canonical)
        } else {
            Err(refused(path))
        }
    }

    /// A granted folder or an entry directly inside one, as `list_backups` lists them.
    pub fn folder_or_entry(&self, path: &Path) -> CommandResult<PathBuf> {
        let canonical = std::fs::canonicalize(path).map_err(|_| refused(path))?;
        let granted = contains(&self.folders, &canonical)
            || canonical
                .parent()
                .is_some_and(|parent| contains(&self.folders, parent));
        if granted {
            Ok(canonical)
        } else {
            Err(refused(path))
        }
    }
}

/// An open dialog run by the backend; the picked file is what the import
/// commands accept afterwards.
#[tauri::command]
pub async fn choose_open_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    filter_name: String,
    extensions: Vec<String>,
) -> CommandResult<Option<String>> {
    let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let Some(chosen) = app
        .dialog()
        .file()
        .add_filter(&filter_name, &extensions)
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    #[cfg(mobile)]
    if matches!(chosen, tauri_plugin_dialog::FilePath::Url(_)) {
        return Ok(state.read_grants.grant_document(chosen));
    }
    let path = chosen.into_path().map_err(|error| error.to_string())?;
    state.read_grants.grant_file(&path);
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::ReadGrants;

    #[test]
    fn only_a_granted_file_is_readable() {
        let directory = tempfile::tempdir().unwrap();
        let chosen = directory.path().join("mail.mbox");
        let other = directory.path().join("other.mbox");
        std::fs::write(&chosen, b"").unwrap();
        std::fs::write(&other, b"").unwrap();
        let grants = ReadGrants::default();

        assert!(grants.file(&chosen).is_err(), "nothing chosen yet");
        grants.grant_file(&chosen);
        assert!(grants.file(&chosen).is_ok());
        assert!(grants.file(&chosen).is_ok(), "a read does not spend it");
        assert!(grants.file(&other).is_err());
        assert!(grants
            .file(&directory.path().join(".").join("mail.mbox"))
            .is_ok());
        assert_eq!(
            grants.file(&other).unwrap_err().key,
            "backend.errors.readSourceNotChosen"
        );
    }

    #[test]
    fn a_granted_folder_covers_its_direct_entries_only() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("satchel-backup-1");
        let nested = backup.join("inner");
        std::fs::create_dir_all(&nested).unwrap();
        let grants = ReadGrants::default();

        assert!(grants.folder(root.path()).is_err());
        assert!(grants.folder_or_entry(&backup).is_err());
        grants.grant_folder(root.path());
        assert!(grants.folder(root.path()).is_ok());
        assert!(grants.folder_or_entry(&backup).is_ok());
        assert!(grants.folder_or_entry(&nested).is_err());
        assert!(grants.folder(&backup).is_err());
        assert!(
            grants.file(root.path()).is_err(),
            "a folder is not a file grant"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_a_granted_folder_is_not_inside_it() {
        let granted = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = elsewhere.path().join("satchel-backup-1");
        std::fs::create_dir(&target).unwrap();
        let link = granted.path().join("satchel-backup-1");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let grants = ReadGrants::default();
        grants.grant_folder(granted.path());

        assert!(grants.folder_or_entry(&link).is_err());
    }
}
