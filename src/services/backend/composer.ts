import type { DeserializeWireInput } from './wireTypes';
import type { DraftInput } from './generated/app/commands/drafts/DraftInput';
import type { OutgoingMessageInput } from './generated/app/commands/messages/OutgoingMessageInput';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type {
  BackendAlias,
  BackendDraft,
  BackendDraftInput,
  BackendOutgoingAttachment,
  BackendRenderedTemplate,
  BackendResolvedSignature,
  BackendSendMessagePayload,
  BackendSendMessageResult,
  BackendSignature,
  BackendSignatureAssignment,
  BackendSignatureSuggestion,
  BackendTemplate,
} from '@/types/composer';

export type {
  BackendAlias,
  BackendDraft,
  BackendDraftInput,
  BackendOutgoingAttachment,
  BackendRenderedTemplate,
  BackendResolvedSignature,
  BackendSendMessagePayload,
  BackendSendMessageResult,
  BackendSignature,
  BackendSignatureAssignment,
  BackendSignatureAsset,
  BackendSignatureSuggestion,
  BackendTemplate,
} from '@/types/composer';


export async function backendSendMessage(
  payload: BackendSendMessagePayload,
): Promise<BackendSendMessageResult> {
  return invoke<BackendSendMessageResult>('send_message', {
    accountId: payload.accountId,
    input: {
      to: payload.to,
      cc: payload.cc ?? [],
      bcc: payload.bcc ?? [],
      subject: payload.subject,
      body: payload.body,
      body_html: payload.bodyHtml ?? null,
      thread_id: payload.threadId ?? null,
      reply_to_message_id: payload.replyToMessageId ?? null,
      attachments: payload.attachments ?? [],
      from_address: payload.fromAddress ?? null,
      from_name: payload.fromName ?? null,
      reply_to: payload.replyTo ?? null,
      draft_id: payload.draftId ?? null,
      scheduled_for: payload.scheduledFor ?? null,
      follow_up_at: payload.followUpAt ?? null,
      smime_sign_with_cert_id: payload.smimeSignWithCertId ?? null,
      smime_encrypt_to_recipients: payload.smimeEncryptToRecipients ?? false,
      pgp_sign_with_key_id: payload.pgpSignWithKeyId ?? null,
      pgp_encrypt_to_recipients: payload.pgpEncryptToRecipients ?? false,
      priority: payload.priority ?? null,
      signature_id: payload.signatureId ?? null,
      signature_variant: payload.signatureVariant ?? null,
      signature_override_text: payload.signatureOverrideText ?? null,
      signature_override_html: payload.signatureOverrideHtml ?? null,
    } satisfies DeserializeWireInput<OutgoingMessageInput, 'core::drafts::outgoing::OutgoingMessageInput'>,
  });
}


/**
 * Original sources of the threads dropped on a composer, one `.eml` per
 * thread. Threads whose source cannot be fetched are missing from the result.
 */
export async function backendThreadEmlAttachments(
  threadIds: string[],
): Promise<BackendOutgoingAttachment[]> {
  return invoke<BackendOutgoingAttachment[]>('thread_eml_attachments', { threadIds });
}


export async function backendSaveDraft(input: BackendDraftInput): Promise<BackendDraft> {
  return invoke<BackendDraft>('save_draft', {
    input: {
      id: input.id ?? null,
      account_id: input.accountId,
      thread_id: input.threadId ?? null,
      to: input.to ?? [],
      cc: input.cc ?? [],
      bcc: input.bcc ?? [],
      subject: input.subject ?? '',
      body: input.body ?? '',
      body_html: input.bodyHtml ?? null,
      from_address: input.fromAddress ?? null,
      from_name: input.fromName ?? null,
      reply_to: input.replyTo ?? null,
      scheduled_for: input.scheduledFor ?? null,
      attachments: input.attachments ?? [],
      signature_id: input.signatureId ?? null,
      signature_variant: input.signatureVariant ?? null,
      signature_override_text: input.signatureOverrideText ?? null,
      signature_override_html: input.signatureOverrideHtml ?? null,
      send_options: input.sendOptions ?? null,
      expected_revision: input.expectedRevision ?? null,
    } satisfies DeserializeWireInput<DraftInput, 'core::drafts::DraftInput'>,
  });
}

/**
 * Claims `draftId` for this window's composer. `false` means another window
 * already edits the draft and was brought forward instead.
 */
export async function backendClaimDraftEditor(draftId: string): Promise<boolean> {
  return invoke<boolean>('claim_draft_editor', { draftId });
}

/** Gives up this window's claim when its composer closes. */
export async function backendReleaseDraftEditor(draftId: string): Promise<void> {
  return invoke<void>('release_draft_editor', { draftId });
}

/** Brings forward the other window that edits `draftId`; `false` when there is none. */
export async function backendFocusDraftEditor(draftId: string): Promise<boolean> {
  return invoke<boolean>('focus_draft_editor', { draftId });
}


export async function backendListDrafts(): Promise<BackendDraft[]> {
  return invoke<BackendDraft[]>('list_drafts');
}


export async function backendLoadDraft(id: string): Promise<BackendDraft | null> {
  return invoke<BackendDraft | null>('load_draft', { id });
}


export async function backendDeleteDraft(id: string): Promise<void> {
  return invoke<void>('delete_draft', { id });
}


export async function backendListSignatures(accountId?: string): Promise<BackendSignature[]> {
  return invoke<BackendSignature[]>('list_signatures', {
    accountId: accountId ?? null,
  });
}


