import type { BackendDraftInput, BackendDraftSendOptions, BackendOutgoingAttachment } from '@/types/composer';
import { normalizeVisibleText, normalizedAddresses } from './composerNormalization';

export interface ComposerDraftState {
  accountId: string;
  fromAddress: string;
  fromName: string | null;
  threadId: string | null;
  replyTo: string | null;
  sendOptions: BackendDraftSendOptions;
  to: readonly string[];
  cc: readonly string[];
  bcc: readonly string[];
  subject: string;
  body: string;
  bodyHtml: string | null;
  plainText: boolean;
  scheduledFor: string | null;
  signatureId: string | null;
  signatureVariant: 'auto' | 'long' | 'short' | 'none';
  signatureOverrideText: string | null;
  signatureOverrideHtml: string | null;
  attachments: readonly BackendOutgoingAttachment[];
}

function attachmentContentDigest(value: string): string {
  let first = 0x811c9dc5;
  let second = 0x9e3779b9;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    first = Math.imul(first ^ code, 0x01000193);
    second = Math.imul(second ^ code, 0x85ebca6b);
  }
  return [first, second]
    .map((part) => (part >>> 0).toString(16).padStart(8, '0'))
    .join('');
}

const attachmentDigests = new WeakMap<BackendOutgoingAttachment, { content: string; digest: string }>();

function attachmentDigest(attachment: BackendOutgoingAttachment): string {
  const cached = attachmentDigests.get(attachment);
  if (cached?.content === attachment.content_base64) return cached.digest;
  const digest = attachmentContentDigest(attachment.content_base64);
  attachmentDigests.set(attachment, { content: attachment.content_base64, digest });
  return digest;
}

/** Stable representation used by the composer close guard. */
export function composerDraftSnapshot(state: ComposerDraftState): string {
  return JSON.stringify({
    accountId: state.accountId,
    fromAddress: state.fromAddress.toLowerCase(),
    fromName: state.fromName,
    threadId: state.threadId,
    replyTo: state.replyTo,
    sendOptions: state.sendOptions,
    to: normalizedAddresses(state.to),
    cc: normalizedAddresses(state.cc),
    bcc: normalizedAddresses(state.bcc),
    subject: normalizeVisibleText(state.subject),
    body: normalizeVisibleText(state.body),
    bodyHtml: state.bodyHtml,
    plainText: state.plainText,
    scheduledFor: state.scheduledFor,
    signatureId: state.signatureId,
    signatureVariant: state.signatureVariant,
    signatureOverrideText: state.signatureOverrideText,
    signatureOverrideHtml: state.signatureOverrideHtml,
    attachments: state.attachments.map((attachment) => ({
      filename: attachment.filename,
      mimeType: attachment.mime_type,
      inline: Boolean(attachment.inline),
      contentId: attachment.content_id ?? null,
      size: attachment.content_base64.length,
      contentDigest: attachmentDigest(attachment),
    })),
  });
}

/** Whether closing this draft would discard something visible to the sender. */
export function hasMeaningfulComposerContent(state: {
  to?: readonly string[];
  cc?: readonly string[];
  bcc?: readonly string[];
  subject?: string;
  body?: string;
  bodyHtml?: string | null;
  attachments?: readonly BackendOutgoingAttachment[];
}): boolean {
  const html = state.bodyHtml ?? '';
  return normalizedAddresses([...(state.to ?? []), ...(state.cc ?? []), ...(state.bcc ?? [])]).length > 0
    || normalizeVisibleText(state.subject ?? '').length > 0
    || normalizeVisibleText(state.body ?? '').length > 0
    || normalizeVisibleText(html.replace(/<[^>]*>/g, '').replace(/&nbsp;/gi, ' ')).length > 0
    || /<(?:img|hr)\b/i.test(html)
    || (state.attachments?.length ?? 0) > 0;
}

/** The saved representation and close guard share one complete editor state. */
export function composerDraftInput(state: ComposerDraftState): Omit<BackendDraftInput, 'id' | 'expectedRevision'> {
  const { plainText, to, cc, bcc, attachments, ...input } = state;
  return {
    ...input,
    to: [...to], cc: [...cc], bcc: [...bcc], attachments: [...attachments],
    bodyHtml: plainText ? null : input.bodyHtml,
  };
}

/**
 * The close guard's view of a saved draft: the persisted fields only,
 * normalised like `composerDraftSnapshot`. Plain-text mode is not stored on
 * its own — a plain-text draft simply has no HTML body — so it is left out.
 */
export function composerSavedSnapshot(input: Omit<BackendDraftInput, 'id' | 'expectedRevision'>): string {
  return composerDraftSnapshot({
    accountId: input.accountId,
    fromAddress: input.fromAddress ?? '',
    fromName: input.fromName ?? null,
    threadId: input.threadId ?? null,
    replyTo: input.replyTo ?? null,
    sendOptions: input.sendOptions ?? {},
    to: input.to ?? [],
    cc: input.cc ?? [],
    bcc: input.bcc ?? [],
    subject: input.subject ?? '',
    body: input.body ?? '',
    bodyHtml: input.bodyHtml ?? null,
    plainText: false,
    scheduledFor: input.scheduledFor ?? null,
    signatureId: input.signatureId ?? null,
    signatureVariant: input.signatureVariant ?? 'auto',
    signatureOverrideText: input.signatureOverrideText ?? null,
    signatureOverrideHtml: input.signatureOverrideHtml ?? null,
    attachments: input.attachments ?? [],
  });
}

/** Recovery retains delivery policy; attachment bytes remain in the draft row. */
export function composerRecoveryContent({ attachments: _attachments, ...content }: ComposerDraftState) {
  return content;
}

/** Track every persisted field, keeping attachment payloads out of the snapshot. */
export function composerDraftInputSnapshot(input: BackendDraftInput | null): string {
  if (!input) return '';
  return JSON.stringify({
    ...input,
    attachments: input.attachments?.map(attachment => ({
      filename: attachment.filename,
      mime_type: attachment.mime_type,
      inline: attachment.inline,
      content_id: attachment.content_id,
      contentDigest: attachmentDigest(attachment),
    })),
  });
}
