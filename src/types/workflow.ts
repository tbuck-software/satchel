export type { WorkflowState } from '../services/backend/generated/core/workflow/states/WorkflowState';
import type { WorkflowState } from '../services/backend/generated/core/workflow/states/WorkflowState';

/**
 * What a workflow list shows: one state, everything, or `later` — snoozed
 * mail and mail waiting for a reply in one view. `later` is a list scope the
 * backend understands (`workflow_state: 'later'`, `in:later`), not a state a
 * thread can be in.
 */
export type WorkflowFilter = WorkflowState | 'all' | 'later';

/** The states the Later view gathers, in the order its sections appear. */
export const LATER_STATES: readonly WorkflowState[] = ['snoozed', 'waiting'];

export function matchesWorkflowFilter(state: WorkflowState, filter: WorkflowFilter): boolean {
  if (filter === 'all') return true;
  if (filter === 'later') return LATER_STATES.includes(state);
  return state === filter;
}

export type Priority = 0 | 1 | 2 | 3;

export interface WorkflowTransition {
  from: WorkflowState;
  to: WorkflowState;
  reason: 'manual' | 'rule' | 'snooze_expired' | 'reply_received';
  at: string;
}
