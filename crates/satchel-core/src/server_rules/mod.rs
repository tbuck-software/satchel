//! Satchel rules and the vacation reply as a Sieve script on the IMAP server
//! (ManageSieve), so they run while the app is closed.
//!
//! The person's own server setup is never destroyed. Before anything is
//! written the active script is read back; another active script is either
//! kept running through a small `include` wrapper or replaced — both only on
//! an explicit choice. Without one the push stops at [`PushOutcome::NeedsConsent`]
//! and the rules keep running locally.
//!
//! The server script is an addition, not a replacement: every rule keeps
//! running in Satchel too, so it also fires while the app is closed. That is
//! safe because every action compiled today (`mark_read`) is idempotent. An
//! action that is not (`fileinto`, `redirect`) must not be added before the
//! local engine has an exact gate for skipping what the server already did:
//! a content hash of the active script, a read-back verification and a safety
//! margin around the moment it went live.

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

/// Owned or absent: `None` also when the name belongs to someone else.
/// Any other failure (a timeout, a dropped connection) propagates; reading
/// it as "not ours" would report a removal that never happened.
async fn owned_or_none(
    client: &mut ManageSieveClient,
    scripts: &[ScriptInfo],
    name: &str,
) -> Result<Option<String>, PushError> {
    match owned(client, scripts, name).await {
        Err(PushError::NameTaken(_)) => Ok(None),
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Plan {
    /// `satchel` becomes (or stays) the active script.
    Direct { previous: Option<String> },
    /// `satchel-main` includes `satchel`, then `other`.
    Wrapper { other: String },
}

/// What a push will do, decided from what the server holds right now. Nothing
/// has been written yet: the caller records [`PlannedPush::mode`] and
/// [`PlannedPush::previous_active`] first, so a push that breaks off after
/// SETACTIVE still knows what to restore.
#[derive(Debug)]
pub struct PlannedPush {
    plan: Plan,
    active: Option<String>,
    /// Satchel's own `satchel-main`, if one exists.
    own_wrapper: Option<String>,
}

impl PlannedPush {
    pub fn mode(&self) -> ActivationMode {
        match self.plan {
            Plan::Direct { .. } => ActivationMode::Direct,
            Plan::Wrapper { .. } => ActivationMode::Wrapper,
        }
    }

    /// The script to re-activate when server rules are turned off.
    pub fn previous_active(&self) -> Option<String> {
        match &self.plan {
            Plan::Direct { previous } => previous.clone(),
            Plan::Wrapper { other } => Some(other.clone()),
        }
    }
}

#[derive(Debug)]
pub enum Planned {
    Ready(PlannedPush),
    /// Another script is active and no matching choice was made.
    NeedsConsent {
        active_script: String,
        include_available: bool,
    },
}

/// Read the server's scripts and decide how Satchel's script will run.
///
/// `previous_active` is what an earlier push recorded. `expected_active` is
/// the script the person saw when they chose `Wrapper` or `Replace`: if
/// another one is active by now, the question is asked again.
pub async fn plan(
    client: &mut ManageSieveClient,
    choice: ActivationChoice,
    previous_active: Option<String>,
    expected_active: Option<&str>,
) -> Result<Planned, PushError> {
    let include_available = client.capabilities().has_extension("include");
    let scripts = client.list_scripts().await?;
    owned(client, &scripts, OWN_SCRIPT).await?;
    let active = active_name(&scripts);
    let wrapper_content = if exists(&scripts, WRAPPER_SCRIPT) {
        Some(client.get_script(WRAPPER_SCRIPT).await?)
    } else {
        None
    };
    let own_wrapper = wrapper_content.filter(|content| is_satchel_script(content));
    let foreign_wrapper = exists(&scripts, WRAPPER_SCRIPT) && own_wrapper.is_none();
    let is_ours = |name: &str| name == OWN_SCRIPT || name == WRAPPER_SCRIPT;

    let plan = match active.as_deref() {
        None => Plan::Direct { previous: None },
        Some(OWN_SCRIPT) => Plan::Direct {
            previous: previous_active.filter(|name| exists(&scripts, name) && !is_ours(name)),
        },
        Some(WRAPPER_SCRIPT) if own_wrapper.is_some() => {
            let wrapped = own_wrapper
                .as_deref()
                .and_then(|content| wrapped_script_name(content, OWN_SCRIPT))
                .filter(|other| exists(&scripts, other) && !is_ours(other));
            match wrapped {
                Some(other) => Plan::Wrapper { other },
                // Satchel's own wrapper around a script that is gone: it is
                // Satchel's to replace, and there is nothing left to keep.
                None => Plan::Direct { previous: None },
            }
        }
        Some(name) => {
            let consent = || Planned::NeedsConsent {
                active_script: name.to_string(),
                include_available,
            };
            if choice == ActivationChoice::Ask || expected_active != Some(name) {
                return Ok(consent());
            }
            // A foreign script named like Satchel's wrapper can neither be
            // wrapped nor replaced: turning off would mistake it for ours.
            if name == WRAPPER_SCRIPT || foreign_wrapper {
                return Err(PushError::NameTaken(WRAPPER_SCRIPT.to_string()));
            }
            match choice {
                ActivationChoice::Wrapper if !include_available => {
                    return Err(PushError::IncludeUnavailable)
                }
                ActivationChoice::Wrapper => Plan::Wrapper {
                    other: name.to_string(),
                },
                _ => Plan::Direct {
                    previous: Some(name.to_string()),
                },
            }
        }
    };
    Ok(Planned::Ready(PlannedPush {
        plan,
        active,
        own_wrapper,
    }))
}

/// Write `compiled` as `satchel` and activate the planned script, then read
/// the active script back. Returns the server's warnings.
pub async fn apply(
    client: &mut ManageSieveClient,
    planned: &PlannedPush,
    compiled: &CompiledScript,
) -> Result<Option<String>, PushError> {
    let warnings = store(client, OWN_SCRIPT, &compiled.content).await?;
    let expected = match &planned.plan {
        Plan::Direct { .. } => OWN_SCRIPT,
        Plan::Wrapper { other } => {
            let wrapper = wrapper_script(OWN_SCRIPT, other);
            if planned.own_wrapper.as_deref() != Some(wrapper.as_str()) {
                store(client, WRAPPER_SCRIPT, &wrapper).await?;
            }
            WRAPPER_SCRIPT
        }
    };
    if planned.active.as_deref() != Some(expected) {
        client.set_active(expected).await?;
    }
    // A wrapper of ours that is no longer used (inactive now) goes away.
    if expected == OWN_SCRIPT && planned.own_wrapper.is_some() {
        client.delete_script(WRAPPER_SCRIPT).await?;
    }

    // Read back: a push only counts once the server says it is active.
    let found = active_name(&client.list_scripts().await?);
    if found.as_deref() != Some(expected) {
        return Err(PushError::Unverified { found });
    }
    Ok(warnings)
}

/// [`plan`] and [`apply`] in one go, for callers with nothing to record in
/// between.
pub async fn push(
    client: &mut ManageSieveClient,
    compiled: &CompiledScript,
    choice: ActivationChoice,
    previous_active: Option<String>,
    expected_active: Option<&str>,
) -> Result<PushOutcome, PushError> {
    match plan(client, choice, previous_active, expected_active).await? {
        Planned::NeedsConsent {
            active_script,
            include_available,
        } => Ok(PushOutcome::NeedsConsent {
            active_script,
            include_available,
        }),
        Planned::Ready(planned) => {
            let warnings = apply(client, &planned, compiled).await?;
            Ok(PushOutcome::Activated {
                mode: planned.mode(),
                previous_active: planned.previous_active(),
                warnings,
            })
        }
    }
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
/// touched, and a script by our name that is not ours stays. Safe to repeat.
pub async fn remove(
    client: &mut ManageSieveClient,
    previous_active: Option<&str>,
) -> Result<(), PushError> {
    let scripts = client.list_scripts().await?;
    let own = owned_or_none(client, &scripts, OWN_SCRIPT).await?;
    let wrapper = owned_or_none(client, &scripts, WRAPPER_SCRIPT).await?;
    let active = active_name(&scripts);
    let ours_active = match active.as_deref() {
        Some(OWN_SCRIPT) => own.is_some(),
        Some(WRAPPER_SCRIPT) => wrapper.is_some(),
        _ => false,
    };
    if ours_active {
        let valid =
            |name: &str| exists(&scripts, name) && name != OWN_SCRIPT && name != WRAPPER_SCRIPT;
        // Without a record, the wrapper itself names the script it kept running.
        let recovered = wrapper
            .as_deref()
            .filter(|_| active.as_deref() == Some(WRAPPER_SCRIPT))
            .and_then(|content| wrapped_script_name(content, OWN_SCRIPT));
        let restore = previous_active
            .map(str::to_string)
            .filter(|name| valid(name))
            .or(recovered.filter(|name| valid(name)))
            .unwrap_or_default();
        client.set_active(&restore).await?;
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
