use crate::runtime_capabilities::{
    capabilities_for, current_platform, PlatformKind, RuntimeCapabilities,
};

#[derive(serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/runtime/"))]
pub struct RuntimeCapabilitiesDto {
    pub platform: PlatformKind,
    #[serde(flatten)]
    pub capabilities: RuntimeCapabilities,
}

#[tauri::command]
pub fn runtime_capabilities() -> RuntimeCapabilitiesDto {
    let platform = current_platform();
    RuntimeCapabilitiesDto {
        platform,
        capabilities: capabilities_for(platform),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_capabilities_keep_the_existing_flat_wire_shape() {
        let value = serde_json::to_value(runtime_capabilities()).unwrap();
        let fields = value.as_object().unwrap();
        let mut names = fields.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "autostart",
                "background_sync",
                "contacts",
                "deep_links",
                "file_associations",
                "mobile",
                "native_crypto",
                "platform",
                "plugins",
                "push",
                "single_instance",
                "tray",
            ]
        );
        assert!(fields
            .iter()
            .all(|(key, value)| key == "platform" || value.is_boolean()));
        assert_eq!(
            value["platform"],
            format!("{:?}", current_platform()).to_ascii_lowercase()
        );
        assert!(value.get("capabilities").is_none());
    }
}
