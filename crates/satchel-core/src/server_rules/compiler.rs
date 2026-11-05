//! Turns Satchel rules and the vacation reply into one Sieve script.
//!
//! Only what Sieve can express *the same way the local engine does* is
//! compiled. A rule whose condition or action has no faithful Sieve form, or
//! needs an extension the server does not advertise, stays local and says why.
//! A rule is compiled whole or not at all: half a rule on the server and half
//! on the desktop would run the halves at different times.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// First line of every script Satchel writes. A script by that name without
/// this line belongs to someone else and is never overwritten.
pub const SCRIPT_MARKER: &str =
    "# Satchel-managed script. Edits here are replaced on the next push.";

/// A rule as stored, reduced to what the compiler reads.
#[derive(Debug, Clone)]
pub struct RuleSpec {
    pub id: String,
    pub name: String,
    pub trigger_type: String,
    /// The raw JSON column; the fingerprint is taken over it verbatim.
    pub trigger_config: String,
    /// The raw JSON column.
    pub actions: String,
}

impl RuleSpec {
    pub fn fingerprint(&self) -> String {
        rule_fingerprint(&self.trigger_type, &self.trigger_config, &self.actions)
    }
}

/// Identifies one version of a rule's behaviour. The local engine compares it
/// with the pushed one: an edited rule runs locally until it is pushed again.
pub fn rule_fingerprint(trigger_type: &str, trigger_config: &str, actions: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [trigger_type, trigger_config, actions] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Why a rule keeps running in Satchel. Serialized as `{ kind, detail }`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum LocalReason {
    /// Only `on_receive` rules exist on a server.
    Trigger,
    /// An action with no faithful Sieve form (workflow state, priority,
    /// category, tags live in Satchel alone). Detail: the action kind.
    Action(String),
    /// A condition Satchel does not know how to translate.
    Condition(String),
    /// The server lacks an extension the rule needs. Detail: its name.
    Extension(String),
    /// The rule could not be read.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleCompilation {
    Server {
        fragment: String,
        requires: BTreeSet<String>,
    },
    Local(LocalReason),
}

impl RuleCompilation {
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server { .. })
    }
}

const CONDITION_KEYS: [&str; 4] = [
    "from_contains",
    "to_contains",
    "subject_contains",
    "body_contains",
];

