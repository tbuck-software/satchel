/** SQLite rows used by the local mail rendering tools. */
export interface MailAccount {
  provider: string;
  config: string | null;
}

export interface MailMessage extends MailAccount {
  id: string;
  thread_id: string;
  subject: string;
  from_address: string;
  from_name: string;
  date: string;
  body_text: string;
  body_html: string;
  message_remote_id: string;
}

export interface Attachment {
  id: string;
  filename: string;
  content_type: string;
  content_id: string | null;
  cached_path: string | null;
  remote_uid: string | null;
  remote_part_id: string | null;
  message_remote_id?: string;
  size?: number;
  is_inline?: number;
}

export interface RendererAttachment {
  id: string;
  filename: string;
  contentType: string;
  size: number;
  isInline: boolean;
  contentId: string | null;
  cachedPath: string | null;
  dataUrl: string | null;
}

export interface FidelityPayload {
  id: string;
  label: string;
  subject: string;
  fromAddress: string;
  fromName: string;
  date: string;
  bodyHtml?: string;
  bodyText: string;
  normalizeQuotes: boolean;
  attachments: RendererAttachment[];
  meta: string[];
}

export interface FidelityCase {
  key: string;
  label: string;
  where: string;
  normalizeQuotes?: boolean;
}

export interface SelectedCase extends FidelityCase {
  payload: FidelityPayload;
}

export interface FidelityResult {
  key: string;
  label: string;
  subject: string;
  payload: string;
  reference: string;
  screenshot: string;
  issues: string[];
  baseline: string | null;
  baseline_accepted: boolean;
}

export interface FidelityReport {
  generated_at: string;
  renderer: string;
  db: string;
  results: FidelityResult[];
  missing_cases: Array<{ key: string, label: string }>;
}
