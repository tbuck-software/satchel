use super::compiler::{compile_script, RuleSpec, VacationSpec, SCRIPT_MARKER};
use super::*;
use crate::providers::managesieve_fake::{connected, shared, FakeState};

fn compiled() -> CompiledScript {
    let rule = RuleSpec {
        id: "rule-1".into(),
        name: "Read receipts".into(),
        trigger_type: "on_receive".into(),
        trigger_config: r#"{"from_contains":"noreply@"}"#.into(),
        actions: r#"["mark_read"]"#.into(),
    };
    let extensions = FakeState::default()
        .extensions
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    compile_script(&[rule], &VacationSpec::default(), &extensions)
}

fn with_scripts(scripts: &[(&str, &str)], active: Option<&str>) -> FakeState {
    FakeState {
        scripts: scripts
            .iter()
            .map(|(name, content)| (name.to_string(), content.to_string()))
            .collect(),
        active: active.map(String::from),
        ..FakeState::default()
    }
}

#[tokio::test]
async fn activates_directly_when_nothing_is_active() {
    let state = shared(FakeState::default());
    let mut client = connected(state.clone()).await;
    let outcome = push(&mut client, &compiled(), ActivationChoice::Ask, None)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        PushOutcome::Activated {
            mode: ActivationMode::Direct,
            previous_active: None,
            ..
        }
    ));
    let state = state.lock().unwrap();
    assert_eq!(state.active.as_deref(), Some(OWN_SCRIPT));
    assert!(state.scripts[OWN_SCRIPT].contains("address :all :contains \"from\" \"noreply@\""));
    // Read back before and after; validated before stored.
    let log: Vec<&str> = state.log.iter().map(String::as_str).collect();
    assert_eq!(
        log,
        vec![
            "AUTHENTICATE",
            "LISTSCRIPTS",
            "HAVESPACE",
            "CHECKSCRIPT",
            "PUTSCRIPT",
            "SETACTIVE",
            "LISTSCRIPTS"
        ]
    );
}

#[tokio::test]
async fn asks_before_touching_a_foreign_active_script() {
    let state = shared(with_scripts(&[("roundcube", "keep;")], Some("roundcube")));
    let mut client = connected(state.clone()).await;
    let outcome = push(&mut client, &compiled(), ActivationChoice::Ask, None)
        .await
        .unwrap();
    match outcome {
        PushOutcome::NeedsConsent {
            active_script,
            include_available,
        } => {
            assert_eq!(active_script, "roundcube");
            assert!(include_available);
        }
        other => panic!("unexpected {other:?}"),
    }
    let state = state.lock().unwrap();
    assert_eq!(state.active.as_deref(), Some("roundcube"));
    assert!(!state.scripts.contains_key(OWN_SCRIPT), "nothing written");
    assert!(!state
        .log
        .iter()
        .any(|verb| verb == "PUTSCRIPT" || verb == "SETACTIVE"));
}

#[tokio::test]
async fn the_wrapper_keeps_the_other_script_running() {
    let state = shared(with_scripts(&[("roundcube", "keep;")], Some("roundcube")));
    let mut client = connected(state.clone()).await;
    let outcome = push(&mut client, &compiled(), ActivationChoice::Wrapper, None)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        PushOutcome::Activated { mode: ActivationMode::Wrapper, previous_active: Some(ref name), .. } if name == "roundcube"
    ));
    {
        let state = state.lock().unwrap();
        assert_eq!(state.active.as_deref(), Some(WRAPPER_SCRIPT));
        assert_eq!(state.scripts["roundcube"], "keep;", "untouched");
        let wrapper = &state.scripts[WRAPPER_SCRIPT];
        assert!(wrapper.contains("include :personal \"satchel\";"));
        assert!(wrapper.contains("include :personal \"roundcube\";"));
    }

    // A later background push keeps the wrapper without asking again.
    let mut client = connected(state.clone()).await;
    let outcome = push(
        &mut client,
        &compiled(),
        ActivationChoice::Ask,
        Some("roundcube".into()),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PushOutcome::Activated {
            mode: ActivationMode::Wrapper,
            ..
        }
    ));

    // Turning it off restores the original and removes only Satchel's scripts.
    let mut client = connected(state.clone()).await;
    remove(&mut client, Some("roundcube")).await.unwrap();
    let state = state.lock().unwrap();
    assert_eq!(state.active.as_deref(), Some("roundcube"));
    assert_eq!(state.scripts.keys().collect::<Vec<_>>(), vec!["roundcube"]);
}

