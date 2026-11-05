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
    }
}

fn fragment(compiled: RuleCompilation) -> (String, BTreeSet<String>) {
    match compiled {
        RuleCompilation::Server { fragment, requires } => (fragment, requires),
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
fn body_needs_the_body_extension() {
    let config = serde_json::json!({"body_contains": "unsubscribe"});
    let (text, requires) = fragment(compile_rule(
        &rule(config.clone(), &["mark_read"]),
        &all_extensions(),
    ));
    assert!(
        text.contains("body :text :contains \"unsubscribe\""),
        "{text}"
    );
    assert!(requires.contains("body"));

    let without_body: Vec<String> = all_extensions()
        .into_iter()
        .filter(|ext| ext != "body")
        .collect();
    assert_eq!(
        compile_rule(&rule(config, &["mark_read"]), &without_body),
        RuleCompilation::Local(LocalReason::Extension("body".into()))
    );
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
            rule(serde_json::json!({"body_contains": "a"}), &["mark_read"]),
            second,
            local,
        ],
        &VacationSpec::default(),
        &all_extensions(),
    );
    assert!(compiled.content.starts_with(SCRIPT_MARKER));
    assert!(is_satchel_script(&compiled.content));
    assert!(compiled
        .content
        .contains("require [\"body\", \"imap4flags\"];\r\n"));
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
        "vacation :subject \"Away \\\"now\\\"\" :addresses [\"me@example.com\"] \"Back on Monday.\r\nRegards\";"
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
