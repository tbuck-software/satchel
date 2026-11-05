//! Satchel rules and the vacation reply as a Sieve script on the IMAP server
//! (ManageSieve), so they run while the app is closed.
//!
//! The person's own server setup is never destroyed. Before anything is
//! written the active script is read back; another active script is either
//! kept running through a small `include` wrapper or replaced — both only on
//! an explicit choice. Without one the push stops at [`PushOutcome::NeedsConsent`]
//! and the rules keep running locally.

pub mod compiler;
pub mod service;

use serde::{Deserialize, Serialize};

use crate::providers::managesieve::{ManageSieveClient, ScriptInfo, SieveError};
use compiler::{is_satchel_script, wrapped_script_name, wrapper_script, CompiledScript};

/// The script Satchel owns.
pub const OWN_SCRIPT: &str = "satchel";
/// The wrapper that runs Satchel's script and the one that was active before.
pub const WRAPPER_SCRIPT: &str = "satchel-main";

/// What to do when another script is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationChoice {
    /// Never take over: stop and ask. Background pushes always use this.
    Ask,
    /// Activate a wrapper that includes both scripts (needs `include`).
    Wrapper,
    /// Activate Satchel's script instead; the other one stops running.
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    /// `satchel` itself is the active script.
    Direct,
    /// `satchel-main` is active and includes `satchel` and the other script.
    Wrapper,
}

impl ActivationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Wrapper => "wrapper",
        }
    }
}

#[derive(Debug)]
pub enum PushOutcome {
    Activated {
        mode: ActivationMode,
        /// The script that was active before Satchel's, restored on disable.
        previous_active: Option<String>,
        warnings: Option<String>,
    },
    /// Another script is active and no choice was made; nothing was written.
    NeedsConsent {
        active_script: String,
        include_available: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error(transparent)]
    Sieve(#[from] SieveError),
    /// A script by one of Satchel's names exists and is not Satchel's.
    #[error("a foreign script is named {0}")]
    NameTaken(String),
    #[error("the server lacks the include extension")]
    IncludeUnavailable,
    #[error("the server reports {found:?} active after the push")]
    Unverified { found: Option<String> },
}

impl PushError {
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Sieve(error) => error.message_key(),
            Self::NameTaken(_) => "backend.errors.serverRules.nameTaken",
            Self::IncludeUnavailable => "backend.errors.serverRules.includeUnavailable",
            Self::Unverified { .. } => "backend.errors.serverRules.unverified",
        }
    }
}

fn active_name(scripts: &[ScriptInfo]) -> Option<String> {
    scripts
        .iter()
        .find(|script| script.active)
        .map(|script| script.name.clone())
}

fn exists(scripts: &[ScriptInfo], name: &str) -> bool {
    scripts.iter().any(|script| script.name == name)
}

/// A script named `name` exists and is Satchel's. `Err(NameTaken)` when the
/// name is taken by someone else's script.
async fn owned(
    client: &mut ManageSieveClient,
    scripts: &[ScriptInfo],
    name: &str,
) -> Result<Option<String>, PushError> {
    if !exists(scripts, name) {
        return Ok(None);
    }
    let content = client.get_script(name).await?;
    if is_satchel_script(&content) {
        Ok(Some(content))
    } else {
        Err(PushError::NameTaken(name.to_string()))
    }
}

enum Plan {
    Direct { previous: Option<String> },
    Wrapper { other: String },
}

