import { invoke } from './core';
import { outgoingMessageWire } from './composer';
import type { BackendSendMessagePayload } from '@/types/composer';
import type {
  BackendMergeCsv,
  BackendMergeJob,
  BackendMergeJobAction,
  BackendMergeJobDetail,
  BackendMergeList,
  BackendMergePreview,
  BackendMergeSignature,
  BackendMergeTemplate,
} from '@/types/mailMerge';

export type * from '@/types/mailMerge';

/** Reads a CSV file the user picked; delimiter and encoding are detected. */
export async function backendParseMailMergeCsv(contentBase64: string): Promise<BackendMergeCsv> {
  return invoke<BackendMergeCsv>('parse_mail_merge_csv', { input: { content_base64: contentBase64 } });
}

/**
 * Which rows can be sent, and the message row `index` receives. The same
 * rendering the job uses, so the preview is what goes out.
 */
export async function backendPreviewMailMerge(input: {
  accountId: string | null;
  template: BackendMergeTemplate;
  list: BackendMergeList;
  index: number | null;
  /** The composer's signature choice; the preview then shows what `create` appends. */
  signature?: BackendMergeSignature | null;
}): Promise<BackendMergePreview> {
  return invoke<BackendMergePreview>('preview_mail_merge', {
    input: {
      account_id: input.accountId,
      template: input.template,
      fields: input.list.fields,
      email_field: input.list.emailField,
      rows: input.list.rows,
      index: input.index,
      signature: input.signature ? {
        from_address: input.signature.fromAddress ?? null,
        signature_id: input.signature.signatureId ?? null,
        signature_variant: input.signature.signatureVariant ?? null,
        signature_override_text: input.signature.signatureOverrideText ?? null,
        signature_override_html: input.signature.signatureOverrideHtml ?? null,
      } : null,
    },
  });
}

/** Stores the job and starts sending: one copy per ready row, through the outbox. */
export async function backendCreateMailMergeJob(input: {
  message: BackendSendMessagePayload;
  sourceLabel: string;
  delaySeconds: number;
  list: BackendMergeList;
  /** Same token on a retried start: the backend returns the job it made. */
  requestId: string;
}): Promise<BackendMergeJob> {
  return invoke<BackendMergeJob>('create_mail_merge_job', {
    input: {
      account_id: input.message.accountId,
      message: outgoingMessageWire(input.message),
      source_label: input.sourceLabel,
      delay_seconds: input.delaySeconds,
      fields: input.list.fields,
      email_field: input.list.emailField,
      rows: input.list.rows,
      request_id: input.requestId,
    },
  });
}

export async function backendListMailMergeJobs(): Promise<BackendMergeJob[]> {
  return invoke<BackendMergeJob[]>('list_mail_merge_jobs');
}

export async function backendGetMailMergeJob(id: string): Promise<BackendMergeJobDetail> {
  return invoke<BackendMergeJobDetail>('get_mail_merge_job', { id });
}

/** Pause, resume, cancel, retry failed copies, or delete a finished job. */
export async function backendControlMailMergeJob(
  id: string,
  action: BackendMergeJobAction,
): Promise<BackendMergeJob | null> {
  return invoke<BackendMergeJob | null>('control_mail_merge_job', { id, action });
}
