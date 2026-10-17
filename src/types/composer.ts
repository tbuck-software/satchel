import type { OutgoingMessageInput as WireOutgoingMessageInput } from '../services/backend/generated/app/commands/messages/OutgoingMessageInput';
import type { ProjectedWireFields, RefinedWireFields } from '../services/backend/wireTypes';
import type { OutgoingAttachmentInput as WireOutgoingAttachmentInput } from '../services/backend/generated/app/commands/messages/OutgoingAttachmentInput';
import type { SendMessageResult as WireSendMessageResult } from '../services/backend/generated/app/commands/messages/SendMessageResult';
import type { DraftDto as WireDraftDto } from '../services/backend/generated/app/commands/drafts/DraftDto';
import type { SignatureDto as WireSignatureDto } from '../services/backend/generated/app/commands/signatures/SignatureDto';
import type { SignatureAssetDto as WireSignatureAssetDto } from '../services/backend/generated/app/commands/signatures/SignatureAssetDto';
import type { SignatureAssignmentDto as WireSignatureAssignmentDto } from '../services/backend/generated/app/commands/signatures/SignatureAssignmentDto';
import type { ResolvedSignature as WireResolvedSignature } from '../services/backend/generated/app/commands/signatures/ResolvedSignature';
import type { SignatureSuggestionDto as WireSignatureSuggestionDto } from '../services/backend/generated/app/commands/signatures/SignatureSuggestionDto';
import type { AliasDto as WireAliasDto } from '../services/backend/generated/app/commands/aliases/AliasDto';
import type { TemplateDto as WireTemplateDto } from '../services/backend/generated/app/commands/templates/TemplateDto';
import type { RenderedTemplate as WireRenderedTemplate } from '../services/backend/generated/app/commands/templates/RenderedTemplate';
import type { OptionalWireFields } from '../services/backend/wireTypes';
export type BackendOutgoingAttachment = OptionalWireFields<WireOutgoingAttachmentInput, 'content_id' | 'inline'>;

export interface InlineImagePayload {
  filename: string;
  mimeType: string;
  contentBase64: string;
  dataUrl: string;
}

export interface BackendSendMessagePayload {
  accountId: string;
  to: string[];
  cc?: string[];
  bcc?: string[];
  subject: string;
  body: string;
  bodyHtml?: string | null;
  threadId?: string | null;
  /** Local id of the message being answered; decides In-Reply-To/References. */
  replyToMessageId?: string | null;
  attachments?: BackendOutgoingAttachment[];
  fromAddress?: string | null;
  fromName?: string | null;
  replyTo?: string | null;
  draftId?: string | null;
  scheduledFor?: string | null;
  followUpAt?: string | null;
  smimeSignWithCertId?: string | null;
  smimeEncryptToRecipients?: boolean;
  pgpSignWithKeyId?: string | null;
  pgpEncryptToRecipients?: boolean;
  priority?: 'high' | 'normal' | 'low' | null;
  signatureId?: string | null;
  signatureVariant?: 'auto' | 'long' | 'short' | 'none' | null;
  signatureOverrideText?: string | null;
  signatureOverrideHtml?: string | null;
}

export type BackendSendMessageResult = OptionalWireFields<WireSendMessageResult, 'queued_id' | 'undo_until'>;

// Persisted JSON uses the matching subset of OutgoingMessageInput.
export type BackendDraftSendOptions = Partial<Pick<WireOutgoingMessageInput,
  'reply_to_message_id' | 'follow_up_at' | 'smime_sign_with_cert_id' |
  'smime_encrypt_to_recipients' | 'pgp_sign_with_key_id' | 'pgp_encrypt_to_recipients'
>> & { priority?: 'high' | 'normal' | 'low' | null };

export type BackendDraft = RefinedWireFields<ProjectedWireFields<OptionalWireFields<WireDraftDto, 'send_cancelled' | 'send_options' | 'thread_id' | 'body_html' | 'from_address' | 'from_name' | 'reply_to' | 'scheduled_for' | 'attachments' | 'signature_id' | 'signature_variant' | 'signature_override_text' | 'signature_override_html' | 'revision' | 'remote_error'>, { send_options?: BackendDraftSendOptions | null; attachments?: BackendOutgoingAttachment[] }>, { signature_variant?: 'auto' | 'long' | 'short' | 'none' | null }>;

export interface BackendDraftInput {
  sendOptions?: BackendDraftSendOptions | null;
  id?: string | null;
  accountId: string;
  threadId?: string | null;
  to?: string[];
  cc?: string[];
  bcc?: string[];
  subject?: string;
  body?: string;
  bodyHtml?: string | null;
  fromAddress?: string | null;
  fromName?: string | null;
  replyTo?: string | null;
  scheduledFor?: string | null;
  attachments?: BackendOutgoingAttachment[];
  signatureId?: string | null;
  signatureVariant?: 'auto' | 'long' | 'short' | 'none' | null;
  signatureOverrideText?: string | null;
  signatureOverrideHtml?: string | null;
  /** The revision this editor last saw; the save is refused when it moved on. */
  expectedRevision?: number | null;
}

export type BackendSignature = OptionalWireFields<WireSignatureDto, 'body_html' | 'short_body_html'>;

export type BackendSignatureAsset = OptionalWireFields<WireSignatureAssetDto, 'id'>;

export type BackendSignatureAssignment = RefinedWireFields<OptionalWireFields<WireSignatureAssignmentDto, 'signature_id'>, { first_contact_mode: 'always_full' | 'full_then_short' | 'always_short' }>;

export type BackendResolvedSignature = OptionalWireFields<WireResolvedSignature, 'body_html'>;

export type BackendSignatureSuggestion = OptionalWireFields<WireSignatureSuggestionDto, 'body_html'>;

export type BackendAlias = OptionalWireFields<WireAliasDto, 'display_name'>;

export type BackendTemplate = OptionalWireFields<WireTemplateDto, 'account_id' | 'subject' | 'body_html' | 'shortcut' | 'cc_json' | 'bcc_json' | 'attachments_json'>;

export type BackendRenderedTemplate = OptionalWireFields<WireRenderedTemplate, 'subject' | 'body_html' | 'attachments_json'>;
