use satchel_core::workflow::states::WorkflowState;

#[test]
fn aside_round_trips_and_can_be_entered_or_left_from_every_filing_state() {
    assert_eq!(WorkflowState::Aside.as_db_value(), "aside");
    assert_eq!(
        WorkflowState::from_db_value("aside"),
        Some(WorkflowState::Aside)
    );

    for state in [
        WorkflowState::Inbox,
        WorkflowState::Todo,
        WorkflowState::Waiting,
        WorkflowState::Done,
        WorkflowState::Snoozed,
    ] {
        assert!(state.can_transition_to(WorkflowState::Aside));
        assert!(WorkflowState::Aside.can_transition_to(state));
    }
}
