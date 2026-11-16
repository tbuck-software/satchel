use super::*;

fn all_extensions() -> Vec<String> {
    [
        "fileinto",
        "imap4flags",
        "vacation",
        "body",
        "include",
        "date",
        "relational",
        "comparator-i;unicode-casemap",
        "copy",
        "mailbox",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn rule(config: serde_json::Value, actions: &[&str]) -> RuleSpec {
    RuleSpec {
        id: "rule-1".into(),
        name: "Billing".into(),
        trigger_type: "on_receive".into(),
        trigger_config: config.to_string(),
        actions: serde_json::to_string(actions).unwrap(),
        account_id: None,
    }
}

fn fragment(compiled: RuleCompilation) -> (String, BTreeSet<String>) {
    match compiled {
        RuleCompilation::Server {
            fragment, requires, ..
        } => (fragment, requires),
        RuleCompilation::Local(reason) => panic!("stayed local: {reason:?}"),
    }
}

#[test]
fn from_matches_the_address_like_the_local_engine() {
    let (text, requires) = fragment(compile_rule(
        &rule(
            serde_json::json!({"from_contains": "billing@"}),
            &["mark_read"],
        ),
        &all_extensions(),
    ));
    assert!(
        text.contains("address :all :contains \"from\" \"billing@\""),
        "{text}"
    );
    assert!(text.contains("addflag \"\\\\Seen\";"), "{text}");
    assert_eq!(requires, BTreeSet::from(["imap4flags".to_string()]));
}

#[test]
fn to_covers_cc_and_subject_uses_the_header() {
    let (text, _) = fragment(compile_rule(
        &rule(
            serde_json::json!({"match": "all", "to_contains": "team@", "subject_contains": "Invoice"}),
            &["mark_read"],
        ),
        &all_extensions(),
    ));
    assert!(text.contains("if allof ("), "{text}");
    assert!(
        text.contains("address :all :contains [\"to\", \"cc\"] \"team@\""),
        "{text}"
    );
    assert!(
        text.contains("header :contains \"subject\" \"Invoice\""),
        "{text}"
    );
    // Local evaluation order: to before subject.
    assert!(text.find("\"to\"").unwrap() < text.find("\"subject\"").unwrap());
}

#[test]
fn any_becomes_anyof() {
    let (text, _) = fragment(compile_rule(
        &rule(
            serde_json::json!({"match": "any", "from_contains": "a@", "subject_contains": "b"}),
            &["mark_read"],
        ),
        &all_extensions(),
    ));
    assert!(text.contains("if anyof ("), "{text}");
}

#[test]
fn body_stays_local_until_verified_on_a_server() {
    // Stored plain text locally, raw text parts in Sieve: not the same match.
    assert_eq!(
        compile_rule(
            &rule(
                serde_json::json!({"body_contains": "unsubscribe"}),
                &["mark_read"]
            ),
            &all_extensions()
        ),
        RuleCompilation::Local(LocalReason::Condition("body_contains".into()))
    );
}

#[test]
fn line_breaks_in_a_needle_read_as_spaces() {
    let (text, _) = fragment(compile_rule(
        &rule(
            serde_json::json!({"subject_contains": "two\r\nlines"}),
            &["mark_read"],
        ),
        &all_extensions(),
    ));
    assert!(text.contains("\"subject\" \"two lines\""), "{text}");
}

#[test]
fn mark_read_needs_imap4flags() {
    assert_eq!(
        compile_rule(
            &rule(serde_json::json!({"from_contains": "x"}), &["mark_read"]),
            &["fileinto".to_string()]
        ),
        RuleCompilation::Local(LocalReason::Extension("imap4flags".into()))
    );
}

#[test]
fn satchel_only_actions_stay_local_whole() {
    for (action, kind) in [
        ("set_state:done", "set_state"),
        ("set_priority:2", "set_priority"),
        ("set_category:updates", "set_category"),
        ("add_tag:billing", "add_tag"),
    ] {
        assert_eq!(
            compile_rule(
                &rule(
                    serde_json::json!({"from_contains": "x"}),
                    &["mark_read", action]
                ),
                &all_extensions()
            ),
            RuleCompilation::Local(LocalReason::Action(kind.into())),
            "{action}"
        );
    }
}

#[test]
fn other_triggers_and_unknown_conditions_stay_local() {
    let mut scheduled = rule(serde_json::json!({"from_contains": "x"}), &["mark_read"]);
    scheduled.trigger_type = "scheduled".into();
    assert_eq!(
        compile_rule(&scheduled, &all_extensions()),
        RuleCompilation::Local(LocalReason::Trigger)
    );
    assert_eq!(
        compile_rule(
            &rule(serde_json::json!({"size_over": "100"}), &["mark_read"]),
            &all_extensions()
        ),
        RuleCompilation::Local(LocalReason::Condition("size_over".into()))
    );
    let mut broken = rule(serde_json::json!({}), &["mark_read"]);
    broken.trigger_config = "{not json".into();
    assert_eq!(
        compile_rule(&broken, &all_extensions()),
        RuleCompilation::Local(LocalReason::Invalid)
    );
}

#[test]
fn strings_are_escaped() {
    let (text, _) = fragment(compile_rule(
        &rule(
            serde_json::json!({"subject_contains": "say \"hi\" \\o/"}),
            &["mark_read"],
        ),
        &all_extensions(),
    ));
    assert!(
        text.contains(r#"header :contains "subject" "say \"hi\" \\o/""#),
        "{text}"
    );
    assert_eq!(sieve_string("a\"b\\c\0d"), r#""a\"b\\cd""#);
}

#[test]
fn variables_syntax_stays_literal_without_the_extension() {
    // `${…}` only expands under `require "variables"`, which is never asked for.
    let compiled = compile_script(
        &[rule(
            serde_json::json!({"subject_contains": "${hex:41}"}),
            &["mark_read"],
        )],
        &VacationSpec::default(),
        &all_extensions(),
    );
    assert!(!compiled.content.contains("variables"));
    assert!(!compiled.content.contains("encoded-character"));
    assert!(compiled.content.contains("\"${hex:41}\""));
}

#[test]
fn non_ascii_needles_need_unicode_casemap() {
    let config = serde_json::json!({"subject_contains": "Größe"});
    let (text, requires) = fragment(compile_rule(
        &rule(config.clone(), &["mark_read"]),
        &all_extensions(),
    ));
    assert!(
        text.contains("header :comparator \"i;unicode-casemap\" :contains \"subject\" \"Größe\""),
        "{text}"
    );
    assert!(requires.contains("comparator-i;unicode-casemap"));
    assert_eq!(
        compile_rule(&rule(config, &["mark_read"]), &["imap4flags".to_string()]),
        RuleCompilation::Local(LocalReason::Extension(
            "comparator-i;unicode-casemap".into()
        ))
    );
}

#[test]
fn a_rule_name_cannot_break_out_of_its_comment() {
    let mut named = rule(serde_json::json!({"from_contains": "x"}), &["mark_read"]);
    named.name = "Evil\nkeep; discard;".into();
    let (text, _) = fragment(compile_rule(&named, &all_extensions()));
    assert!(
        text.starts_with("# Rule: Evil keep; discard;\r\nif "),
        "{text}"
    );
}

#[test]
fn the_script_carries_marker_requires_and_rules_in_order() {
    let mut second = rule(serde_json::json!({"subject_contains": "b"}), &["mark_read"]);
    second.id = "rule-2".into();
    second.name = "Second".into();
    let local = RuleSpec {
        id: "rule-3".into(),
        ..rule(
            serde_json::json!({"from_contains": "c"}),
            &["set_state:done"],
        )
    };
    let compiled = compile_script(
        &[
            rule(serde_json::json!({"to_contains": "a"}), &["mark_read"]),
            second,
            local,
        ],
        &VacationSpec::default(),
        &all_extensions(),
    );
    assert!(compiled.content.starts_with(SCRIPT_MARKER));
    assert!(is_satchel_script(&compiled.content));
    assert!(compiled.content.contains("require [\"imap4flags\"];\r\n"));
    assert!(
        compiled.content.find("# Rule: Billing").unwrap()
            < compiled.content.find("# Rule: Second").unwrap()
    );
    let server: Vec<&str> = compiled.server_rules().map(|(id, _)| id).collect();
    assert_eq!(server, vec!["rule-1", "rule-2"]);
    assert!(
        !compiled.content.contains("\n\n"),
        "CRLF only: {:?}",
        compiled.content
    );
}

#[test]
fn an_empty_script_is_valid_and_keeps_everything() {
    let compiled = compile_script(&[], &VacationSpec::default(), &all_extensions());
    assert_eq!(compiled.content, format!("{SCRIPT_MARKER}\r\n"));
}

fn vacation() -> VacationSpec {
    VacationSpec {
        enabled: true,
        subject: Some("Away \"now\"".into()),
        body: "Back on Monday.\nRegards".into(),
        starts_at: None,
        ends_at: None,
        addresses: vec!["me@example.com".into()],
    }
}

#[test]
fn vacation_without_a_window() {
    let VacationCompilation::Server { fragment, requires } =
        compile_vacation(&vacation(), &all_extensions())
    else {
        panic!("not compiled");
    };
    assert_eq!(requires, BTreeSet::from(["vacation".to_string()]));
    assert!(fragment.contains(
        "vacation :days 7 :subject \"Away \\\"now\\\"\" :addresses [\"me@example.com\"] \"Back on Monday.\r\nRegards\";"
    ), "{fragment}");
    assert!(!fragment.contains("currentdate"));
}

#[test]
fn vacation_window_uses_date_and_relational() {
    let mut spec = vacation();
    spec.starts_at = Some("2026-12-20T08:00:00Z".parse().unwrap());
    spec.ends_at = Some("2027-01-06T00:00:00Z".parse().unwrap());
    let VacationCompilation::Server { fragment, requires } =
        compile_vacation(&spec, &all_extensions())
    else {
        panic!("not compiled");
    };
    assert!(requires.contains("date") && requires.contains("relational"));
    assert!(
        fragment.contains(
            "currentdate :zone \"+0000\" :value \"ge\" \"iso8601\" \"2026-12-20T08:00:00\""
        ),
        "{fragment}"
    );
    assert!(
        fragment.contains(
            "currentdate :zone \"+0000\" :value \"lt\" \"iso8601\" \"2027-01-06T00:00:00\""
        ),
        "{fragment}"
    );

    let no_date: Vec<String> = all_extensions()
        .into_iter()
        .filter(|ext| ext != "date")
        .collect();
    assert_eq!(
        compile_vacation(&spec, &no_date),
        VacationCompilation::Unsupported("date".into())
    );
}

#[test]
fn vacation_needs_the_extension_and_off_means_nothing() {
    assert_eq!(
        compile_vacation(&vacation(), &["imap4flags".to_string()]),
        VacationCompilation::Unsupported("vacation".into())
    );
    let off = VacationSpec {
        enabled: false,
        ..vacation()
    };
    assert_eq!(
        compile_vacation(&off, &all_extensions()),
        VacationCompilation::Off
    );
    let script = compile_script(&[], &vacation(), &all_extensions());
    assert!(script.content.contains("require [\"vacation\"];"));
    assert!(script.content.contains("# Vacation reply"));
}

#[test]
fn wrapper_names_round_trip() {
    let wrapper = wrapper_script("satchel", "my \"rules\"");
    assert!(wrapper.contains("require [\"include\"];"));
    assert!(wrapper.find("\"satchel\"").unwrap() < wrapper.find("my \\\"rules\\\"").unwrap());
    assert_eq!(
        wrapped_script_name(&wrapper, "satchel").as_deref(),
        Some("my \"rules\"")
    );
    assert_eq!(
        wrapped_script_name("include :personal \"x\";", "satchel"),
        None
    );
}

#[test]
fn fingerprints_change_with_behaviour() {
    let base = rule(serde_json::json!({"from_contains": "a"}), &["mark_read"]);
    let mut renamed = base.clone();
    renamed.name = "Other name".into();
    assert_eq!(base.fingerprint(), renamed.fingerprint());
    let changed = rule(serde_json::json!({"from_contains": "b"}), &["mark_read"]);
    assert_ne!(base.fingerprint(), changed.fingerprint());
}

fn folders() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([
        ("f-billing".to_string(), "INBOX.Rechnungen".to_string()),
        ("f-drafts".to_string(), "Entw&APw-rfe/2026".to_string()),
    ])
}

fn target<'a>(
    extensions: &'a [String],
    folders: &'a std::collections::BTreeMap<String, String>,
) -> Target<'a> {
    Target {
        extensions,
        account_id: Some("acc"),
        folders,
    }
}

fn on_account(mut spec: RuleSpec) -> RuleSpec {
    spec.account_id = Some("acc".into());
    spec
}

#[test]
fn star_adds_the_flagged_flag() {
    let (text, requires) = fragment(compile_rule(
        &rule(serde_json::json!({"from_contains": "boss@"}), &["star"]),
        &all_extensions(),
    ));
    assert!(text.contains("addflag \"\\\\Flagged\";"), "{text}");
    assert_eq!(requires, BTreeSet::from(["imap4flags".to_string()]));
    assert_eq!(
        compile_rule(
            &rule(serde_json::json!({"from_contains": "boss@"}), &["star"]),
            &["fileinto".to_string()]
        ),
        RuleCompilation::Local(LocalReason::Extension("imap4flags".into()))
    );
}

#[test]
fn a_move_files_into_the_server_name_of_the_folder() {
    let extensions = all_extensions();
    let folders = folders();
    let compiled = compile_script_for(
        &[on_account(rule(
            serde_json::json!({"from_contains": "billing@"}),
            &["mark_read", "move_to_folder:f-billing"],
        ))],
        &VacationSpec::default(),
        &target(&extensions, &folders),
    );
    let text = &compiled.content;
    assert!(
        text.contains("require [\"fileinto\", \"imap4flags\", \"mailbox\"];"),
        "{text}"
    );
    // Flags are set before the move, which takes them along.
    let seen = text.find("addflag \"\\\\Seen\";").unwrap();
    let filed = text.find("fileinto \"INBOX.Rechnungen\";").unwrap();
    assert!(seen < filed, "{text}");
    assert!(
        text.contains("mailboxexists \"INBOX.Rechnungen\""),
        "a folder deleted on the server is not recreated: {text}"
    );
    assert!(text.contains("addflag \"$SatchelFiled\";"), "{text}");
    assert_eq!(compiled.server_rules().count(), 1);
}

#[test]
fn folder_names_are_decoded_from_modified_utf7() {
    let extensions = all_extensions();
    let folders = folders();
    let compiled = compile_rule_for(
        &on_account(rule(
            serde_json::json!({"subject_contains": "Entwurf"}),
            &["move_to_folder:f-drafts"],
        )),
        &target(&extensions, &folders),
    );
    let RuleCompilation::Server {
        fragment, filing, ..
    } = compiled
    else {
        panic!("stayed local");
    };
    assert!(fragment.is_empty(), "a move alone has nothing else to do");
    assert_eq!(filing.unwrap().mailbox, "Entwürfe/2026");
    assert_eq!(decode_imap_utf7("A&-B").as_deref(), Some("A&B"));
    assert_eq!(decode_imap_utf7("Gr&APYA3w-e").as_deref(), Some("Größe"));
    assert_eq!(decode_imap_utf7("Bad&AP"), None);
}

#[test]
fn a_move_without_fileinto_or_a_known_folder_stays_local() {
    let folders = folders();
    let flags_only = vec!["imap4flags".to_string()];
    let spec = on_account(rule(
        serde_json::json!({"from_contains": "x"}),
        &["move_to_folder:f-billing"],
    ));
    assert_eq!(
        compile_rule_for(&spec, &target(&flags_only, &folders)),
        RuleCompilation::Local(LocalReason::Extension("fileinto".into()))
    );
    let fileinto_only = vec!["fileinto".to_string()];
    assert_eq!(
        compile_rule_for(&spec, &target(&fileinto_only, &folders)),
        RuleCompilation::Local(LocalReason::Extension("imap4flags".into())),
        "the filed mark needs imap4flags"
    );
    let extensions = all_extensions();
    let unknown = on_account(rule(
        serde_json::json!({"from_contains": "x"}),
        &["move_to_folder:f-gone"],
    ));
    assert_eq!(
        compile_rule_for(&unknown, &target(&extensions, &folders)),
        RuleCompilation::Local(LocalReason::Folder)
    );
    // Without an account the folder cannot be named.
    assert_eq!(
        compile_rule(&spec, &extensions),
        RuleCompilation::Local(LocalReason::Folder)
    );
}

#[test]
fn without_mailbox_the_move_has_no_existence_check() {
    let extensions: Vec<String> = ["fileinto", "imap4flags"].map(String::from).to_vec();
    let folders = folders();
    let compiled = compile_script_for(
        &[on_account(rule(
            serde_json::json!({"from_contains": "x"}),
            &["move_to_folder:f-billing"],
        ))],
        &VacationSpec::default(),
        &target(&extensions, &folders),
    );
    assert!(
        !compiled.content.contains("mailbox"),
        "{}",
        compiled.content
    );
    assert!(compiled.content.contains("fileinto \"INBOX.Rechnungen\";"));
}

#[test]
fn the_first_matching_move_wins_like_the_local_engine() {
    let extensions = all_extensions();
    let folders = folders();
    let first = on_account(rule(
        serde_json::json!({"from_contains": "a@"}),
        &["move_to_folder:f-billing"],
    ));
    let mut second = on_account(rule(
        serde_json::json!({"from_contains": "b@"}),
        &["star", "move_to_folder:f-drafts"],
    ));
    second.id = "rule-2".into();
    second.name = "Second".into();
    let compiled = compile_script_for(
        &[first, second],
        &VacationSpec::default(),
        &target(&extensions, &folders),
    );
    let text = &compiled.content;
    let chain = text.find("# Moves:").unwrap();
    assert!(
        text.find("addflag \"\\\\Flagged\";").unwrap() < chain,
        "{text}"
    );
    assert!(text[chain..].contains("\r\n} elsif allof ("), "{text}");
    assert_eq!(text.matches("fileinto ").count(), 2);
}

#[test]
fn a_rule_for_another_account_is_left_out() {
    let extensions = all_extensions();
    let folders = folders();
    let mut other = rule(serde_json::json!({"from_contains": "x"}), &["mark_read"]);
    other.account_id = Some("someone-else".into());
    let compiled = compile_script_for(
        &[other],
        &VacationSpec::default(),
        &target(&extensions, &folders),
    );
    assert_eq!(compiled.rules.len(), 0);
    assert_eq!(compiled.content, format!("{SCRIPT_MARKER}\r\n"));
}

#[test]
fn a_forward_keeping_a_copy_is_redirect_copy_behind_the_loop_guard() {
    let (text, requires) = fragment(compile_rule(
        &rule(
            serde_json::json!({"from_contains": "billing@"}),
            &["forward:books@example.test"],
        ),
        &all_extensions(),
    ));
    assert!(
        text.contains("redirect :copy \"books@example.test\";"),
        "{text}"
    );
    assert!(
        text.contains("addflag \"$SatchelFwd\";"),
        "one keyword for every forwarding rule: {text}"
    );
    for test in [
        "allof (exists \"auto-submitted\", not header :is \"auto-submitted\" \"no\")",
        "exists [\"list-id\", \"list-unsubscribe\"]",
        "header :is \"precedence\" [\"bulk\", \"list\", \"junk\"]",
        "header :is \"return-path\" [\"<>\", \"< >\"]",
        "address :localpart :is \"from\" [\"postmaster\", \"mailer-daemon\"]",
        "header :matches \"content-type\" \"multipart/report*\"",
        "header :is \"x-spam-flag\" \"yes\"",
        "header :matches \"x-spam-status\" \"yes*\"",
        "address :all :is [\"from\", \"reply-to\"] \"books@example.test\"",
    ] {
        assert!(text.contains(test), "guard lacks {test}: {text}");
    }
    let guard = text.find("if not anyof (").unwrap();
    let mark = text.find("addflag \"$SatchelFwd\";").unwrap();
    assert!(
        guard < mark && mark < text.find("redirect").unwrap(),
        "marked only when it really forwards: {text}"
    );
    assert_eq!(
        requires,
        BTreeSet::from(["copy".to_string(), "imap4flags".to_string()])
    );
}

#[test]
fn a_forward_without_a_copy_is_plain_redirect_and_needs_no_copy_extension() {
    let extensions: Vec<String> = ["imap4flags"].map(String::from).to_vec();
    let (text, requires) = fragment(compile_rule(
        &rule(
            serde_json::json!({"from_contains": "x"}),
            &["forward_no_copy:books@example.test"],
        ),
        &extensions,
    ));
    assert!(text.contains("redirect \"books@example.test\";"), "{text}");
    assert!(!text.contains(":copy"));
    assert_eq!(requires, BTreeSet::from(["imap4flags".to_string()]));
}

#[test]
fn a_forward_stays_local_without_copy_or_imap4flags() {
    let spec = rule(
        serde_json::json!({"from_contains": "x"}),
        &["forward:books@example.test"],
    );
    let no_copy: Vec<String> = ["imap4flags", "fileinto"].map(String::from).to_vec();
    assert_eq!(
        compile_rule(&spec, &no_copy),
        RuleCompilation::Local(LocalReason::Extension("copy".into()))
    );
    let no_flags: Vec<String> = ["copy"].map(String::from).to_vec();
    assert_eq!(
        compile_rule(&spec, &no_flags),
        RuleCompilation::Local(LocalReason::Extension("imap4flags".into())),
        "without the mark Satchel could not tell it was sent"
    );
    let bad = rule(
        serde_json::json!({"from_contains": "x"}),
        &["forward:Team <books@example.test>"],
    );
    assert_eq!(
        compile_rule(&bad, &all_extensions()),
        RuleCompilation::Local(LocalReason::Invalid)
    );
}

#[test]
fn a_forward_address_is_escaped_like_every_string() {
    // Validation refuses quotes in addresses; the compiler escapes anyway.
    assert_eq!(sieve_string("a\"b@example.test"), "\"a\\\"b@example.test\"");
}

#[test]
fn scoping_a_rule_changes_its_version() {
    let all = rule(serde_json::json!({"from_contains": "x"}), &["mark_read"]);
    let scoped = on_account(all.clone());
    assert_ne!(all.fingerprint(), scoped.fingerprint());
    assert_eq!(
        all.fingerprint(),
        rule_fingerprint(&all.trigger_type, &all.trigger_config, &all.actions),
        "unscoped rules keep the fingerprint they were pushed with"
    );
}

#[test]
fn a_renamed_target_folder_is_a_new_version() {
    let spec = on_account(rule(
        serde_json::json!({"from_contains": "x"}),
        &["move_to_folder:f-billing"],
    ));
    let before = folders();
    let mut after = folders();
    after.insert("f-billing".into(), "INBOX.Invoices".into());
    assert_ne!(spec.fingerprint_for(&before), spec.fingerprint_for(&after));
    let mut gone = folders();
    gone.remove("f-billing");
    assert_ne!(spec.fingerprint_for(&before), spec.fingerprint_for(&gone));
    let no_move = rule(serde_json::json!({"from_contains": "x"}), &["mark_read"]);
    assert_eq!(no_move.fingerprint_for(&before), no_move.fingerprint());
    let extensions = all_extensions();
    let compiled = compile_script_for(
        std::slice::from_ref(&spec),
        &VacationSpec::default(),
        &target(&extensions, &before),
    );
    assert_eq!(compiled.rules[0].1, spec.fingerprint_for(&before));
}

#[test]
fn a_bounce_is_recognized_by_its_envelope_where_the_server_can_tell() {
    let spec = rule(
        serde_json::json!({"from_contains": "x"}),
        &["forward:books@example.test"],
    );
    let mut extensions = all_extensions();
    let (without, requires) = fragment(compile_rule(&spec, &extensions));
    assert!(
        without.contains("header :is \"return-path\" [\"<>\", \"< >\"]"),
        "{without}"
    );
    assert!(!requires.contains("envelope"));
    extensions.push("envelope".into());
    let (with, requires) = fragment(compile_rule(&spec, &extensions));
    assert!(with.contains("envelope :all :is \"from\" \"\""), "{with}");
    assert!(!with.contains("return-path"), "{with}");
    assert!(requires.contains("envelope"));
}

#[test]
fn the_target_with_a_plus_tag_counts_as_the_target() {
    let (text, _) = fragment(compile_rule(
        &rule(
            serde_json::json!({"from_contains": "x"}),
            &["forward:books+in@example.test"],
        ),
        &all_extensions(),
    ));
    assert!(
        text.contains("address :all :matches [\"from\", \"reply-to\"] \"books+*@example.test\""),
        "{text}"
    );
    assert_eq!(escape_wildcards("a*b?c"), "a\\*b\\?c");
    let (odd, _) = fragment(compile_rule(
        &rule(
            serde_json::json!({"from_contains": "x"}),
            &["forward:a*b@example.test"],
        ),
        &all_extensions(),
    ));
    assert!(
        odd.contains(r#"address :all :matches ["from", "reply-to"] "a\\*b+*@example.test""#),
        "a literal * in the address stays literal: {odd}"
    );
}
