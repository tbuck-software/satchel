#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/runtime_capabilities/")
)]
#[serde(rename_all = "lowercase")]
pub enum PlatformKind {
    Android,
    Ios,
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/runtime_capabilities/")
)]
pub struct RuntimeCapabilities {
    pub mobile: bool,
    pub tray: bool,
    pub autostart: bool,
    pub single_instance: bool,
    pub plugins: bool,
    pub native_crypto: bool,
    pub background_sync: bool,
    pub contacts: bool,
    pub file_associations: bool,
    pub deep_links: bool,
    pub push: bool,
}

pub fn capabilities_for(platform: PlatformKind) -> RuntimeCapabilities {
    let mobile = matches!(platform, PlatformKind::Android | PlatformKind::Ios);
    let desktop = !mobile;
    RuntimeCapabilities {
        mobile,
        tray: desktop,
        autostart: desktop,
        single_instance: desktop,
        plugins: desktop,
        native_crypto: true,
        background_sync: true,
        contacts: matches!(platform, PlatformKind::Macos),
        file_associations: desktop,
        deep_links: true,
        push: mobile,
    }
}

pub fn current_platform() -> PlatformKind {
    if cfg!(target_os = "android") {
        PlatformKind::Android
    } else if cfg!(target_os = "ios") {
        PlatformKind::Ios
    } else if cfg!(target_os = "macos") {
        PlatformKind::Macos
    } else if cfg!(target_os = "windows") {
        PlatformKind::Windows
    } else {
        PlatformKind::Linux
    }
}

#[cfg(test)]
mod tests {
    use super::{capabilities_for, PlatformKind};

    #[test]
    fn platform_names_keep_the_existing_wire_strings() {
        for (platform, name) in [
            (PlatformKind::Android, "android"),
            (PlatformKind::Ios, "ios"),
            (PlatformKind::Linux, "linux"),
            (PlatformKind::Macos, "macos"),
            (PlatformKind::Windows, "windows"),
        ] {
            assert_eq!(serde_json::to_value(platform).unwrap(), name);
        }
    }

    #[test]
    fn desktop_platforms_keep_desktop_integrations() {
        for platform in [
            PlatformKind::Linux,
            PlatformKind::Macos,
            PlatformKind::Windows,
        ] {
            let capabilities = capabilities_for(platform);
            assert!(capabilities.tray);
            assert!(capabilities.autostart);
            assert!(capabilities.single_instance);
            assert!(capabilities.file_associations);
            assert!(capabilities.native_crypto);
        }
    }

    #[test]
    fn mobile_platforms_do_not_claim_unimplemented_integrations() {
        for platform in [PlatformKind::Android, PlatformKind::Ios] {
            let capabilities = capabilities_for(platform);
            assert!(!capabilities.tray);
            assert!(!capabilities.autostart);
            assert!(!capabilities.single_instance);
            assert!(!capabilities.file_associations);
            assert!(capabilities.native_crypto);
        }
    }

    #[test]
    fn mobile_claims_push_and_background_sync_only_after_native_paths_exist() {
        for platform in [PlatformKind::Android, PlatformKind::Ios] {
            let mobile = capabilities_for(platform);
            assert!(mobile.native_crypto);
            assert!(mobile.push);
            assert!(mobile.background_sync);
        }
    }

    #[test]
    fn mobile_platforms_expose_mobile_shell() {
        assert!(capabilities_for(PlatformKind::Android).mobile);
        assert!(capabilities_for(PlatformKind::Ios).mobile);
        assert!(!capabilities_for(PlatformKind::Macos).mobile);
    }
}
