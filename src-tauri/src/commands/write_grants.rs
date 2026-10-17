//! Where the export and backup commands may write.
//!
//! They took any path the webview named, so a compromised or buggy frontend
//! could overwrite `~/.zshrc` with an mbox. A path the user picked in a save
//! dialog cannot be told apart from one JavaScript made up — unless the dialog
//! is opened here. These commands open it, and the path they return is the
//! only one the next write accepts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri_plugin_dialog::DialogExt;

use crate::commands::error::{CommandError, CommandResult};

#[derive(Default)]
pub struct WriteGrants {
    granted: Mutex<HashSet<PathBuf>>,
}

/// The path with symlinks resolved; a file that does not exist yet is resolved
/// through its directory.
pub fn canonical_target(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return std::fs::canonicalize(path);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    Ok(std::fs::canonicalize(parent)?.join(name))
}

impl WriteGrants {
    pub fn grant(&self, path: &Path) {
        if let (Ok(canonical), Ok(mut granted)) = (canonical_target(path), self.granted.lock()) {
            granted.insert(canonical);
        }
    }

    /// Consumes the grant for `path`, or refuses the write.
    pub fn take(&self, path: &Path) -> CommandResult<PathBuf> {
        let refused = || {
            CommandError::new("backend.errors.writeDestinationNotChosen")
                .with_param("path", path.display().to_string())
        };
        let canonical = canonical_target(path).map_err(|_| refused())?;
        let mut granted = self.granted.lock().map_err(|_| refused())?;
        if granted.remove(&canonical) {
            Ok(canonical)
        } else {
            Err(refused())
        }
    }
}

#[tauri::command]
pub async fn choose_save_path(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    suggested_name: String,
    filter_name: String,
    extensions: Vec<String>,
) -> CommandResult<Option<String>> {
    let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let chosen = app
        .dialog()
        .file()
        .set_file_name(&suggested_name)
        .add_filter(&filter_name, &extensions)
        .blocking_save_file();
    Ok(granted_path(&state, chosen))
}

#[tauri::command]
pub async fn choose_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    default_path: Option<String>,
) -> CommandResult<Option<String>> {
    #[cfg(desktop)]
    {
        let mut dialog = app.dialog().file();
        if let Some(default_path) = default_path.filter(|path| !path.is_empty()) {
            dialog = dialog.set_directory(default_path);
        }
        Ok(granted_path(&state, dialog.blocking_pick_folder()))
    }
    #[cfg(mobile)]
    {
        let _ = (app, state, default_path);
        Err(CommandError::new(
            "backend.errors.directorySelectionUnavailable",
        ))
    }
}

fn granted_path(
    state: &crate::AppState,
    chosen: Option<tauri_plugin_dialog::FilePath>,
) -> Option<String> {
    let path = chosen?.into_path().ok()?;
    state.write_grants.grant(&path);
    Some(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::WriteGrants;

    #[test]
    fn only_a_chosen_destination_is_writable_and_only_once() {
        let directory = tempfile::tempdir().unwrap();
        let chosen = directory.path().join("export.mbox");
        let grants = WriteGrants::default();

        assert!(grants.take(&chosen).is_err(), "nothing chosen yet");
        grants.grant(&chosen);

        assert!(grants.take(&directory.path().join("other.mbox")).is_err());
        assert!(grants.take(&chosen).is_ok());
        assert!(
            grants.take(&chosen).is_err(),
            "a grant is spent by the write"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_chosen_directory_is_not_the_chosen_file() {
        let chosen_dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let link = chosen_dir.path().join("link");
        std::os::unix::fs::symlink(elsewhere.path(), &link).unwrap();
        let grants = WriteGrants::default();
        grants.grant(&chosen_dir.path().join("export.mbox"));

        assert!(grants.take(&link.join("export.mbox")).is_err());
    }

    #[test]
    fn a_granted_path_matches_however_it_is_spelled() {
        let directory = tempfile::tempdir().unwrap();
        let grants = WriteGrants::default();
        grants.grant(&directory.path().join("export.json"));

        let spelled = directory.path().join(".").join("export.json");

        assert!(grants.take(&spelled).is_ok());
    }
}
