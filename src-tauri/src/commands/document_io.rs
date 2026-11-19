//! Native document URLs are usable only after a backend-owned picker grants them.

use std::path::{Path, PathBuf};

#[cfg(mobile)]
use tauri_plugin_dialog::FilePath;
#[cfg(mobile)]
use tauri_plugin_fs::FsExt;

use tauri::Manager;

use crate::commands::error::CommandResult;

pub(super) fn temporary_directory(app: &tauri::AppHandle) -> CommandResult<tempfile::TempDir> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
    tempfile::tempdir_in(cache).map_err(|error| error.to_string().into())
}

pub(super) struct DocumentAccess {
    #[cfg(target_os = "ios")]
    app: tauri::AppHandle,
    #[cfg(target_os = "ios")]
    url: FilePath,
}

#[cfg(target_os = "ios")]
impl Drop for DocumentAccess {
    fn drop(&mut self) {
        let _ = self
            .app
            .fs()
            .stop_accessing_security_scoped_resource(self.url.clone());
    }
}

#[cfg(mobile)]
pub(super) async fn open_document(
    app: &tauri::AppHandle,
    url: FilePath,
    options: tauri_plugin_fs::OpenOptions,
) -> CommandResult<(tokio::fs::File, DocumentAccess)> {
    let app = app.clone();
    let (file, access) = tokio::task::spawn_blocking(move || {
        let file = app.fs().open(url.clone(), options);
        let access = DocumentAccess {
            #[cfg(target_os = "ios")]
            app,
            #[cfg(target_os = "ios")]
            url,
        };
        file.map(|file| (file, access))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok((tokio::fs::File::from_std(file), access))
}

/// Local exports keep their existing publication path. A document provider
/// gets the finished export copied to its URI, via its own writable descriptor.
pub(super) struct ExportDestination {
    path: PathBuf,
    #[cfg(mobile)]
    document: Option<(FilePath, tempfile::TempDir)>,
}

impl ExportDestination {
    pub fn local(path: PathBuf) -> Self {
        Self {
            path,
            #[cfg(mobile)]
            document: None,
        }
    }

    #[cfg(mobile)]
    pub fn document(app: &tauri::AppHandle, url: FilePath) -> CommandResult<Self> {
        let directory = temporary_directory(app)?;
        Ok(Self {
            path: directory.path().join("export"),
            document: Some((url, directory)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn publish(self, app: &tauri::AppHandle) -> CommandResult<()> {
        #[cfg(desktop)]
        let _ = app;
        #[cfg(mobile)]
        if let Some((url, _directory)) = self.document {
            use tokio::io::AsyncWriteExt;
            let mut input = tokio::fs::File::open(self.path)
                .await
                .map_err(|error| error.to_string())?;
            let (mut output, _access) = open_document(
                app,
                url,
                tauri_plugin_fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .to_owned(),
            )
            .await?;
            tokio::io::copy(&mut input, &mut output)
                .await
                .map_err(|error| error.to_string())?;
            output.flush().await.map_err(|error| error.to_string())?;
            drop(output);
        }
        Ok(())
    }
}