#[tokio::test]
async fn the_wrapper_needs_include() {
    let mut fake = with_scripts(&[("main", "keep;")], Some("main"));
    fake.extensions.retain(|ext| *ext != "include");
    let state = shared(fake);
    let mut client = connected(state.clone()).await;
    match push(&mut client, &compiled(), ActivationChoice::Ask, None)
        .await
        .unwrap()
    {
        PushOutcome::NeedsConsent {
            include_available, ..
        } => assert!(!include_available),
        other => panic!("unexpected {other:?}"),
    }
    let error = push(&mut client, &compiled(), ActivationChoice::Wrapper, None)
        .await
        .unwrap_err();
    assert!(matches!(error, PushError::IncludeUnavailable));
}

#[tokio::test]
async fn replacing_records_what_to_restore() {
    let state = shared(with_scripts(&[("main", "keep;")], Some("main")));
    let mut client = connected(state.clone()).await;
    let outcome = push(&mut client, &compiled(), ActivationChoice::Replace, None)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        PushOutcome::Activated { mode: ActivationMode::Direct, previous_active: Some(ref name), .. } if name == "main"
    ));
    assert_eq!(state.lock().unwrap().active.as_deref(), Some(OWN_SCRIPT));

    // Re-pushing keeps knowing what to restore.
    let mut client = connected(state.clone()).await;
    let outcome = push(
        &mut client,
        &compiled(),
        ActivationChoice::Ask,
        Some("main".into()),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        PushOutcome::Activated { previous_active: Some(ref name), .. } if name == "main"
    ));

    let mut client = connected(state.clone()).await;
    remove(&mut client, Some("main")).await.unwrap();
    let state = state.lock().unwrap();
    assert_eq!(state.active.as_deref(), Some("main"));
    assert!(!state.scripts.contains_key(OWN_SCRIPT));
}

#[tokio::test]
async fn a_foreign_script_named_satchel_is_never_overwritten() {
    let state = shared(with_scripts(&[(OWN_SCRIPT, "# mine\r\nkeep;")], None));
    let mut client = connected(state.clone()).await;
    let error = push(&mut client, &compiled(), ActivationChoice::Replace, None)
        .await
        .unwrap_err();
    assert!(matches!(error, PushError::NameTaken(ref name) if name == OWN_SCRIPT));
    assert_eq!(error.message_key(), "backend.errors.serverRules.nameTaken");
    assert_eq!(state.lock().unwrap().scripts[OWN_SCRIPT], "# mine\r\nkeep;");

    // Removal leaves it alone too, even while it is active.
    let active = shared(with_scripts(&[(OWN_SCRIPT, "# mine")], Some(OWN_SCRIPT)));
    let mut client = connected(active.clone()).await;
    remove(&mut client, None).await.unwrap();
    let active = active.lock().unwrap();
    assert_eq!(active.active.as_deref(), Some(OWN_SCRIPT));
    assert_eq!(active.scripts[OWN_SCRIPT], "# mine");
}

#[tokio::test]
async fn a_script_the_server_rejects_is_not_stored() {
    let state = shared(FakeState::default());
    let mut client = connected(state.clone()).await;
    let mut bad = compiled();
    bad.content = format!("{SCRIPT_MARKER}\r\nSYNTAX-ERROR;\r\n");
    let error = push(&mut client, &bad, ActivationChoice::Ask, None)
        .await
        .unwrap_err();
    assert_eq!(error.message_key(), "backend.errors.serverRules.refused");
    let state = state.lock().unwrap();
    assert!(state.scripts.is_empty());
    assert!(state.active.is_none());
}

#[tokio::test]
async fn quota_is_checked_before_writing() {
    let state = shared(FakeState {
        quota: 10,
        ..FakeState::default()
    });
    let mut client = connected(state.clone()).await;
    let error = push(&mut client, &compiled(), ActivationChoice::Ask, None)
        .await
        .unwrap_err();
    assert_eq!(error.message_key(), "backend.errors.serverRules.quota");
    assert!(state.lock().unwrap().scripts.is_empty());
}

#[tokio::test]
async fn servers_without_version_skip_checkscript() {
    let state = shared(FakeState {
        version: false,
        ..FakeState::default()
    });
    let mut client = connected(state.clone()).await;
    push(&mut client, &compiled(), ActivationChoice::Ask, None)
        .await
        .unwrap();
    assert!(!state
        .lock()
        .unwrap()
        .log
        .iter()
        .any(|verb| verb == "CHECKSCRIPT"));
}

#[tokio::test]
async fn removal_without_a_previous_script_deactivates() {
    let state = shared(FakeState::default());
    let mut client = connected(state.clone()).await;
    push(&mut client, &compiled(), ActivationChoice::Ask, None)
        .await
        .unwrap();
    let mut client = connected(state.clone()).await;
    remove(&mut client, None).await.unwrap();
    let state = state.lock().unwrap();
    assert!(state.active.is_none());
    assert!(state.scripts.is_empty());
}
