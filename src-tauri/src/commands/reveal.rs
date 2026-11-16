//! Reveal a freshly saved file in the system file manager. Only a path the
//! user picked in a native dialog is revealed, and it is still checked against
//! the filesystem before anything launches.

use std::path::Path;

use tauri_plugin_fs::FsExt;

use crate::commands::error::CommandResult;

#[tauri::command]
pub fn show_in_folder(app: tauri::AppHandle, path: String) -> CommandResult<()> {
    // The dialog plugin adds every picked path to the fs scope; nothing else does.
    reveal_in_folder(&path, |candidate| {
        app.try_fs_scope()
            .is_some_and(|scope| scope.is_allowed(candidate))
    })
}

fn reveal_in_folder(path: &str, picked: impl Fn(&Path) -> bool) -> CommandResult<()> {
    let resolved = std::fs::canonicalize(path).map_err(|_| "path no longer exists")?;
    let target = resolved.as_path();
    if !picked(Path::new(path)) {
        return Err(crate::commands::error::CommandError::new(
            "backend.errors.revealTargetBlocked",
        ));
    }
    validate_reveal_target(target)?;
    if !target.exists() {
        return Err("path no longer exists".into());
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("open");
        // `open -R` selects the file in Finder; for a directory the user chose
        // as a save target, opening it directly is what they expect.
        if target.is_dir() {
            command.arg(target);
        } else {
            command.arg("-R").arg(target);
        }
        command
            .spawn()
            .map_err(|err| format!("open in Finder failed: {err}"))?;
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("explorer");
        if target.is_dir() {
            command.arg(target);
        } else {
            command.arg(format!("/select,{}", target.display()));
        }
        command
            .spawn()
            .map_err(|err| format!("open in Explorer failed: {err}"))?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // xdg-open has no "select" mode, so fall back to the containing folder.
        let folder = if target.is_dir() {
            target
        } else {
            target.parent().unwrap_or(target)
        };
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map_err(|err| format!("open folder failed: {err}"))?;
    }

    Ok(())
}

fn validate_reveal_target(target: &Path) -> CommandResult<()> {
    // An executable or a child of a bundle must never reach `open` as a directory.
    for ancestor in target.ancestors() {
        if ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let extension = Path::new(name)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                [
                    "bundle",
                    "framework",
                    "plugin",
                    "kext",
                    "prefpane",
                    "qlgenerator",
                    "mdimporter",
                    "saver",
                    "appex",
                ]
                .contains(&extension.as_str())
                    || crate::commands::attachments::attachment_open_risk(name).executable
            })
        {
            return Err(crate::commands::error::CommandError::new(
                "backend.errors.revealTargetBlocked",
            ));
        }
    }
    let metadata = std::fs::metadata(target).map_err(|_| "path no longer exists")?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(crate::commands::error::CommandError::new(
            "backend.errors.revealTargetBlocked",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
            return Err(crate::commands::error::CommandError::new(
                "backend.errors.revealTargetBlocked",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_bundles_executables_and_bundle_children() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["Fixture.app", "Fixture.bundle", "Fixture.prefPane"] {
            let bundle = dir.path().join(name);
            let child = bundle.join("Contents");
            std::fs::create_dir_all(&child).unwrap();
            assert!(super::validate_reveal_target(&bundle).is_err());
            assert!(super::validate_reveal_target(&child).is_err());
        }
        let executable = dir.path().join("run.exe");
        std::fs::write(&executable, b"binary").unwrap();
        assert!(super::validate_reveal_target(&executable).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_executable_bits_on_a_file_without_an_extension() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run");
        std::fs::write(&path, b"binary").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(super::validate_reveal_target(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(super::validate_reveal_target(&path).is_ok());
    }

    #[test]
    fn rejects_missing_paths() {
        let result = super::reveal_in_folder("/definitely/not/a/real/path/xyz", |_| true);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_a_path_no_dialog_picked_before_launching_anything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"text").unwrap();
        let error = super::reveal_in_folder(&path.to_string_lossy(), |_| false).unwrap_err();
        assert_eq!(error.key, "backend.errors.revealTargetBlocked");
    }
}
