use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the real build script with fixture credentials, without Cargo's cache or
/// OAuth configuration from the developer's shell.
struct BuildFixture {
    _directory: tempfile::TempDir,
    main: PathBuf,
    worktree: PathBuf,
    script: PathBuf,
}

impl BuildFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let main = root.join("main checkout");
        let worktree = root.join("mobile worktree");
        std::fs::create_dir_all(main.join(".git/worktrees/mobile")).unwrap();
        std::fs::create_dir_all(worktree.join("crates/satchel-core")).unwrap();
        let source = directory.path().join("build.rs");
        std::fs::write(&source, include_str!("../build.rs")).unwrap();
        let script = directory
            .path()
            .join(format!("build-script{}", std::env::consts::EXE_SUFFIX));
        let output = Command::new("rustc")
            .args(["--edition=2021", "--crate-name=oauth_build_fixture"])
            .arg(source)
            .arg("-o")
            .arg(&script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            _directory: directory,
            main,
            worktree,
            script,
        }
    }

    fn link_worktree(&self, gitdir: &Path) {
        std::fs::write(
            self.worktree.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();
    }

    fn run(&self, overrides: &[(&str, &str)]) -> String {
        let mut command = Command::new(&self.script);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("SATCHEL_") {
                command.env_remove(key);
            }
        }
        let output = command
            .env(
                "CARGO_MANIFEST_DIR",
                self.worktree.join("crates/satchel-core"),
            )
            .env("CARGO_CFG_TARGET_OS", "android")
            .envs(overrides.iter().copied())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

#[test]
fn android_worktree_bakes_main_checkout_client_identity() {
    let fixture = BuildFixture::new();
    fixture.link_worktree(&fixture.main.join(".git/worktrees/mobile"));
    std::fs::write(
        fixture.main.join(".env"),
        "SATCHEL_GMAIL_CLIENT_ID=fixture-google-client\nSATCHEL_OUTLOOK_CLIENT_ID=fixture-outlook-client\n",
    )
    .unwrap();

    let output = fixture.run(&[]);
    assert!(output.contains("cargo:rustc-env=SATCHEL_GMAIL_CLIENT_ID=fixture-google-client\n"));
    assert!(output.contains("cargo:rustc-env=SATCHEL_OUTLOOK_CLIENT_ID=fixture-outlook-client\n"));
    assert!(output.contains(&format!(
        "cargo:rerun-if-changed={}\n",
        fixture.main.join(".env").display()
    )));
}

#[test]
fn worktree_git_pointer_can_be_relative() {
    let fixture = BuildFixture::new();
    fixture.link_worktree(Path::new("../main checkout/.git/worktrees/mobile"));
    std::fs::write(
        fixture.main.join(".env"),
        "SATCHEL_GMAIL_CLIENT_ID=fixture-google-client\n",
    )
    .unwrap();

    assert!(fixture
        .run(&[])
        .contains("cargo:rustc-env=SATCHEL_GMAIL_CLIENT_ID=fixture-google-client\n"));
}

#[test]
fn worktree_env_overrides_main_checkout_and_is_watched_when_missing() {
    let fixture = BuildFixture::new();
    fixture.link_worktree(&fixture.main.join(".git/worktrees/mobile"));
    std::fs::write(
        fixture.main.join(".env"),
        "SATCHEL_GMAIL_CLIENT_ID=main-client\n",
    )
    .unwrap();
    let output = fixture.run(&[]);
    // Creating a worktree-local .env must invalidate Cargo's cached build.
    assert!(output.contains(&format!(
        "cargo:rerun-if-changed={}\n",
        fixture.worktree.join(".env").display()
    )));

    std::fs::write(
        fixture.worktree.join(".env"),
        "SATCHEL_GMAIL_CLIENT_ID=local-client\n",
    )
    .unwrap();
    let output = fixture.run(&[]);
    assert!(output.contains("cargo:rustc-env=SATCHEL_GMAIL_CLIENT_ID=local-client\n"));
    assert!(!output.contains("cargo:rustc-env=SATCHEL_GMAIL_CLIENT_ID=main-client\n"));
}

#[test]
fn shell_identity_overrides_file_without_baking_client_secrets() {
    let fixture = BuildFixture::new();
    std::fs::write(
        fixture.worktree.join(".env"),
        "SATCHEL_GMAIL_CLIENT_ID=file-client\nSATCHEL_GMAIL_CLIENT_SECRET=fixture-secret\nSATCHEL_GMAIL_REDIRECT_URI=de.tbuck.satchel://oauth/callback\n",
    )
    .unwrap();
    let output = fixture.run(&[
        ("SATCHEL_GMAIL_CLIENT_ID", "shell-client"),
        ("SATCHEL_OUTLOOK_CLIENT_SECRET", "fixture-secret"),
    ]);
    assert!(output.contains("cargo:rustc-env=SATCHEL_GMAIL_CLIENT_ID=shell-client\n"));
    assert!(output.contains(
        "cargo:rustc-env=SATCHEL_GMAIL_REDIRECT_URI=de.tbuck.satchel://oauth/callback\n"
    ));
    assert!(!output.contains("fixture-secret"));
}

#[test]
fn checkout_without_configuration_does_not_invent_a_client_identity() {
    let fixture = BuildFixture::new();
    std::fs::create_dir(fixture.worktree.join(".git")).unwrap();
    assert!(!fixture.run(&[]).contains("cargo:rustc-env=SATCHEL_"));
}
