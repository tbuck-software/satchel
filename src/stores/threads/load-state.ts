import { ref, type Ref } from 'vue';
import type { BackendListThreadsInput, BackendSmartInboxSummary } from '@/services/backend';
import type { WorkflowState } from '@/types/workflow';
import type { MailboxFilter } from '@/utils/threadList';

/**
 * Load state shared by Filter+Buckets and Persistence.
 *
 * Persistence writes it, Filter+Buckets reads it. Owning it outside both is
 * what lets the façade build them in one order — before, each needed the
 * other to exist first, and the gap was bridged by getters that threw.
 */
export interface ThreadLoadState {
  threadLoadInFlight: Ref<boolean>;
  smartInboxSummary: Ref<BackendSmartInboxSummary | null>;
  /** The scope `smartInboxSummary` was fetched for. */
  smartInboxSummaryInputKey: Ref<string>;
}

export function createThreadLoadState(): ThreadLoadState {
  return {
    threadLoadInFlight: ref(false),
    smartInboxSummary: ref<BackendSmartInboxSummary | null>(null),
    smartInboxSummaryInputKey: ref(''),
  };
}

/** What a thread list query is scoped to. */
export interface ThreadListScope {
  accountId: string;
  activeContactEmail: string;
  workflowFilter: WorkflowState | 'all';
  mailboxFilter: MailboxFilter;
  activeSmartBucketId: string;
}

/** The backend list request for a scope. */
export function threadListInput(scope: ThreadListScope, limit: number): BackendListThreadsInput {
  const input: BackendListThreadsInput = { limit };

  if (scope.accountId) {
    input.account_id = scope.accountId;
  }

  if (scope.activeContactEmail) {
    input.contact_email = scope.activeContactEmail;
  }

  if (scope.workflowFilter !== 'all') {
    input.workflow_state = scope.workflowFilter;
  }

  if (!scope.activeContactEmail && scope.mailboxFilter.kind !== 'all') {
    input.mailbox_kind = scope.mailboxFilter.kind;
    input.mailbox_value = scope.mailboxFilter.value;
  }

  if (scope.activeSmartBucketId.startsWith('bucket-')) {
    input.smart_bucket = scope.activeSmartBucketId.slice('bucket-'.length) as BackendListThreadsInput['smart_bucket'];
  }

  return input;
}

/**
 * Identity of the smart inbox summary for a scope.
 *
 * Opening one smart bucket adds `smart_bucket` to the row query, but the
 * Inbox summary itself is still the same account/mailbox metadata. Keeping
 * that field out of this key lets the clicked bucket resolve while its rows
 * are loading; account and mailbox switches still invalidate immediately.
 */
export function smartInboxSummaryScopeKey(scope: ThreadListScope): string {
  const input = threadListInput(scope, 0);
  return JSON.stringify({
    account_id: input.account_id ?? '',
    contact_email: input.contact_email ?? '',
    workflow_state: input.workflow_state ?? '',
    mailbox_kind: input.mailbox_kind ?? '',
    mailbox_value: input.mailbox_value ?? '',
  });
}