export async function backendUpsertSignature(input: {
  id?: string | null;
  accountId: string;
  name: string;
  bodyText: string;
  bodyHtml?: string | null;
  shortBodyText?: string;
  shortBodyHtml?: string | null;
  assets?: Array<{
    filename: string;
    mimeType: string;
    contentId: string;
    contentBase64: string;
  }>;
  isDefault?: boolean;
}): Promise<BackendSignature> {
  return invoke<BackendSignature>('upsert_signature', {
    input: {
      id: input.id ?? null,
      account_id: input.accountId,
      name: input.name,
      body_text: input.bodyText,
      body_html: input.bodyHtml ?? null,
      short_body_text: input.shortBodyText ?? '',
      short_body_html: input.shortBodyHtml ?? null,
      assets: (input.assets ?? []).map((asset) => ({
        filename: asset.filename,
        mime_type: asset.mimeType,
        content_id: asset.contentId,
        content_base64: asset.contentBase64,
      })),
      is_default: input.isDefault ?? false,
    },
  });
}


export async function backendDeleteSignature(id: string): Promise<void> {
  return invoke<void>('delete_signature', { id });
}


export async function backendListSignatureAssignments(
  accountId?: string,
): Promise<BackendSignatureAssignment[]> {
  return invoke<BackendSignatureAssignment[]>('list_signature_assignments', {
    accountId: accountId ?? null,
  });
}


export async function backendUpsertSignatureAssignment(input: {
  accountId: string;
  identityEmail: string;
  signatureId?: string | null;
  firstContactMode: BackendSignatureAssignment['first_contact_mode'];
}): Promise<BackendSignatureAssignment> {
  return invoke<BackendSignatureAssignment>('upsert_signature_assignment', {
    input: {
      account_id: input.accountId,
      identity_email: input.identityEmail,
      signature_id: input.signatureId ?? null,
      first_contact_mode: input.firstContactMode,
    },
  });
}


export async function backendResolveSignature(input: {
  accountId: string;
  identityEmail: string;
  threadId?: string | null;
  signatureId?: string | null;
  preference: 'auto' | 'long' | 'short' | 'none';
}): Promise<BackendResolvedSignature | null> {
  return invoke<BackendResolvedSignature | null>('resolve_signature', {
    input: {
      account_id: input.accountId,
      identity_email: input.identityEmail,
      thread_id: input.threadId ?? null,
      signature_id: input.signatureId ?? null,
      preference: input.preference,
    },
  });
}


export async function backendDiscoverSignatureSuggestions(input: {
  accountId: string;
  identityEmail: string;
}): Promise<BackendSignatureSuggestion[]> {
  return invoke<BackendSignatureSuggestion[]>('discover_signature_suggestions', {
    accountId: input.accountId,
    identityEmail: input.identityEmail,
  });
}


export async function backendListSignatureSuggestions(
  accountId?: string,
): Promise<BackendSignatureSuggestion[]> {
  return invoke<BackendSignatureSuggestion[]>('list_signature_suggestions', {
    accountId: accountId ?? null,
  });
}


export async function backendSetSignatureSuggestionStatus(
  id: string,
  status: 'accepted' | 'dismissed' | 'pending',
): Promise<void> {
  return invoke<void>('set_signature_suggestion_status', { id, status });
}


export async function backendListAliases(accountId?: string): Promise<BackendAlias[]> {
  return invoke<BackendAlias[]>('list_aliases', { accountId: accountId ?? null });
}


export async function backendUpsertAlias(input: {
  id?: string | null;
  accountId: string;
  email: string;
  displayName?: string | null;
  isDefault?: boolean;
}): Promise<BackendAlias> {
  return invoke<BackendAlias>('upsert_alias', {
    input: {
      id: input.id ?? null,
      account_id: input.accountId,
      email: input.email,
      display_name: input.displayName ?? null,
      is_default: input.isDefault ?? false,
    },
  });
}


export async function backendDeleteAlias(id: string): Promise<void> {
  return invoke<void>('delete_alias', { id });
}


export async function backendListTemplates(accountId?: string): Promise<BackendTemplate[]> {
  return invoke<BackendTemplate[]>('list_templates', { accountId: accountId ?? null });
}


export async function backendUpsertTemplate(input: {
  id?: string | null;
  accountId?: string | null;
  name: string;
  subject?: string | null;
  bodyText: string;
  bodyHtml?: string | null;
  shortcut?: string | null;
  cc?: string[];
  bcc?: string[];
  attachmentsJson?: string | null;
}): Promise<BackendTemplate> {
  return invoke<BackendTemplate>('upsert_template', {
    input: {
      id: input.id ?? null,
      account_id: input.accountId ?? null,
      name: input.name,
      subject: input.subject ?? null,
      body_text: input.bodyText,
      body_html: input.bodyHtml ?? null,
      shortcut: input.shortcut ?? null,
      cc: input.cc ?? [],
      bcc: input.bcc ?? [],
      attachments_json: input.attachmentsJson ?? null,
    },
  });
}


export async function backendDeleteTemplate(id: string): Promise<void> {
  return invoke<void>('delete_template', { id });
}


export async function backendRenderTemplate(input: {
  templateId: string;
  variables: Record<string, string>;
}): Promise<BackendRenderedTemplate> {
  return invoke<BackendRenderedTemplate>('render_template', {
    input: {
      template_id: input.templateId,
      variables: input.variables,
    },
  });
}
