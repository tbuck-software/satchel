//! Clear quarantine only on our own bundle after a verified update replaced it.
//!
//! LSFileQuarantineEnabled must stay enabled for downloaded mail attachments.
//! It also marks the updater's extracted bundle, which would otherwise restart
//! in App Translocation and prevent the next update from writing to the app.

use std::{
    io,
    path::{Path, PathBuf},
};

pub(super) fn own_bundle_path(executable: &Path) -> io::Result<PathBuf> {
    let macos = executable.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let contents = macos.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let bundle = contents.parent().ok_or(io::ErrorKind::InvalidInput)?;
    if !executable.is_absolute()
        || macos.file_name().is_none_or(|name| name != "MacOS")
        || contents.file_name().is_none_or(|name| name != "Contents")
        || bundle
            .extension()
            .is_none_or(|extension| extension != "app")
        || executable.components().any(|component| {
            component == std::path::Component::ParentDir
                || component.as_os_str() == "AppTranslocation"
        })
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(bundle.to_path_buf())
}

/// Never follow bundle symlinks into the system or a user's mail directory.
fn visit_bundle(bundle: &Path, action: &mut impl FnMut(&Path) -> io::Result<()>) -> io::Result<()> {
    let root = std::fs::symlink_metadata(bundle)?;
    if !root.is_dir()
        || root.is_symlink()
        || bundle
            .extension()
            .is_none_or(|extension| extension != "app")
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    fn visit(path: &Path, action: &mut impl FnMut(&Path) -> io::Result<()>) -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.is_symlink() {
            return Ok(());
        }
        action(path)?;
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path)? {
                visit(&entry?.path(), action)?;
            }
        }
        Ok(())
    }
    visit(bundle, action)
}

#[cfg(target_os = "macos")]
pub(super) fn clear_update_quarantine(bundle: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    visit_bundle(bundle, &mut |path| {
        let path = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: both strings are NUL-terminated and valid for the call.
        // NOFOLLOW also protects against a symlink substituted after metadata.
        let result = unsafe {
            libc::removexattr(
                path.as_ptr(),
                c"com.apple.quarantine".as_ptr(),
                libc::XATTR_NOFOLLOW,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOATTR) {
            Ok(())
        } else {
            Err(error)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_normally_installed_app_bundle_can_be_updated() {
        assert_eq!(
            own_bundle_path(Path::new(
                "/Applications/Satchel.app/Contents/MacOS/satchel"
            ))
            .unwrap(),
            Path::new("/Applications/Satchel.app")
        );
        for invalid in [
            "/tmp/satchel",
            "/Applications/Satchel.app/Resources/MacOS/satchel",
            "Satchel.app/Contents/MacOS/satchel",
            "/Applications/Other/Contents/MacOS/satchel",
            "/private/var/folders/AppTranslocation/id/d/Satchel.app/Contents/MacOS/satchel",
            "/Applications/../Satchel.app/Contents/MacOS/satchel",
        ] {
            assert!(own_bundle_path(Path::new(invalid)).is_err(), "{invalid}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn traversal_includes_the_bundle_but_never_follows_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Satchel.app");
        let contents = bundle.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        let executable = contents.join("satchel");
        std::fs::write(&executable, b"verified update fixture").unwrap();
        let attachment = temp.path().join("mail-attachment");
        std::fs::write(&attachment, b"untrusted mail attachment").unwrap();
        std::os::unix::fs::symlink(&attachment, contents.join("outside")).unwrap();
        let mut visited = Vec::new();
        visit_bundle(&bundle, &mut |path| {
            visited.push(path.to_path_buf());
            Ok(())
        })
        .unwrap();
        visited.sort();
        let mut expected = vec![bundle.clone(), contents, executable];
        expected.sort();
        assert_eq!(visited, expected);
        assert!(visit_bundle(&attachment, &mut |_| panic!(
            "must reject before touching file"
        ))
        .is_err());
        let alias = temp.path().join("Alias.app");
        std::os::unix::fs::symlink(&bundle, &alias).unwrap();
        assert!(visit_bundle(&alias, &mut |_| panic!("must reject root symlink")).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clearing_update_quarantine_keeps_mail_attachments_quarantined() {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("Satchel.app");
        std::fs::create_dir(&bundle).unwrap();
        let executable = bundle.join("satchel");
        let attachment = temp.path().join("attachment");
        for file in [&executable, &attachment] {
            std::fs::write(file, b"fixture").unwrap();
        }
        std::os::unix::fs::symlink(&attachment, bundle.join("attachment-link")).unwrap();
        for path in [&bundle, &executable, &attachment] {
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let value = b"0081;fixture;Satchel;";
            assert_eq!(
                unsafe {
                    libc::setxattr(
                        path.as_ptr(),
                        c"com.apple.quarantine".as_ptr(),
                        value.as_ptr().cast(),
                        value.len(),
                        0,
                        libc::XATTR_NOFOLLOW,
                    )
                },
                0
            );
        }
        clear_update_quarantine(&bundle).unwrap();
        clear_update_quarantine(&bundle).unwrap();
        for (file, quarantined) in [(&bundle, false), (&executable, false), (&attachment, true)] {
            let path = CString::new(file.as_os_str().as_bytes()).unwrap();
            let size = unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    c"com.apple.quarantine".as_ptr(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    libc::XATTR_NOFOLLOW,
                )
            };
            assert_eq!(size >= 0, quarantined, "{}", file.display());
        }
    }
}
