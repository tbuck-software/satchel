export type { WorkflowState } from '../services/backend/generated/core/workflow/states/WorkflowState';
import type { WorkflowState } from '../services/backend/generated/core/workflow/states/WorkflowState';

export type Priority = 0 | 1 | 2 | 3;

export interface WorkflowTransition {
  from: WorkflowState;
  to: WorkflowState;
  reason: 'manual' | 'rule' | 'snooze_expired' | 'reply_received';
  at: string;
}
