use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub sync_interval_seconds: u64,
    pub enable_notifications: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sync_interval_seconds: 30,
            enable_notifications: true,
        }
    }
}