/// Compile one rule against the server's extension list (lower case).
pub fn compile_rule(rule: &RuleSpec, extensions: &[String]) -> RuleCompilation {
    let has = |name: &str| extensions.iter().any(|ext| ext.eq_ignore_ascii_case(name));
    if rule.trigger_type != "on_receive" {
        return RuleCompilation::Local(LocalReason::Trigger);
    }
    let Ok(config) = serde_json::from_str::<serde_json::Value>(&rule.trigger_config) else {
        return RuleCompilation::Local(LocalReason::Invalid);
    };
    let Some(config) = config.as_object() else {
        return RuleCompilation::Local(LocalReason::Invalid);
    };
    let Ok(actions) = serde_json::from_str::<Vec<String>>(&rule.actions) else {
        return RuleCompilation::Local(LocalReason::Invalid);
    };

    let mut requires = BTreeSet::new();
    let mut tests = Vec::new();
    for (key, value) in config {
        if key == "match" {
            continue;
        }
        let Some(needle) = value.as_str().map(str::trim).filter(|v| !v.is_empty()) else {
            continue;
        };
        if !CONDITION_KEYS.contains(&key.as_str()) {
            return RuleCompilation::Local(LocalReason::Condition(key.clone()));
        }
        // The local engine lower-cases with Unicode rules; Sieve's default
        // comparator folds ASCII only. Anything else needs unicode-casemap.
        let comparator = if needle.is_ascii() {
            ""
        } else if has("comparator-i;unicode-casemap") {
            requires.insert("comparator-i;unicode-casemap".to_string());
            ":comparator \"i;unicode-casemap\" "
        } else {
            return RuleCompilation::Local(LocalReason::Extension(
                "comparator-i;unicode-casemap".to_string(),
            ));
        };
        let needle = sieve_string(needle);
        // Tests in the order the local engine evaluates them; it only
        // matters for readability, all/any is order-free.
        let test = match key.as_str() {
            // The local engine matches the stored address, not the display
            // name: `address :all` is that, `header` would be the whole line.
            "from_contains" => format!("address :all {comparator}:contains \"from\" {needle}"),
            "to_contains" => {
                format!("address :all {comparator}:contains [\"to\", \"cc\"] {needle}")
            }
            "subject_contains" => format!("header {comparator}:contains \"subject\" {needle}"),
            "body_contains" => {
                if !has("body") {
                    return RuleCompilation::Local(LocalReason::Extension("body".to_string()));
                }
                requires.insert("body".to_string());
                format!("body :text {comparator}:contains {needle}")
            }
            _ => unreachable!("checked against CONDITION_KEYS"),
        };
        tests.push((
            CONDITION_KEYS.iter().position(|k| *k == key).unwrap_or(0),
            test,
        ));
    }
    if tests.is_empty() {
        return RuleCompilation::Local(LocalReason::Invalid);
    }
    tests.sort_by_key(|(position, _)| *position);

    let mut commands = Vec::new();
    for action in &actions {
        let action = action.trim();
        match action {
            "mark_read" => {
                if !has("imap4flags") {
                    return RuleCompilation::Local(LocalReason::Extension(
                        "imap4flags".to_string(),
                    ));
                }
                requires.insert("imap4flags".to_string());
                commands.push("addflag \"\\\\Seen\";".to_string());
            }
            other => {
                let kind = other.split(':').next().unwrap_or(other).to_string();
                return RuleCompilation::Local(LocalReason::Action(kind));
            }
        }
    }
    if commands.is_empty() {
        return RuleCompilation::Local(LocalReason::Invalid);
    }

    let combinator = if config.get("match").and_then(|v| v.as_str()) == Some("any") {
        "anyof"
    } else {
        "allof"
    };
    let tests = tests
        .into_iter()
        .map(|(_, test)| test)
        .collect::<Vec<_>>()
        .join(",\r\n        ");
    let body = commands
        .iter()
        .map(|command| format!("    {command}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    let fragment = format!(
        "# Rule: {}\r\nif {combinator} ({tests}) {{\r\n{body}\r\n}}\r\n",
        comment_text(&rule.name)
    );
    RuleCompilation::Server { fragment, requires }
}

/// The vacation reply as Satchel stores it.
#[derive(Debug, Clone, Default)]
pub struct VacationSpec {
    pub enabled: bool,
    pub subject: Option<String>,
    pub body: String,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    /// The account's own addresses, so a reply goes out for mail sent to an
    /// alias as well.
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VacationCompilation {
    /// No reply configured.
    Off,
    Server {
        fragment: String,
        requires: BTreeSet<String>,
    },
    /// The server cannot send it as configured. Detail: the missing extension.
    Unsupported(String),
}

pub fn compile_vacation(vacation: &VacationSpec, extensions: &[String]) -> VacationCompilation {
    let has = |name: &str| extensions.iter().any(|ext| ext.eq_ignore_ascii_case(name));
    if !vacation.enabled {
        return VacationCompilation::Off;
    }
    if !has("vacation") {
        return VacationCompilation::Unsupported("vacation".to_string());
    }
    let mut requires = BTreeSet::from(["vacation".to_string()]);
    let mut window = Vec::new();
    if vacation.starts_at.is_some() || vacation.ends_at.is_some() {
        // A window the server cannot check would answer outside of it.
        for ext in ["date", "relational"] {
            if !has(ext) {
                return VacationCompilation::Unsupported(ext.to_string());
            }
            requires.insert(ext.to_string());
        }
        // Compared as text in UTC without the zone suffix: the server's
        // "…T10:00:00+00:00" sorts after "…T10:00:00", so the start is
        // inclusive and the end exclusive, like the local definition.
        if let Some(start) = vacation.starts_at {
            window.push(format!(
                "currentdate :zone \"+0000\" :value \"ge\" \"iso8601\" {}",
                sieve_string(&start.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }
        if let Some(end) = vacation.ends_at {
            window.push(format!(
                "currentdate :zone \"+0000\" :value \"lt\" \"iso8601\" {}",
                sieve_string(&end.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }
    }
    let mut command = String::from("vacation");
    if let Some(subject) = vacation
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        command.push_str(" :subject ");
        command.push_str(&sieve_string(&single_line(subject)));
    }
    let addresses = vacation
        .addresses
        .iter()
        .map(|address| address.trim())
        .filter(|address| !address.is_empty())
        .map(sieve_string)
        .collect::<Vec<_>>();
    if !addresses.is_empty() {
        command.push_str(&format!(" :addresses [{}]", addresses.join(", ")));
    }
    command.push(' ');
    command.push_str(&sieve_string(&normalize_newlines(&vacation.body)));
    command.push(';');

    let fragment = if window.is_empty() {
        format!("# Vacation reply\r\n{command}\r\n")
    } else {
        format!(
            "# Vacation reply\r\nif allof ({}) {{\r\n    {command}\r\n}}\r\n",
            window.join(",\r\n        ")
        )
    };
    VacationCompilation::Server { fragment, requires }
}

/// One assembled script plus the per-rule result.
#[derive(Debug, Clone)]
pub struct CompiledScript {
    pub content: String,
    pub rules: Vec<(String, String, RuleCompilation)>,
    pub vacation: VacationCompilation,
}

impl CompiledScript {
    /// `(rule id, fingerprint)` of every rule the script runs.
    pub fn server_rules(&self) -> impl Iterator<Item = (&str, &str)> {
        self.rules
            .iter()
            .filter(|(_, _, compiled)| compiled.is_server())
            .map(|(id, fingerprint, _)| (id.as_str(), fingerprint.as_str()))
    }
}

/// Build the `satchel` script. Rules come in local priority order.
pub fn compile_script(
    rules: &[RuleSpec],
    vacation: &VacationSpec,
    extensions: &[String],
) -> CompiledScript {
    let mut requires = BTreeSet::new();
    let mut sections = Vec::new();

    let vacation_result = compile_vacation(vacation, extensions);
    if let VacationCompilation::Server {
        fragment,
        requires: needed,
    } = &vacation_result
    {
        requires.extend(needed.iter().cloned());
        sections.push(fragment.clone());
    }

    let mut compiled_rules = Vec::new();
    for rule in rules {
        let compiled = compile_rule(rule, extensions);
        if let RuleCompilation::Server {
            fragment,
            requires: needed,
        } = &compiled
        {
            requires.extend(needed.iter().cloned());
            sections.push(fragment.clone());
        }
        compiled_rules.push((rule.id.clone(), rule.fingerprint(), compiled));
    }

    let mut content = format!("{SCRIPT_MARKER}\r\n");
    if !requires.is_empty() {
        let list = requires
            .iter()
            .map(|ext| sieve_string(ext))
            .collect::<Vec<_>>()
            .join(", ");
        content.push_str(&format!("require [{list}];\r\n"));
    }
    for section in sections {
        content.push_str("\r\n");
        content.push_str(&section);
    }
    CompiledScript {
        content,
        rules: compiled_rules,
        vacation: vacation_result,
    }
}

/// The wrapper that keeps someone's own active script running beside
/// Satchel's: Satchel's first (it never stops or files away), theirs after.
pub fn wrapper_script(own_script: &str, other_script: &str) -> String {
    format!(
        "{SCRIPT_MARKER}\r\n# Runs Satchel's rules, then the script that was active before.\r\nrequire [\"include\"];\r\ninclude :personal {};\r\ninclude :personal {};\r\n",
        sieve_string(own_script),
        sieve_string(other_script)
    )
}

/// The script a Satchel wrapper includes after Satchel's own, if `content`
/// is such a wrapper.
pub fn wrapped_script_name(content: &str, own_script: &str) -> Option<String> {
    if !is_satchel_script(content) {
        return None;
    }
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("include :personal "))
        .filter_map(|rest| rest.trim_end_matches(';').trim().strip_prefix('"'))
        .filter_map(|rest| rest.strip_suffix('"'))
        .map(unescape_sieve)
        .find(|name| name != own_script)
}

pub fn is_satchel_script(content: &str) -> bool {
    content.lines().next().map(str::trim_end) == Some(SCRIPT_MARKER)
}

/// A Sieve quoted string (RFC 5228 2.4.2): backslash and double quote
/// escaped, everything else literal.
pub fn sieve_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '"' | '\\' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            '\0' => {}
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

fn unescape_sieve(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Scripts travel with CRLF line ends (RFC 5228 2.2); a quoted string may
/// span lines, but a lone CR or LF inside one is not a line end.
fn normalize_newlines(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

fn single_line(value: &str) -> String {
    value
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text after a `#` runs to the end of the line; keep a name on one line.
fn comment_text(value: &str) -> String {
    single_line(value)
        .chars()
        .filter(|ch| !ch.is_control())
        .collect()
}

#[cfg(test)]
#[path = "compiler_cases.rs"]
mod tests;
