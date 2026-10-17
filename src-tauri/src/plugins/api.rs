use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/plugins/api/"))]
pub struct PluginHostContext {
    pub account_id: Option<String>,
    pub active_thread_id: Option<String>,
}

pub trait PluginApi: Send + Sync {
    fn get_thread_snippet(&self, thread_id: &str) -> Result<String, String>;
    fn notify(&self, title: &str, body: &str) -> Result<(), String>;
}
