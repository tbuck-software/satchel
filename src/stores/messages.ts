import { backendErrorLabel } from '@/utils/backendErrorLabels';
import {
  cidCandidates,
  cleanCid,
  isImageMimeType,
  missingInlineImageSource,
  pendingInlineImageCount,
  referencedInlineCids,
  rewriteInlineAttachmentSources,
} from '@/utils/emailInlineImages';
import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import type {
  Message,
  MessagePgpStatus,
  MessageSmimeStatus,
  PgpMessageState,
  SmimeMessageState,
} from '@/types/mail';
import {
  backendDownloadAttachment,
  backendListAttachmentsForMessages,
  backendListMessageBodies,
  backendLoadMessageBody,
  backendLoadThreadMessages,
  backendListMessagesPreview,
  backendCommandsAvailable,
  backendUnsubscribe,
  backendGetMessageSource,
  backendChooseSavePath,
  backendExportMessage,
  type BackendAttachmentDto,
  type BackendDraft,
  type BackendMessage,
  type BackendMessageBody,
  type BackendMessageSource,
  type BackendUnsubscribeResult,
} from '@/services/backend';
import { perfLog, perfStart } from '@/utils/perf';
import { useAccountsStore } from '@/stores/accounts';
import { isOwnEmail } from '@/utils/mailIdentity';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';
import { useConnectionStore } from '@/stores/connection';
import { t } from '@/i18n/runtime';
import { sanitizeThreadFilename } from '@/utils/threadExport';
import { createCoalescingRunner, type CoalescingRunner } from '@/utils/coalescingRunner';

function normalizeLineBreaks(input: string): string {
  return input.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
}

function cleanDisplayBody(input: string): string {
  return normalizeLineBreaks(input).trim();
}
function parseSourcePayload(raw?: string | null): Message['sourcePayload'] {
  if (!raw) return undefined;
  try {
    return JSON.parse(raw) as Message['sourcePayload'];
  } catch {
    return undefined;
  }
}

const pgpStates: PgpMessageState[] = [
  'encrypted_no_key',
  'key_available_locked',
  'decrypting',
  'decrypted',
  'signature_valid',
  'signature_partial',
  'signature_identity_mismatch',
  'signature_invalid',
  'signature_unknown_key',
  'error',
];

const smimeStates: SmimeMessageState[] = [
  'encrypted_no_cert',
  'cert_available_locked',
  'decrypting',
  'decrypted',
  'signature_valid',
  'signature_partial',
  'signature_unchained',
  'signature_identity_mismatch',
  'signature_invalid',
  'error',
];

function mapPgpStatus(row: {
  pgp_state?: string | null;
  pgp_error_detail?: string | null;
  pgp_signer_name?: string | null;
  pgp_signer_fingerprint?: string | null;
}): MessagePgpStatus | undefined {
  const stateRaw = (row.pgp_state || '').trim();
  if (!pgpStates.includes(stateRaw as PgpMessageState)) {
    return undefined;
  }

  return {
    state: stateRaw as PgpMessageState,
    errorDetail: row.pgp_error_detail || undefined,
    signerName: row.pgp_signer_name || undefined,
    signerFingerprint: row.pgp_signer_fingerprint || undefined,
  };
}

function mapSmimeStatus(row: {
  smime_state?: string | null;
  smime_error_detail?: string | null;
  smime_signer_email?: string | null;
  smime_signer_cn?: string | null;
}): MessageSmimeStatus | undefined {
  const stateRaw = (row.smime_state || '').trim();
  if (!smimeStates.includes(stateRaw as SmimeMessageState)) {
    return undefined;
  }
  return {
    state: stateRaw as SmimeMessageState,
    errorDetail: row.smime_error_detail || undefined,
    signerEmail: row.smime_signer_email || undefined,
    signerCn: row.smime_signer_cn || undefined,
  };
}

function withoutInvalidContent(message: Message): Message {
  if (!message.contentError) return message;
  return {
    ...message,
    bodyText: '',
    bodyHtml: undefined,
    sourcePayload: undefined,
    bodyLoaded: true,
    bodyLoadError: backendErrorLabel(message.contentError),
    attachments: [],
    pgp: undefined,
    smime: undefined,
    inlineImagesPending: 0,
    inlineImagesError: undefined,
  };
}

function mapBackendMessage(row: BackendMessage, threadId: string): Message {
  const pgpStatus = mapPgpStatus(row);
  const smimeStatus = mapSmimeStatus(row);
  const rawBodyText = (row.body_text || '').trim();
  const bodyTextCandidate = pgpStatus || smimeStatus
    ? normalizeLineBreaks(rawBodyText)
    : cleanDisplayBody(rawBodyText);
  const snippetCandidate = cleanDisplayBody((row.snippet || '').trim());
  const fallbackText = bodyTextCandidate || snippetCandidate || row.subject;
  const htmlCandidate = (row.body_html || '').trim();

  return withoutInvalidContent({
    id: row.id,
    remoteUidValidity: row.remote_uidvalidity ?? null,
    contentError: row.content_error ?? null,
    threadId: row.thread_id || threadId,
    from: {
      name: row.from_name || t('common.unknown'),
      email: row.from_address || 'unknown@satchel.local',
    },
    to:
      row.to_addresses.length > 0
        ? row.to_addresses.map((email) => ({ name: email, email }))
        : [{ name: t('common.you'), email: 'you@satchel.local' }],
    cc:
      row.cc_addresses.length > 0
        ? row.cc_addresses.map((email) => ({ name: email, email }))
        : undefined,
    replyTo: row.reply_to || undefined,
    subject: row.subject,
    bodyText: fallbackText,
    bodyHtml: htmlCandidate || undefined,
    bodyLoaded: row.body_loaded,
    bodyLoadError: row.content_error ? backendErrorLabel(row.content_error) : undefined,
    timestamp: row.date || new Date().toISOString(),
    isRead: row.is_read,
    isDraft: row.is_draft,
    attachments: [],
    pgp: pgpStatus,
    smime: smimeStatus,
    listUnsubscribe: row.list_unsubscribe ?? null,
    listUnsubscribePost: row.list_unsubscribe_post ?? null,
    authSpf: row.auth_spf ?? null,
    authDkim: row.auth_dkim ?? null,
    authDmarc: row.auth_dmarc ?? null,
    sourcePayload: parseSourcePayload(row.source_payload),
  });
}

