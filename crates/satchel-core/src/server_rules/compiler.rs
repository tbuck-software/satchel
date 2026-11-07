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
#[derive(Debug, Clone, Default)]
pub struct RuleSpec {
    pub id: String,
    pub name: String,
    pub trigger_type: String,
    /// The raw JSON column; the fingerprint is taken over it verbatim.
    pub trigger_config: String,
    /// The raw JSON column.
    pub actions: String,
    /// The one account the rule applies to; `None` for every account.
    pub account_id: Option<String>,
}

impl RuleSpec {
    pub fn fingerprint(&self) -> String {
        match self.account_id.as_deref() {
            None => rule_fingerprint(&self.trigger_type, &self.trigger_config, &self.actions),
            // Scoping a rule changes what it does: part of its version.
            Some(account) => rule_fingerprint(
                &self.trigger_type,
                &self.trigger_config,
                &format!("{}\0{account}", self.actions),
            ),
        }
    }

    /// The fingerprint of the rule as it runs on one account: with the server
    /// name of every folder it moves to, so a renamed or deleted folder makes
    /// it a different version (pushed again, and not mistaken for the old
    /// one by the local engine). Without a move it is [`Self::fingerprint`].
    pub fn fingerprint_for(&self, folders: &std::collections::BTreeMap<String, String>) -> String {
        let targets: Vec<String> = serde_json::from_str::<Vec<String>>(&self.actions)
            .unwrap_or_default()
            .iter()
            .filter_map(|action| match crate::rules::RuleAction::parse(action) {
                Some(crate::rules::RuleAction::MoveToFolder(id)) => Some(
                    folders
                        .get(&id)
                        .map(|name| format!("{id}={name}"))
                        .unwrap_or_else(|| format!("{id}=")),
                ),
                _ => None,
            })
            .collect();
        if targets.is_empty() {
            return self.fingerprint();
        }
        rule_fingerprint(
            &self.trigger_type,
            &self.trigger_config,
            &format!(
                "{}\0{}\0{}",
                self.actions,
                self.account_id.as_deref().unwrap_or_default(),
                targets.join("\0")
            ),
        )
    }

    /// Whether the rule applies to mail of `account_id`.
    pub fn applies_to(&self, account_id: &str) -> bool {
        self.account_id
            .as_deref()
            .is_none_or(|own| own == account_id)
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
    /// The target folder is not one of this account's, or its name cannot be
    /// given to the server.
    Folder,
    /// The rule could not be read.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleCompilation {
    Server {
        /// Everything but the move: flags, marks and the forward. Empty for
        /// a rule that only moves.
        fragment: String,
        /// The move, which [`compile_script`] places in one `if … elsif …`
        /// chain after every rule's fragment: the first matching rule decides
        /// the folder, like the local engine, and the flags of all matching
        /// rules are set by then (`fileinto` takes the flags it sees).
        filing: Option<Filing>,
        requires: BTreeSet<String>,
    },
    Local(LocalReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filing {
    pub rule_name: String,
    /// The rule's test, with the target's existence check when the server
    /// has `mailbox`.
    pub test: String,
    /// The folder as Sieve names it (UTF-8, the server's hierarchy delimiter).
    pub mailbox: String,
}

/// What the compiler knows about the account it compiles for.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// The server's extensions (lower case).
    pub extensions: &'a [String],
    /// `None` when compiling without an account (tests, the badge for a rule
    /// that applies to every account).
    pub account_id: Option<&'a str>,
    /// Folder id -> IMAP mailbox name as the server sent it (modified UTF-7),
    /// for this account's folders.
    pub folders: &'a std::collections::BTreeMap<String, String>,
}

static NO_FOLDERS: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();

impl<'a> Target<'a> {
    /// A target that knows extensions only: rules that move stay local.
    pub fn extensions_only(extensions: &'a [String]) -> Self {
        Self {
            extensions,
            account_id: None,
            folders: &NO_FOLDERS,
        }
    }
}

impl RuleCompilation {
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server { .. })
    }
}

