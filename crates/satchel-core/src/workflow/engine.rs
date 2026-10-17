use thiserror::Error;

use super::states::WorkflowState;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("invalid transition from {0:?} to {1:?}")]
    InvalidTransition(WorkflowState, WorkflowState),
}

#[derive(Debug, Default)]
pub struct WorkflowEngine;

impl WorkflowEngine {
    pub fn validate_transition(
        &self,
        from: WorkflowState,
        to: WorkflowState,
    ) -> Result<(), WorkflowError> {
        if from.can_transition_to(to) {
            Ok(())
        } else {
            Err(WorkflowError::InvalidTransition(from, to))
        }
    }
}
