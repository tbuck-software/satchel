use serde::{Deserialize, Serialize};

pub use wit_bindgen;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub subject: String,
    pub workflow_state: String,
}

pub trait Plugin {
    fn manifest_json(&self) -> String;
    fn run_command(&self, command_id: &str, payload: &str) -> Result<String, String>;
}