/// Conditions with a Sieve form. `body_contains` is not among them yet: the
/// local engine searches the stored plain text, Sieve's `body :text` the raw
/// text parts (HTML-only mail, quoted-printable, signatures differ), and that
/// has not been checked against a real server.
const CONDITION_KEYS: [&str; 3] = ["from_contains", "to_contains", "subject_contains"];

/// Compile one rule against the server's extension list (lower case),
/// without an account: a rule that moves mail stays local.
pub fn compile_rule(rule: &RuleSpec, extensions: &[String]) -> RuleCompilation {
    compile_rule_for(rule, &Target::extensions_only(extensions))
}

/// The loop guard of a forward to `target`, the same headers the local engine
/// reads ([`crate::rules::forward_loop_guard`]): automatic mail (other than
/// `Auto-Submitted: no`), bounces, delivery and read reports, list, bulk and
/// spam mail, and mail from or answering to the target itself.
fn forward_loop_guard(target: &str, envelope: bool) -> String {
    // A bounce has the empty envelope sender; the Return-Path header is how
    // the delivery agent reports it to a server without `envelope`.
    let bounce = if envelope {
        "envelope :all :is \"from\" \"\"".to_string()
    } else {
        "header :is \"return-path\" [\"<>\", \"< >\"]".to_string()
    };
    let mut tests = vec![
        "allof (exists \"auto-submitted\", not header :is \"auto-submitted\" \"no\")".to_string(),
        "exists [\"list-id\", \"list-unsubscribe\"]".to_string(),
        "header :is \"precedence\" [\"bulk\", \"list\", \"junk\"]".to_string(),
        bounce,
        "address :localpart :is \"from\" [\"postmaster\", \"mailer-daemon\"]".to_string(),
        "header :matches \"content-type\" \"multipart/report*\"".to_string(),
        "header :is \"x-spam-flag\" \"yes\"".to_string(),
        "header :matches \"x-spam-status\" \"yes*\"".to_string(),
        format!(
            "address :all :is [\"from\", \"reply-to\"] {}",
            sieve_string(target)
        ),
    ];
    // `local+tag@domain` is the target too, as the local engine sees it.
    if let Some((local, domain)) = target.trim().rsplit_once('@') {
        let local = local.split('+').next().unwrap_or(local);
        tests.push(format!(
            "address :all :matches [\"from\", \"reply-to\"] {}",
            sieve_string(&format!(
                "{}+*@{}",
                escape_wildcards(local),
                escape_wildcards(domain)
            ))
        ));
    }
    tests.join(",\r\n            ")
}

