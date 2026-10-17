//! Tauri-free persistence for plugin capability grants.
use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct PluginPermissionUpsertRequest {
    pub plugin_id: String,
    pub capability: String,
    pub granted: bool,
}

pub type PluginPermissionGrants = HashMap<String, HashSet<String>>;

#[derive(Debug, Error)]
pub enum PluginPermissionError {
    #[error("plugin permission persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for PluginPermissionError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

/// Owns durable plugin permission rows while leaving capability parsing and
/// runtime grant policy to the app crate.
pub struct PluginPermissionRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> PluginPermissionRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Return only granted capabilities, grouped by plugin identifier.
    pub async fn list_granted(&self) -> Result<PluginPermissionGrants, PluginPermissionError> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT plugin_id, capability FROM plugin_permissions WHERE granted = 1",
        )
        .fetch_all(self.pool)
        .await?;

        let mut grants = HashMap::new();
        for (plugin_id, capability) in rows {
            grants
                .entry(plugin_id)
                .or_insert_with(HashSet::new)
                .insert(capability);
        }
        Ok(grants)
    }

    pub async fn upsert(
        &self,
        request: PluginPermissionUpsertRequest,
    ) -> Result<(), PluginPermissionError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO plugin_permissions (plugin_id, capability, granted, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(plugin_id, capability) DO UPDATE SET
                granted = excluded.granted,
                updated_at = excluded.updated_at",
        )
        .bind(request.plugin_id)
        .bind(request.capability)
        .bind(if request.granted { 1 } else { 0 })
        .bind(now)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