/**
 * The body keeps its `cid:` references; the bytes live once, on the
 * attachment, and the renderer resolves them. A refreshed body therefore keeps
 * its images without downloading them again.
 */
function applyMessageBody(message: Message, body: BackendMessageBody): Message {
  const pgpStatus = mapPgpStatus(body);
  const smimeStatus = mapSmimeStatus(body);
  const rawBodyText = (body.body_text || '').trim();
  const bodyTextCandidate = pgpStatus || smimeStatus
    ? normalizeLineBreaks(rawBodyText)
    : cleanDisplayBody(rawBodyText);
  const snippetCandidate = cleanDisplayBody((body.snippet || '').trim());
  const fallbackText = bodyTextCandidate || snippetCandidate || message.subject;
  const htmlCandidate = (body.body_html || '').trim();

  return withoutInvalidContent({
    ...message,
    remoteUidValidity: body.remote_uidvalidity ?? null,
    contentError: body.content_error ?? null,
    sourcePayload: parseSourcePayload(body.source_payload) ?? message.sourcePayload,
    bodyText: fallbackText,
    bodyHtml: htmlCandidate || undefined,
    bodyLoaded: body.body_loaded,
    bodyLoadError: body.content_error ? backendErrorLabel(body.content_error) : undefined,
    pgp: pgpStatus,
    smime: smimeStatus,
    // Previews leave these empty; the body is where the backend sends them.
    listUnsubscribe: body.list_unsubscribe ?? message.listUnsubscribe ?? null,
    listUnsubscribePost: body.list_unsubscribe_post ?? message.listUnsubscribePost ?? null,
    authSpf: body.auth_spf ?? null,
    authDkim: body.auth_dkim ?? null,
    authDmarc: body.auth_dmarc ?? null,
  });
}

function changedGeneration(generation: number | null | undefined, existing?: Message, contentError?: string | null): boolean {
  return Boolean(existing) && ((generation ?? null) !== (existing?.remoteUidValidity ?? null)
    || (contentError ?? null) !== (existing?.contentError ?? null));
}

/**
 * Refresh header metadata without throwing away a body that is already on
 * screen. Preview rows deliberately omit heavy body and attachment data.
 */
function mergeMessagePreview(preview: Message, existing?: Message): Message {
  if (!existing || preview.contentError || changedGeneration(preview.remoteUidValidity, existing, preview.contentError)) return preview;

  const stableAttachments = existing.attachments.length > 0
    ? existing.attachments
    : preview.attachments;
  if (existing.bodyLoaded === false || preview.bodyLoaded !== false) {
    return { ...preview, attachments: stableAttachments };
  }

  return {
    ...preview,
    bodyText: existing.bodyText,
    bodyHtml: existing.bodyHtml,
    bodyLoaded: existing.bodyLoaded,
    bodyLoadError: existing.bodyLoadError,
    pgp: existing.pgp,
    smime: existing.smime,
    listUnsubscribe: existing.listUnsubscribe ?? preview.listUnsubscribe,
    listUnsubscribePost: existing.listUnsubscribePost ?? preview.listUnsubscribePost,
    authSpf: existing.authSpf,
    authDkim: existing.authDkim,
    authDmarc: existing.authDmarc,
    sourcePayload: existing.sourcePayload ?? preview.sourcePayload,
    attachments: stableAttachments,
    inlineImagesPending: existing.inlineImagesPending,
    inlineImagesError: existing.inlineImagesError,
  };
}

function mapAttachment(row: BackendAttachmentDto): Message['attachments'][number] {
  return {
    id: row.id,
    filename: row.filename || t('thread.attachments.filenameFallback'),
    contentType: row.content_type || 'application/octet-stream',
    size: row.size ?? 0,
    isInline: Boolean(row.is_inline),
    contentId: row.content_id ?? null,
    cachedPath: row.cached_path ?? null,
  };
}

/**
 * The body with its inline images substituted, for callers that hand the HTML
 * on (resend). A `cid:` reference without bytes keeps its URL; one without any
 * attachment becomes the placeholder.
 */
export function bodyHtmlWithInlineImages(message: Message): string | undefined {
  if (!message.bodyHtml) return undefined;
  return rewriteInlineAttachmentSources(message.bodyHtml, message.attachments, {
    attachmentsResolved: true,
    missingSource: missingInlineImageSource(t('errors.inlineImageUnavailable')),
  }).html;
}

/**
 * Whether a part has to be fetched to render the body. A reference from
 * `<img src="cid:…">` outranks the metadata: senders routinely ship signature
 * logos as `Content-Disposition: attachment` or `application/octet-stream`,
 * and trusting the flags alone left those images blank.
 */
function isNeededInlineImage(
  attachment: Message['attachments'][number],
  referencedCids: Set<string>,
): boolean {
  const cidReferenced = cidCandidates(attachment)
    .some((candidate) => referencedCids.has(cleanCid(candidate).toLowerCase()));
  if (cidReferenced) {
    return true;
  }
  return Boolean(attachment.isInline)
    && isImageMimeType(attachment.contentType);
}

type InlineHydration = {
  attachment: Message['attachments'][number];
  error?: string;
  generationMismatch?: boolean;
};