/// Escape `:matches` wildcards (RFC 5228 2.7.1) so they match literally.
fn escape_wildcards(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '*' | '?' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Compile one rule for one account's server.
pub fn compile_rule_for(rule: &RuleSpec, target: &Target<'_>) -> RuleCompilation {
    use crate::rules::RuleAction;
    let extensions = target.extensions;
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
        // A line break cannot be part of a header match; the local engine
        // compares against unfolded text, so it reads as a space.
        let Some(needle) = value
            .as_str()
            .map(|v| v.replace("\r\n", " ").replace(['\r', '\n'], " "))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let needle = needle.as_str();
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

    let need = |name: &str, requires: &mut BTreeSet<String>| {
        if has(name) {
            requires.insert(name.to_string());
            Ok(())
        } else {
            Err(RuleCompilation::Local(LocalReason::Extension(
                name.to_string(),
            )))
        }
    };
    // Flags first: a `fileinto` later in the script takes the flags it sees.
    let mut flags = Vec::new();
    let mut forwards = Vec::new();
    let mut mailbox = None;
    for action in &actions {
        let Some(parsed) = RuleAction::parse(action) else {
            let action = action.trim();
            let kind = action.split(':').next().unwrap_or(action).to_string();
            return RuleCompilation::Local(LocalReason::Action(kind));
        };
        match parsed {
            RuleAction::MarkRead => {
                if let Err(local) = need("imap4flags", &mut requires) {
                    return local;
                }
                flags.push("addflag \"\\\\Seen\";".to_string());
            }
            RuleAction::Star => {
                if let Err(local) = need("imap4flags", &mut requires) {
                    return local;
                }
                flags.push("addflag \"\\\\Flagged\";".to_string());
            }
            RuleAction::MoveToFolder(folder_id) => {
                // The filed mark tells Satchel to run its local-only rules
                // on mail the server took out of the Inbox.
                for extension in ["fileinto", "imap4flags"] {
                    if let Err(local) = need(extension, &mut requires) {
                        return local;
                    }
                }
                let Some(name) = target
                    .account_id
                    .filter(|account| rule.applies_to(account))
                    .and_then(|_| target.folders.get(&folder_id))
                    .and_then(|wire| decode_imap_utf7(wire))
                    .filter(|name| !name.trim().is_empty() && !name.contains('\0'))
                else {
                    return RuleCompilation::Local(LocalReason::Folder);
                };
                mailbox = Some(name);
            }
            RuleAction::Forward { to, keep_copy } => {
                if !crate::rules::is_forward_address(&to) {
                    return RuleCompilation::Local(LocalReason::Invalid);
                }
                // The forward mark is how Satchel knows the server sent it.
                if let Err(local) = need("imap4flags", &mut requires) {
                    return local;
                }
                if keep_copy {
                    if let Err(local) = need("copy", &mut requires) {
                        return local;
                    }
                }
                let copy = if keep_copy { ":copy " } else { "" };
                forwards.push((
                    to.trim().to_string(),
                    format!("redirect {copy}{};", sieve_string(to.trim())),
                ));
            }
            other => return RuleCompilation::Local(LocalReason::Action(other.kind().to_string())),
        }
    }
    if flags.is_empty() && forwards.is_empty() && mailbox.is_none() {
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
    let test = format!("{combinator} ({tests})");
    let name = comment_text(&rule.name);

    let mut body: Vec<String> = flags.iter().map(|flag| format!("    {flag}")).collect();
    // The mark sits behind the guard: it says the server forwarded, and a
    // later `fileinto` or the implicit keep takes it along.
    let envelope = !forwards.is_empty() && has("envelope");
    if envelope {
        requires.insert("envelope".to_string());
    }
    for (target, redirect) in &forwards {
        body.push(format!(
            "    if not anyof ({}) {{",
            forward_loop_guard(target, envelope)
        ));
        body.push(format!(
            "        addflag {};",
            sieve_string(crate::rules::FORWARD_MARK)
        ));
        body.push(format!("        {redirect}"));
        body.push("    }".to_string());
    }
    let fragment = if body.is_empty() {
        String::new()
    } else {
        format!(
            "# Rule: {name}\r\nif {test} {{\r\n{}\r\n}}\r\n",
            body.join("\r\n")
        )
    };
    let filing = mailbox.map(|mailbox| {
        let test = if has("mailbox") {
            requires.insert("mailbox".to_string());
            format!("allof ({test}, mailboxexists {})", sieve_string(&mailbox))
        } else {
            test.clone()
        };
        Filing {
            rule_name: name.clone(),
            test,
            mailbox,
        }
    });
    RuleCompilation::Server {
        fragment,
        filing,
        requires,
    }
}

/// The forward of a rule: its trigger and address, the part that decides
/// whether a message is forwarded (`since` left empty).
pub fn forward_of(rule: &RuleSpec) -> Option<crate::db::server_rules::LiveForward> {
    let actions = serde_json::from_str::<Vec<String>>(&rule.actions).ok()?;
    actions
        .iter()
        .find_map(|action| match crate::rules::RuleAction::parse(action) {
            Some(crate::rules::RuleAction::Forward { to, .. }) => {
                Some(crate::db::server_rules::LiveForward {
                    trigger_type: rule.trigger_type.clone(),
                    trigger_config: rule.trigger_config.clone(),
                    to: to.trim().to_string(),
                    since: String::new(),
                })
            }
            _ => None,
        })
}

/// Decode an IMAP mailbox name (RFC 3501 5.1.3, modified UTF-7) to the UTF-8
/// that Sieve's `fileinto` takes. A name the server already sent as UTF-8
/// passes through. `None` for a malformed name.
pub fn decode_imap_utf7(wire: &str) -> Option<String> {
    use base64::Engine as _;
    let mut out = String::with_capacity(wire.len());
    let mut rest = wire;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('-')?;
        let encoded = &after[..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            let standard = encoded.replace(',', "/");
            let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(standard.as_bytes())
                .ok()?;
            if bytes.len() % 2 != 0 {
                return None;
            }
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            out.push_str(&String::from_utf16(&units).ok()?);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
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

/// How often one sender gets the reply again (RFC 5230 `:days`).
pub const VACATION_DAYS: u32 = 7;

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
    // Once a week per sender, stated rather than left to the server's default.
    let mut command = format!("vacation :days {VACATION_DAYS}");
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
    /// Rule id -> the forward of each rule the script runs that forwards
    /// (`since` empty; the push fills it in).
    pub forwards: std::collections::BTreeMap<String, crate::db::server_rules::LiveForward>,
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

/// Build the `satchel` script without an account; see [`compile_script_for`].
pub fn compile_script(
    rules: &[RuleSpec],
    vacation: &VacationSpec,
    extensions: &[String],
) -> CompiledScript {
    compile_script_for(rules, vacation, &Target::extensions_only(extensions))
}

/// Build the `satchel` script for one account. Rules come in local priority
/// order; a rule for another account is left out. Every rule's flags and
/// forwards first, then one `if … elsif …` chain of the moves.
pub fn compile_script_for(
    rules: &[RuleSpec],
    vacation: &VacationSpec,
    target: &Target<'_>,
) -> CompiledScript {
    let mut requires = BTreeSet::new();
    let mut sections = Vec::new();

    let vacation_result = compile_vacation(vacation, target.extensions);
    if let VacationCompilation::Server {
        fragment,
        requires: needed,
    } = &vacation_result
    {
        requires.extend(needed.iter().cloned());
        sections.push(fragment.clone());
    }

    let mut compiled_rules = Vec::new();
    let mut forwards = std::collections::BTreeMap::new();
    let mut filings = Vec::new();
    for rule in rules {
        if target
            .account_id
            .is_some_and(|account| !rule.applies_to(account))
        {
            continue;
        }
        let compiled = compile_rule_for(rule, target);
        if let RuleCompilation::Server {
            fragment,
            filing,
            requires: needed,
        } = &compiled
        {
            requires.extend(needed.iter().cloned());
            if !fragment.is_empty() {
                sections.push(fragment.clone());
            }
            if let Some(filing) = filing {
                filings.push(filing.clone());
            }
        }
        if compiled.is_server() {
            if let Some(forward) = forward_of(rule) {
                forwards.insert(rule.id.clone(), forward);
            }
        }
        compiled_rules.push((
            rule.id.clone(),
            rule.fingerprint_for(target.folders),
            compiled,
        ));
    }
    if !filings.is_empty() {
        let mut chain = String::from("# Moves: the first matching rule decides the folder.\r\n");
        for (index, filing) in filings.iter().enumerate() {
            let keyword = if index == 0 { "if" } else { "elsif" };
            let close = if index == 0 { "" } else { "} " };
            chain.push_str(&format!(
                "{close}{keyword} {} {{\r\n    # Rule: {}\r\n    addflag {};\r\n    fileinto {};\r\n",
                filing.test,
                filing.rule_name,
                sieve_string(crate::rules::FILED_MARK),
                sieve_string(&filing.mailbox)
            ));
        }
        chain.push_str("}\r\n");
        sections.push(chain);
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
        forwards,
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
