import type { ParsedCsv } from '../services/backend/generated/core/mail_merge/ParsedCsv';
import type { MergeRecipientInput } from '../services/backend/generated/core/mail_merge/MergeRecipientInput';
import type { MergeTemplate } from '../services/backend/generated/core/mail_merge/MergeTemplate';
import type { RowIssue } from '../services/backend/generated/core/mail_merge/RowIssue';
import type { MailMergeJobSummary } from '../services/backend/generated/core/mail_merge/MailMergeJobSummary';
import type { MailMergeJobDetail } from '../services/backend/generated/core/mail_merge/MailMergeJobDetail';
import type { MailMergeRecipientRow } from '../services/backend/generated/core/mail_merge/MailMergeRecipientRow';
import type { MailMergePreview } from '../services/backend/generated/app/commands/mail_merge/MailMergePreview';
import type { MailMergeJobAction } from '../services/backend/generated/app/commands/mail_merge/MailMergeJobAction';

/** A recipient list read from a CSV file: field names, rows, and the address column. */
export type BackendMergeCsv = ParsedCsv;
export type BackendMergeRecipient = MergeRecipientInput;
export type BackendMergeTemplate = MergeTemplate;
export type BackendMergeRowIssue = RowIssue;
export type BackendMergePreview = MailMergePreview;
export type BackendMergeJob = MailMergeJobSummary;
export type BackendMergeJobDetail = MailMergeJobDetail;
export type BackendMergeJobRecipient = MailMergeRecipientRow;
export type BackendMergeJobAction = MailMergeJobAction;

export interface BackendMergeList {
  fields: string[];
  emailField: string;
  rows: BackendMergeRecipient[];
}