async function hydrateInlineAttachment(
  attachment: Message['attachments'][number],
  referencedCids: Set<string>,
  generation: number | null,
): Promise<InlineHydration> {
  if (!isNeededInlineImage(attachment, referencedCids)) {
    return { attachment };
  }
  if (!attachment.contentId && !attachment.filename) {
    return { attachment };
  }

  try {
    const downloaded = await backendDownloadAttachment(attachment.id);
    if ((downloaded.remote_uidvalidity ?? null) !== generation) {
      return { attachment, generationMismatch: true };
    }
    const resolvedContentType = downloaded.mime_type === 'application/octet-stream'
      && isImageMimeType(attachment.contentType)
      ? attachment.contentType : downloaded.mime_type || attachment.contentType;
    if (!downloaded.data_base64 || !isImageMimeType(resolvedContentType)) {
      return { attachment, error: t('errors.attachmentEmpty') };
    }
    return {
      attachment: {
        ...attachment,
        filename: downloaded.filename || attachment.filename,
        contentType: resolvedContentType,
        cachedPath: downloaded.cached_path ?? attachment.cachedPath,
        // Data URLs are owned by the message cache and become collectible when
        // its thread entry is evicted. Avoid object URLs here: unreleased
        // WebKit blob allocations caused the previous memory regression.
        dataUrl: `data:${resolvedContentType};base64,${downloaded.data_base64}`,
      },
    };
  } catch (error) {
    return {
      attachment,
      error: backendErrorLabel(error),
    };
  }
}

function pause(ms: number): Promise<void> {
  return new Promise((resolve) => {
    globalThis.setTimeout(resolve, ms);
  });
}

const MESSAGE_BODY_LOAD_TIMEOUT_MS = 15_000;
const REMOTE_MESSAGE_BODY_LOAD_TIMEOUT_MS = 30_000;
/** How long a thread nobody shows stays cached; t3code keeps its snapshots as long. */
export const THREAD_RETENTION_IDLE_MS = 5 * 60_000;
const MAX_CACHED_BODY_CHARS = 12_000_000;
/** Latest messages whose bodies the open call brings along; one more covers an own reply. */
const OPEN_BODY_COUNT = 2;

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeoutId = globalThis.setTimeout(() => reject(new Error(message)), timeoutMs);
    promise.then(
      (value) => {
        globalThis.clearTimeout(timeoutId);
        resolve(value);
      },
      (error) => {
        globalThis.clearTimeout(timeoutId);
        reject(error);
      },
    );
  });
}

interface RetainedThread {
  lastUsedAt: number;
  /** Readers showing it right now; a held thread is never evicted. */
  holders: number;
  /** A mail refresh happened since the last load. */
  stale: boolean;
  /** Remote bodies and inline images were requested, not only local rows. */
  complete: boolean;
}

