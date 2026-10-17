//! Which build is running. The commit comes from `build.rs`.

use serde::Serialize;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("SATCHEL_GIT_SHA");

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/utils/build_info/", optional_fields = nullable))]
pub struct BuildInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub debug: bool,
}

pub fn current() -> BuildInfo {
    BuildInfo {
        version: VERSION,
        commit: COMMIT,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        debug: cfg!(debug_assertions),
    }
}

/// `0.1.0 (abc123def456) macos/aarch64`, for crash entries and reports.
pub fn summary() -> String {
    let info = current();
    let profile = if info.debug { " debug" } else { "" };
    format!(
        "{} ({}) {}/{}{profile}",
        info.version, info.commit, info.os, info.arch
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn summary_names_version_commit_and_platform() {
        let summary = super::summary();
        assert!(summary.starts_with(env!("CARGO_PKG_VERSION")), "{summary}");
        assert!(summary.contains(super::COMMIT), "{summary}");
        assert!(summary.contains(std::env::consts::OS), "{summary}");
        assert!(!super::COMMIT.is_empty());
    }
}