/// Write `compiled` as `satchel` and make sure it runs, honouring whatever
/// else the person has active. `previous_active` is what an earlier push
/// recorded, so a re-push keeps knowing what to restore.
pub async fn push(
    client: &mut ManageSieveClient,
    compiled: &CompiledScript,
    choice: ActivationChoice,
    previous_active: Option<String>,
) -> Result<PushOutcome, PushError> {
    let include_available = client.capabilities().has_extension("include");
    let scripts = client.list_scripts().await?;
    owned(client, &scripts, OWN_SCRIPT).await?;
    let active = active_name(&scripts);

    let plan = match active.as_deref() {
        None => Plan::Direct { previous: None },
        Some(OWN_SCRIPT) => Plan::Direct {
            previous: previous_active.filter(|name| exists(&scripts, name)),
        },
        Some(name) => {
            // Our own wrapper, still pointing at a script that exists: keep it.
            let wrapped = if name == WRAPPER_SCRIPT {
                let content = client.get_script(WRAPPER_SCRIPT).await?;
                wrapped_script_name(&content, OWN_SCRIPT).filter(|other| exists(&scripts, other))
            } else {
                None
            };
            match (wrapped, choice) {
                (Some(other), _) => Plan::Wrapper { other },
                (None, ActivationChoice::Ask) => {
                    return Ok(PushOutcome::NeedsConsent {
                        active_script: name.to_string(),
                        include_available,
                    })
                }
                (None, ActivationChoice::Wrapper) => {
                    if !include_available {
                        return Err(PushError::IncludeUnavailable);
                    }
                    if name == WRAPPER_SCRIPT {
                        // Active, named like ours, but not ours.
                        return Err(PushError::NameTaken(WRAPPER_SCRIPT.to_string()));
                    }
                    owned(client, &scripts, WRAPPER_SCRIPT).await?;
                    Plan::Wrapper {
                        other: name.to_string(),
                    }
                }
                (None, ActivationChoice::Replace) => Plan::Direct {
                    previous: Some(name.to_string()),
                },
            }
        }
    };

    let warnings = store(client, OWN_SCRIPT, &compiled.content).await?;
    let (mode, previous, expected) = match plan {
        Plan::Direct { previous } => (ActivationMode::Direct, previous, OWN_SCRIPT),
        Plan::Wrapper { other } => {
            let wrapper = wrapper_script(OWN_SCRIPT, &other);
            let current = if exists(&scripts, WRAPPER_SCRIPT) {
                Some(client.get_script(WRAPPER_SCRIPT).await?)
            } else {
                None
            };
            if current.as_deref() != Some(wrapper.as_str()) {
                store(client, WRAPPER_SCRIPT, &wrapper).await?;
            }
            (ActivationMode::Wrapper, Some(other), WRAPPER_SCRIPT)
        }
    };
    if active.as_deref() != Some(expected) {
        client.set_active(expected).await?;
    }

    // Read back: a push only counts once the server says it is active.
    let found = active_name(&client.list_scripts().await?);
    if found.as_deref() != Some(expected) {
        return Err(PushError::Unverified { found });
    }
    Ok(PushOutcome::Activated {
        mode,
        previous_active: previous,
        warnings,
    })
}

/// HAVESPACE, CHECKSCRIPT where the server knows it, then PUTSCRIPT.
async fn store(
    client: &mut ManageSieveClient,
    name: &str,
    content: &str,
) -> Result<Option<String>, PushError> {
    client.have_space(name, content.len()).await?;
    if client.capabilities().version.is_some() {
        client.check_script(content).await?;
    }
    Ok(client.put_script(name, content).await?)
}

/// Stop running Satchel's script: re-activate what was active before (or
/// nothing), then delete Satchel's own scripts. Foreign scripts are never
/// touched, and a script by our name that is not ours stays.
pub async fn remove(
    client: &mut ManageSieveClient,
    previous_active: Option<&str>,
) -> Result<(), PushError> {
    let scripts = client.list_scripts().await?;
    let own = owned(client, &scripts, OWN_SCRIPT).await.ok().flatten();
    let wrapper = owned(client, &scripts, WRAPPER_SCRIPT).await.ok().flatten();
    let active = active_name(&scripts);
    let ours_active = match active.as_deref() {
        Some(OWN_SCRIPT) => own.is_some(),
        Some(WRAPPER_SCRIPT) => wrapper.is_some(),
        _ => false,
    };
    if ours_active {
        let restore = previous_active
            .filter(|name| exists(&scripts, name) && *name != OWN_SCRIPT && *name != WRAPPER_SCRIPT)
            .unwrap_or("");
        client.set_active(restore).await?;
    }
    if wrapper.is_some() {
        client.delete_script(WRAPPER_SCRIPT).await?;
    }
    if own.is_some() {
        client.delete_script(OWN_SCRIPT).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "push_cases.rs"]
mod tests;