export const useMessagesStore = defineStore('messages', () => {
  const accountsStore = useAccountsStore();
  const connectionStore = useConnectionStore();
  const messagesByThread = ref<Record<string, Message[]>>({});
  // Opaque identities are never reused, even after eviction and reopening.
  // Entries live only as long as their cached thread, not its browsing history.
  const threadLoadVersions = new Map<string, symbol>();
  const loadingThreadIds = ref<string[]>([]);
  const loadingBodyMessageIds = ref<string[]>([]);
  const threadLoadPromises = new Map<string, Promise<void>>();
  const threadRefreshRunners = new Map<string, CoalescingRunner<void>>();
  const hydratedAttachmentMessageIds = new Set<string>();
  const hydratingAttachmentMessageIds = new Map<string, { version: symbol }>();
  const bodyRequestTokens = new Map<string, { version: symbol }>();
  const detailsNeedingRevalidation = new Set<string>();
  const revalidationAttempts = new Map<string, string>();
  const demandedDetails = new Set<string>();
  const retained = new Map<string, RetainedThread>();
  let sweepTimer: ReturnType<typeof setTimeout> | null = null;

  function revalidateMessageReference(threadId: string, messageId: string) {
    const message = messagesByThread.value[threadId]?.find(message => message.id === messageId);
    if (!message) return;
    const key = JSON.stringify([message.remoteUidValidity ?? null, message.contentError ?? null]);
    if (revalidationAttempts.get(messageId) === key) {
      patchMessage(threadId, messageId, current => ({ ...current, bodyLoaded: true,
        bodyLoadError: t('errors.messageReferenceChanged') }));
      return;
    }
    revalidationAttempts.set(messageId, key);
    detailsNeedingRevalidation.add(messageId);
    void refreshThread(threadId);
  }

  function forgetDetailsOnGenerationChange(generation: number | null | undefined, existing?: Message, contentError?: string | null) {
    if (!existing || !changedGeneration(generation, existing, contentError)) return;
    hydratedAttachmentMessageIds.delete(existing.id);
    hydratingAttachmentMessageIds.delete(existing.id);
    bodyRequestTokens.delete(existing.id);
    loadingBodyMessageIds.value = loadingBodyMessageIds.value.filter(id => id !== existing.id);
  }

  function mergePreview(preview: Message, existing?: Message): Message {
    if (!existing) {
      hydratedAttachmentMessageIds.delete(preview.id);
      hydratingAttachmentMessageIds.delete(preview.id);
    }
    forgetDetailsOnGenerationChange(preview.remoteUidValidity, existing, preview.contentError);
    return mergeMessagePreview(preview, existing);
  }

  const totalMessages = computed(() =>
    Object.values(messagesByThread.value).reduce((sum, messages) => sum + messages.length, 0),
  );

  function getByThread(threadId: string): Message[] {
    return messagesByThread.value[threadId] ?? [];
  }

  function messageBodyChars(message: Message): number {
    return message.bodyText.length
      + (message.bodyHtml?.length ?? 0)
      + message.attachments.reduce((sum, attachment) => sum + (attachment.dataUrl?.length ?? 0), 0);
  }

  function nextVersion(threadId: string): symbol {
    const version = Symbol();
    threadLoadVersions.set(threadId, version);
    return version;
  }

  function evictThread(threadId: string) {
    retained.delete(threadId);
    const evictedMessages = messagesByThread.value[threadId] ?? [];
    threadLoadVersions.delete(threadId);
    const evictedIds = new Set(evictedMessages.map((message) => message.id));
    for (const id of evictedIds) {
      hydratedAttachmentMessageIds.delete(id);
      hydratingAttachmentMessageIds.delete(id);
      bodyRequestTokens.delete(id);
      detailsNeedingRevalidation.delete(id);
      revalidationAttempts.delete(id);
      demandedDetails.delete(id);
    }
    loadingBodyMessageIds.value = loadingBodyMessageIds.value.filter((id) => !evictedIds.has(id));
    delete messagesByThread.value[threadId];
  }

  function entryFor(threadId: string): RetainedThread {
    let entry = retained.get(threadId);
    if (!entry) {
      entry = { lastUsedAt: Date.now(), holders: 0, stale: false, complete: false };
      retained.set(threadId, entry);
    }
    return entry;
  }

  function touchThreadCache(threadId: string) {
    entryFor(threadId).lastUsedAt = Date.now();
    scheduleSweep();
  }

  function evictable(threadId: string, protectedThreadId?: string): boolean {
    const entry = retained.get(threadId);
    return threadId !== protectedThreadId
      && (entry?.holders ?? 0) === 0
      && !threadLoadPromises.has(threadId);
  }

  function cachedBodyChars(): number {
    return [...retained.keys()].reduce(
      (total, threadId) => total + (messagesByThread.value[threadId] ?? [])
        .reduce((threadTotal, message) => threadTotal + messageBodyChars(message), 0),
      0,
    );
  }

  /** Least recently used first, until the cache fits its byte budget again. */
  function enforceCacheBudget(protectedThreadId?: string) {
    const byAge = [...retained.entries()]
      .sort(([, left], [, right]) => left.lastUsedAt - right.lastUsedAt)
      .map(([threadId]) => threadId);
    for (const threadId of byAge) {
      if (cachedBodyChars() <= MAX_CACHED_BODY_CHARS) return;
      if (evictable(threadId, protectedThreadId)) evictThread(threadId);
    }
  }

  function sweepExpired() {
    sweepTimer = null;
    const now = Date.now();
    for (const [threadId, entry] of retained.entries()) {
      if (now - entry.lastUsedAt >= THREAD_RETENTION_IDLE_MS && evictable(threadId)) {
        evictThread(threadId);
      }
    }
    scheduleSweep();
  }

  function scheduleSweep() {
    if (sweepTimer !== null) return;
    let next = Infinity;
    for (const [threadId, entry] of retained) {
      if (evictable(threadId)) next = Math.min(next, entry.lastUsedAt + THREAD_RETENTION_IDLE_MS);
    }
    if (next === Infinity) return;
    sweepTimer = setTimeout(sweepExpired, Math.max(0, next - Date.now()) + 50);
  }

  /** Keep a thread while a reader shows it; the release starts its idle clock. */
  function retainThread(threadId: string): () => void {
    const entry = entryFor(threadId);
    entry.holders += 1;
    entry.lastUsedAt = Date.now();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const current = retained.get(threadId);
      if (!current) return;
      current.holders = Math.max(0, current.holders - 1);
      current.lastUsedAt = Date.now();
      scheduleSweep();
    };
  }

  function hasThread(threadId: string): boolean {
    return retained.has(threadId) && messagesByThread.value[threadId] !== undefined;
  }

  /** After a mail refresh: cached threads reload their previews on the next open. */
  function markThreadsStale(exceptThreadId?: string) {
    for (const [threadId, entry] of retained) {
      if (threadId !== exceptThreadId) entry.stale = true;
    }
  }

  /** Reprocess loaded PGP bodies after key or plugin settings change. */
  async function reloadPgpMessages(onlyThreadId?: string): Promise<void> {
    const threadIds = onlyThreadId ? [onlyThreadId] : [...retained.keys()];
    await Promise.all(threadIds.map(async (threadId) => {
      // A snapshot taken before the key changed must finish before the reread.
      while (threadLoadPromises.has(threadId)) await threadLoadPromises.get(threadId);
      const ids = (messagesByThread.value[threadId] ?? [])
        .filter((message) => message.bodyLoaded !== false && (
          message.pgp || message.bodyText.includes('-----BEGIN PGP ')
        ))
        .map((message) => message.id);
      if (ids.length === 0) return;
      await trackLoad(threadId, async () => {
        const version = nextVersion(threadId);
        await hydrateMessageBodies(threadId, ids, version);
        if (threadLoadVersions.get(threadId) !== version) return;
        // Decryption may reveal attachments that were absent from the ciphertext.
        for (const id of ids) hydratedAttachmentMessageIds.delete(id);
        await hydrateMessageAttachments(threadId, ids, version);
      }, false);
    }));
  }

  function cacheStats() {
    const cachedMessages = [...retained.keys()].flatMap(
      (threadId) => messagesByThread.value[threadId] ?? [],
    );
    return {
      threadCount: [...retained.keys()].filter((threadId) => messagesByThread.value[threadId]).length,
      messageCount: cachedMessages.length,
      requestGenerationCount: threadLoadVersions.size,
      bodyChars: cachedMessages.reduce(
        (total, message) => total + messageBodyChars(message),
        0,
      ),
    };
  }

  function prioritizedMessageIds(threadId: string): string[] {
    const messages = messagesByThread.value[threadId] ?? [];
    const latest = messages.at(-1);
    const count = latest && isOwnEmail(accountsStore.accounts, latest.from.email) ? 2 : 1;
    return messages.slice(-count).map((message) => message.id);
  }

  function requestedDetailIds(threadId: string): string[] {
    const demanded = (messagesByThread.value[threadId] ?? [])
      .filter(message => demandedDetails.has(message.id) || detailsNeedingRevalidation.has(message.id))
      .map(message => message.id);
    return [...new Set([...prioritizedMessageIds(threadId), ...demanded])];
  }

  function forgetRemovedDetails(previousIds: Iterable<string>, rows: BackendMessage[]) {
    const currentIds = new Set(rows.map(row => row.id));
    for (const id of previousIds) {
      if (currentIds.has(id)) continue;
      demandedDetails.delete(id);
      revalidationAttempts.delete(id);
      detailsNeedingRevalidation.delete(id);
    }
  }

  async function loadThreadOnce(threadId: string, options: { remote: boolean }) {
    touchThreadCache(threadId);
    const version = nextVersion(threadId);
    const endTotal = perfStart(`messages.loadByThread:${threadId}`);

    try {
      const endPreview = perfStart(`messages.open:${threadId}`);
      const opened = await backendLoadThreadMessages(threadId, OPEN_BODY_COUNT);
      endPreview?.();
      if (threadLoadVersions.get(threadId) !== version) {
        endTotal?.();
        return;
      }

      const rows = opened.messages;
      const entry = entryFor(threadId);
      entry.stale = false;
      if (rows.length === 0) {
        messagesByThread.value[threadId] = [];
        entry.complete = true;
        // A thread that really has no messages must clear the error too, or the
        // reader keeps showing the previous failure over a legitimately empty
        // result and never offers a way out of it.
        connectionStore.clearBackendError('messages');
        endTotal?.();
        return;
      }

      const previousById = new Map(
        (messagesByThread.value[threadId] ?? []).map((message) => [message.id, message]),
      );
      forgetRemovedDetails(previousById.keys(), rows);
      messagesByThread.value[threadId] = rows.map((row) => {
        const preview = mapBackendMessage(row, threadId);
        return mergePreview(preview, previousById.get(preview.id));
      });
      connectionStore.clearBackendError('messages');
      perfLog(`messages.open:${threadId}`, `rows=${rows.length} bodies=${opened.bodies.length}`);

      const prioritizedIds = prioritizedMessageIds(threadId);
      // Keep the critical load in flight until the visible (latest) message has
      // a body. Otherwise a duplicate reload can invalidate this request after
      // the preview, then fail and leave the UI on "Loading full message...".
      await applyBodyRows(threadId, prioritizedIds, opened.bodies, version, options);
      if (threadLoadVersions.get(threadId) !== version) {
        endTotal?.();
        return;
      }
      enforceCacheBudget(threadId);
      if (options.remote) {
        const detailIds = requestedDetailIds(threadId);
        const missingBodies = detailIds.filter(id => {
          const message = messagesByThread.value[threadId]?.find(message => message.id === id);
          return message?.bodyLoaded === false && !message.contentError;
        });
        if (missingBodies.length > 0) await hydrateMessageBodies(threadId, missingBodies, version);
        if (threadLoadVersions.get(threadId) !== version) return;
        entry.complete = true;
        // A full reload replaces pending body and image work for expanded mail too.
        void hydrateMessageAttachments(threadId, detailIds, version);
      }
      recordFrontendDiagnostic('messages.loaded', `thread=${threadId}`, cacheStats());
      endTotal?.();
    } catch (error) {
      endTotal?.();
      if (threadLoadVersions.get(threadId) !== version) return;
      if (backendCommandsAvailable()) {
        connectionStore.reportBackendError('messages', error);
      }
    }
  }

  function trackLoad(threadId: string, run: () => Promise<void>, visible: boolean): Promise<void> {
    const existing = threadLoadPromises.get(threadId);
    if (existing) {
      return existing;
    }

    if (visible) loadingThreadIds.value = [...loadingThreadIds.value, threadId];
    const trackedPromise = run().finally(() => {
      if (threadLoadPromises.get(threadId) === trackedPromise) {
        threadLoadPromises.delete(threadId);
      }
      if (visible) loadingThreadIds.value = loadingThreadIds.value.filter((id) => id !== threadId);
      scheduleSweep();
    });
    threadLoadPromises.set(threadId, trackedPromise);
    return trackedPromise;
  }

  /** Load a thread from scratch: rows, the latest bodies, their inline images. */
  function loadByThread(threadId: string): Promise<void> {
    return trackLoad(threadId, () => loadThreadOnce(threadId, { remote: true }), true);
  }

  /**
   * What the reader calls on selection. A cached thread renders as it is and
   * costs no call; a stale one reloads its previews behind the cached view.
   */
  async function openThread(threadId: string): Promise<void> {
    const inFlight = threadLoadPromises.get(threadId);
    if (inFlight) await inFlight;
    if (!hasThread(threadId)) {
      await loadByThread(threadId);
      return;
    }
    touchThreadCache(threadId);
    const entry = entryFor(threadId);
    if (entry.stale) {
      await refreshThread(threadId);
    }
    if (!entry.complete) {
      entry.complete = true;
      await loadAttachmentsForMessages(threadId, prioritizedMessageIds(threadId));
    }
  }

  /**
   * Warm a thread the reader is likely to open next. Local rows and bodies
   * only: no server round trip, no image download.
   */
  function prefetchThread(threadId: string): Promise<void> {
    if (hasThread(threadId) || threadLoadPromises.has(threadId)) {
      return threadLoadPromises.get(threadId) ?? Promise.resolve();
    }
    return trackLoad(threadId, () => loadThreadOnce(threadId, { remote: false }), false);
  }

  /**
   * Reload the previews of a cached thread after a mail refresh. Bodies and
   * images stay within their saved generation; changed visible mail reloads.
   */
  function refreshThread(threadId: string): Promise<void> {
    let runner = threadRefreshRunners.get(threadId);
    if (!runner) {
      runner = createCoalescingRunner(async () => {
        // A load may have taken its snapshot before the change that prompted
        // this refresh. Wait for it, then read again; sharing its promise would
        // silently acknowledge a change we never fetched.
        while (threadLoadPromises.has(threadId)) {
          await threadLoadPromises.get(threadId);
        }
        await refreshThreadOnce(threadId);
      }, () => true);
      threadRefreshRunners.set(threadId, runner);
    }
    const current = runner;
    return current.run().finally(() => {
      if (threadRefreshRunners.get(threadId) === current) threadRefreshRunners.delete(threadId);
    });
  }

  function refreshThreadOnce(threadId: string): Promise<void> {
    if (!hasThread(threadId)) return loadByThread(threadId);
    return trackLoad(threadId, async () => {
      const version = nextVersion(threadId);
      let rows: BackendMessage[];
      try {
        rows = await backendListMessagesPreview(threadId);
      } catch (error) {
        if (threadLoadVersions.get(threadId) !== version) return;
        if (backendCommandsAvailable()) connectionStore.reportBackendError('messages', error);
        return;
      }
      if (threadLoadVersions.get(threadId) !== version) return;
      entryFor(threadId).stale = false;
      const previousById = new Map(
        (messagesByThread.value[threadId] ?? []).map((message) => [message.id, message]),
      );
      forgetRemovedDetails(previousById.keys(), rows);
      messagesByThread.value[threadId] = rows.map((row) => {
        const preview = mapBackendMessage(row, threadId);
        return mergePreview(preview, previousById.get(preview.id));
      });
      connectionStore.clearBackendError('messages');
      const detailIds = requestedDetailIds(threadId);
      const arrived = detailIds.filter((id) => {
        const message = messagesByThread.value[threadId]?.find(message => message.id === id);
        return !previousById.has(id) || changedGeneration(message?.remoteUidValidity, previousById.get(id), message?.contentError)
          || detailsNeedingRevalidation.has(id)
          || (demandedDetails.has(id) && message?.bodyLoaded === false && !message.contentError);
      });
      if (arrived.length > 0) {
        for (const id of arrived) {
          if (detailsNeedingRevalidation.delete(id)) {
            hydratedAttachmentMessageIds.delete(id);
            hydratingAttachmentMessageIds.delete(id);
          }
        }
        await hydrateMessageBodies(threadId, arrived, version);
      }
      // Refresh also replaces unfinished image requests for already loaded bodies.
      void hydrateMessageAttachments(threadId, detailIds, version);
    }, false);
  }

  function isLoading(threadId: string): boolean {
    return loadingThreadIds.value.includes(threadId);
  }

  function isBodyLoading(messageId: string): boolean {
    return loadingBodyMessageIds.value.includes(messageId);
  }

  /** A body still to load, or inline images not yet asked for. */
  function needsDetails(messageId: string): boolean {
    return !hydratedAttachmentMessageIds.has(messageId) && !hydratingAttachmentMessageIds.has(messageId);
  }

  /**
   * Load the original source when backend commands are available.
   * `null` lets callers explain an unsupported runtime without probing it.
   */
  async function loadMessageSource(messageId: string): Promise<BackendMessageSource | null> {
    if (!backendCommandsAvailable()) {
      return null;
    }
    return backendGetMessageSource(messageId);
  }

  async function exportMessage(messageId: string, subject: string): Promise<boolean> {
    if (!backendCommandsAvailable()) throw new Error(t('mail.source.desktopOnly'));
    const path = await backendChooseSavePath({
      suggestedName: `${sanitizeThreadFilename(subject, t('mail.source.defaultFilename'))}.eml`,
      filterName: t('mail.source.fileType'),
      extensions: ['eml'],
    });
    if (!path) return false;
    await backendExportMessage({ path, messageId });
    return true;
  }

  async function unsubscribe(messageId: string): Promise<BackendUnsubscribeResult> {
    return backendUnsubscribe(messageId);
  }

  async function hydrateMessageBodies(
    threadId: string,
    messageIds: string[],
    version: symbol,
  ) {
    if (messageIds.length === 0 || threadLoadVersions.get(threadId) !== version) {
      return;
    }

    const endBodies = perfStart(`messages.bodies:${threadId}:${messageIds.length}`);
    let rows: BackendMessageBody[];
    try {
      rows = await withTimeout(
        backendListMessageBodies(messageIds),
        MESSAGE_BODY_LOAD_TIMEOUT_MS,
        t('errors.messageLoadTimeout'),
      );
    } catch (error) {
      endBodies?.();
      if (threadLoadVersions.get(threadId) !== version) {
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      const failedIds = new Set(messageIds);
      const current = messagesByThread.value[threadId] ?? [];
      messagesByThread.value = {
        ...messagesByThread.value,
        [threadId]: current.map((messageItem) =>
          failedIds.has(messageItem.id)
            ? {
                ...messageItem,
                bodyLoaded: true,
                bodyLoadError: message || t('errors.messageLoadFailed'),
              }
            : messageItem,
        ),
      };
      return;
    }
    endBodies?.();
    await applyBodyRows(threadId, messageIds, rows, version, { remote: true });
  }

  /**
   * Put fetched body rows on their messages. `requestedIds` are the ones the
   * reader waits for: a missing row there is an error, and an empty one is
   * fetched from the server when `remote` allows it.
   */
  async function applyBodyRows(
    threadId: string,
    requestedIds: string[],
    rows: BackendMessageBody[],
    version: symbol,
    options: { remote: boolean },
  ) {
    if (threadLoadVersions.get(threadId) !== version) {
      return;
    }

    const bodiesById = new Map(rows.map((row) => [row.id, row]));
    const requested = new Set(requestedIds);
    const current = messagesByThread.value[threadId] ?? [];
    const currentById = new Map(current.map(message => [message.id, message]));
    const staleBodyIds = new Set(rows
      .filter(row => changedGeneration(row.remote_uidvalidity, currentById.get(row.id), row.content_error))
      .map(row => row.id));
    messagesByThread.value = {
      ...messagesByThread.value,
      [threadId]: current.map((message) => {
        const body = bodiesById.get(message.id);
        if (body) {
          if (staleBodyIds.has(message.id)) return message;
          return applyMessageBody(message, body);
        }
        if (requested.has(message.id)) {
          return {
            ...message,
            bodyLoaded: true,
            bodyLoadError: t('errors.messageLoadFailed'),
          };
        }
        return message;
      }),
    };
    if (staleBodyIds.size > 0) {
      for (const id of staleBodyIds) revalidateMessageReference(threadId, id);
    }

    // A message whose complete body is not stored either comes back empty or
    // explicitly marked as incomplete (for example, an old 64-KiB IMAP
    // prefix). Fetch it now instead of showing the blank page or cached prefix.
    const emptyIds = rows
      .filter((row) => !row.content_error && !staleBodyIds.has(row.id) && requested.has(row.id) && (
        !row.body_loaded
        || (!(row.body_html ?? '').trim() && !row.body_text.trim())
      ))
      .map((row) => row.id);
    if (emptyIds.length > 0 && options.remote) {
      // The prioritized visible message is part of the first readable frame.
      // Keep the thread load active until it has either arrived or failed, so
      // the UI never publishes a half-card followed by a second loader.
      await fetchMissingBodies(threadId, emptyIds, rows, version);
    }
  }

  /**
   * Fetch bodies the local database does not have, one at a time.
   *
   * Sequential on purpose: this talks to the mail server, and a thread of ten
   * old messages should not open ten connections at once. The first one is
   * usually the only one the reader is waiting for.
   */
  async function fetchMissingBodies(
    threadId: string,
    messageIds: string[],
    rows: BackendMessageBody[],
    version: symbol,
  ) {
    const rowsById = new Map(rows.map((row) => [row.id, row]));
    for (const id of messageIds) {
      if (threadLoadVersions.get(threadId) !== version) return;
      if (bodyRequestTokens.get(id)?.version === version) continue;
      const token = { version };
      bodyRequestTokens.set(id, token);
      const generation = messagesByThread.value[threadId]?.find(message => message.id === id)?.remoteUidValidity ?? null;
      let ownsRequest = true;
      if (!isBodyLoading(id)) loadingBodyMessageIds.value = [...loadingBodyMessageIds.value, id];
      let loaded: Awaited<ReturnType<typeof backendLoadMessageBody>>;
      try {
        loaded = await withTimeout(
          backendLoadMessageBody(id),
          REMOTE_MESSAGE_BODY_LOAD_TIMEOUT_MS,
          t('errors.messageLoadTimeout'),
        );
      } catch (error) {
        if (threadLoadVersions.get(threadId) !== version) return;
        if (bodyRequestTokens.get(id) !== token) continue;
        const detail = backendErrorLabel(error);
        patchMessage(threadId, id, (message) => ({
          ...message,
          bodyLoaded: true,
          bodyLoadError: detail || t('errors.messageLoadFailed'),
        }));
        continue;
      } finally {
        ownsRequest = bodyRequestTokens.get(id) === token;
        if (ownsRequest) {
          bodyRequestTokens.delete(id);
          loadingBodyMessageIds.value = loadingBodyMessageIds.value.filter(messageId => messageId !== id);
        }
      }
      if (threadLoadVersions.get(threadId) !== version) return;
      if (!ownsRequest || (messagesByThread.value[threadId]?.find(message => message.id === id)?.remoteUidValidity ?? null) !== generation) continue;
      if ((loaded.remote_uidvalidity ?? null) !== generation) {
        revalidateMessageReference(threadId, id);
        continue;
      }
      const row = rowsById.get(id);
      patchMessage(threadId, id, (message) =>
        applyMessageBody(message, {
          ...(row ?? { id, snippet: '', body_text: '', body_loaded: true }),
          id,
          body_text: loaded.body_text,
          body_html: loaded.body_html,
          body_loaded: true,
          remote_uidvalidity: loaded.remote_uidvalidity,
        }),
      );
    }
  }

  function patchMessage(
    threadId: string,
    messageId: string,
    apply: (message: Message) => Message,
  ) {
    const current = messagesByThread.value[threadId] ?? [];
    messagesByThread.value = {
      ...messagesByThread.value,
      [threadId]: current.map((message) => (message.id === messageId ? apply(message) : message)),
    };
  }

  /**
   * A draft is a message row under the draft's id. Saving it does not raise a
   * thread event, so the card at the end of the thread kept the words and the
   * time it was loaded with after the composer had saved newer ones.
   */
  function applySavedDraft(draft: Pick<BackendDraft, 'id' | 'thread_id' | 'body' | 'body_html' | 'updated_at'>) {
    const threadId = draft.thread_id;
    if (!threadId || !messagesByThread.value[threadId]?.some((message) => message.id === draft.id && message.isDraft)) return;
    patchMessage(threadId, draft.id, (message) => ({
      ...message,
      bodyText: draft.body,
      bodyHtml: draft.body_html ?? undefined,
      timestamp: draft.updated_at || message.timestamp,
    }));
  }

  async function hydrateMessageAttachments(
    threadId: string,
    messageIds: string[],
    version: symbol,
  ) {
    if (messageIds.length === 0) {
      return;
    }

    const pending = messageIds.filter((messageId) =>
      !messagesByThread.value[threadId]?.find(message => message.id === messageId)?.contentError
      && !hydratedAttachmentMessageIds.has(messageId) && hydratingAttachmentMessageIds.get(messageId)?.version !== version);
    if (threadLoadVersions.get(threadId) !== version || pending.length === 0) {
      return;
    }
    const claims = new Map(pending.map(messageId => [messageId, { version }]));
    const generations = new Map(pending.map(messageId => [messageId,
      messagesByThread.value[threadId]?.find(message => message.id === messageId)?.remoteUidValidity ?? null]));
    for (const messageId of pending) hydratingAttachmentMessageIds.set(messageId, claims.get(messageId)!);

    // One round trip for the whole thread; a per-message listing made a topic
    // feed of twenty threads cost 160 invokes before a single image loaded.
    let rowsByMessage: Map<string, BackendAttachmentDto[]> | null = null;
    let listingError: string | undefined;
    try {
      rowsByMessage = new Map();
      for (const row of await backendListAttachmentsForMessages(pending)) {
        const rows = rowsByMessage.get(row.message_id) ?? [];
        rows.push(row);
        rowsByMessage.set(row.message_id, rows);
      }
    } catch (error) {
      rowsByMessage = null;
      listingError = error instanceof Error ? error.message : String(error);
    }

    try {
      for (const messageId of pending) {
        if (threadLoadVersions.get(threadId) !== version) {
          return;
        }
        const isCurrent = () => hydratingAttachmentMessageIds.get(messageId) === claims.get(messageId)
          && (messagesByThread.value[threadId]?.find(message => message.id === messageId)?.remoteUidValidity ?? null) === generations.get(messageId);
        if (!isCurrent()) continue;

        const body = (messagesByThread.value[threadId] ?? [])
          .find((message) => message.id === messageId)?.bodyHtml ?? '';
        const referencedCids = referencedInlineCids(body);
        // `null` means the attachment list itself never arrived — keep whatever
        // the message already had rather than blanking it.
        let attachments: Message['attachments'] | null = null;
        let hydrationError: string | undefined = listingError;
        if (rowsByMessage) {
          const rows = rowsByMessage.get(messageId) ?? [];
          const hydrated = await Promise.all(
            rows.map(mapAttachment).map((row) => hydrateInlineAttachment(row, referencedCids, generations.get(messageId) ?? null)),
          );
          if (hydrated.some(entry => entry.generationMismatch)) {
            if (isCurrent()) revalidateMessageReference(threadId, messageId);
            continue;
          }
          attachments = hydrated.map((entry) => entry.attachment);
          hydrationError = hydrated.find((entry) => entry.error)?.error;
        }
        if (threadLoadVersions.get(threadId) !== version) {
          return;
        }
        if (!isCurrent()) continue;
        hydratingAttachmentMessageIds.delete(messageId);
        // Only a clean pass counts as done. A failed one stays retryable, both
        // through `retryInlineImages` and on the next thread load.
        if (!hydrationError) {
          hydratedAttachmentMessageIds.add(messageId);
        }
        patchMessage(threadId, messageId, (message) => {
          const nextAttachments = attachments ?? message.attachments;
          const pendingImages = pendingInlineImageCount(referencedCids, nextAttachments, !hydrationError);
          return {
            ...message,
            attachments: nextAttachments,
            inlineImagesPending: pendingImages,
            inlineImagesError: pendingImages > 0 ? hydrationError : undefined,
          };
        });
      }
    } finally {
      // A superseded load stops early; what it did not get to has to stay
      // claimable by the next one.
      for (const messageId of pending) {
        if (hydratingAttachmentMessageIds.get(messageId) === claims.get(messageId)) hydratingAttachmentMessageIds.delete(messageId);
      }
    }
    enforceCacheBudget(threadId);
  }

  /**
   * Re-run inline image hydration for one message after a failed attempt —
   * provider rate limits and offline spells are the common case, and both
   * resolve on their own.
   */
  async function retryInlineImages(threadId: string, messageId: string): Promise<void> {
    const version = threadLoadVersions.get(threadId);
    if (!version) {
      return;
    }
    hydratedAttachmentMessageIds.delete(messageId);
    await hydrateMessageAttachments(threadId, [messageId], version);
  }

  /** Bodies and inline images for messages the reader is about to show. */
  async function loadAttachmentsForMessages(threadId: string, messageIds: string[]): Promise<void> {
    const version = threadLoadVersions.get(threadId);
    if (!version) {
      return;
    }
    touchThreadCache(threadId);
    for (const id of messageIds) {
      if (messagesByThread.value[threadId]?.some(message => message.id === id)) demandedDetails.add(id);
    }

    const missingBodyIds = (messagesByThread.value[threadId] ?? [])
      .filter((message) => messageIds.includes(message.id) && message.bodyLoaded === false)
      .map((message) => message.id);
    const chunkSize = 3;
    for (let start = 0; start < missingBodyIds.length; start += chunkSize) {
      await hydrateMessageBodies(threadId, missingBodyIds.slice(start, start + chunkSize), version);
      await pause(0);
    }
    await hydrateMessageAttachments(threadId, messageIds, version);
    enforceCacheBudget(threadId);
    recordFrontendDiagnostic('messages.expanded', `thread=${threadId}`, cacheStats());
  }

  return {
    messagesByThread,
    totalMessages,
    cacheStats,
    getByThread,
    hasThread,
    isLoading,
    isBodyLoading,
    needsDetails,
    loadMessageSource,
    exportMessage,
    unsubscribe,
    loadByThread,
    openThread,
    prefetchThread,
    refreshThread,
    applySavedDraft,
    retainThread,
    markThreadsStale,
    reloadPgpMessages,
    loadAttachmentsForMessages,
    retryInlineImages,
  };
});
