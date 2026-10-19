import { createPersistedSlot, persistence } from './index';
import type { ComposerDraftState } from '@/utils/composerDraftState';
import type { BackendDraftSendOptions } from '@/types/composer';

/**
 * Device-local copies of composer content that is not on disk yet.
 *
 * The autosave writes every ten seconds and can fail for minutes; this copy is
 * written shortly after typing pauses and survives a crash or a forced quit.
 * Attachments stay out: they would blow the storage quota, and the draft row
 * keeps the ones that were saved.
 */
export const COMPOSER_BACKUP_STORAGE_KEY = 'satchel.composer.backups.v1';

/** A backup this old is not offered any more. */
const MAX_AGE_MS = 14 * 24 * 60 * 60 * 1000;
const MAX_BACKUPS = 8;
/** Larger HTML is dropped; the plain-text body still restores. */
export const MAX_BACKUP_HTML_CHARS = 512 * 1024;
/** A composer refreshes `aliveAt` while open; a stale one belongs to a closed window. */
export const BACKUP_HEARTBEAT_MS = 5_000;
const ALIVE_WINDOW_MS = BACKUP_HEARTBEAT_MS * 3;

export type ComposerBackupContent = Omit<ComposerDraftState, 'attachments'>;

export interface ComposerBackup {
  /** One per composer instance. */
  key: string;
  draftId: string | null;
  savedAt: string;
  aliveAt: number;
  content: ComposerBackupContent;
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === 'string');
}

function decodeContent(value: unknown): ComposerBackupContent | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const raw = value as Record<string, unknown>;
  if (
    typeof raw.accountId !== 'string'
    || typeof raw.fromAddress !== 'string'
    || !isStringArray(raw.to)
    || !isStringArray(raw.cc)
    || !isStringArray(raw.bcc)
    || typeof raw.subject !== 'string'
    || typeof raw.body !== 'string'
    || !(typeof raw.bodyHtml === 'string' || raw.bodyHtml === null)
    || typeof raw.plainText !== 'boolean'
  ) {
    return undefined;
  }
  const metadata: Record<string, string | null> = {};
  for (const name of ['fromName', 'threadId', 'replyTo', 'scheduledFor', 'signatureId', 'signatureOverrideText', 'signatureOverrideHtml']) {
    const field = raw[name];
    if (field !== undefined && field !== null && typeof field !== 'string') return undefined;
    metadata[name] = typeof field === 'string' ? field : null;
  }
  const signatureVariant = raw.signatureVariant ?? 'auto';
  if (signatureVariant !== 'auto' && signatureVariant !== 'long' && signatureVariant !== 'short' && signatureVariant !== 'none') return undefined;
  const sendOptions = decodeSendOptions(raw.sendOptions);
  if (!sendOptions) return undefined;
  return {
    accountId: raw.accountId,
    fromAddress: raw.fromAddress,
    to: raw.to,
    cc: raw.cc,
    bcc: raw.bcc,
    subject: raw.subject,
    body: raw.body,
    bodyHtml: raw.bodyHtml,
    plainText: raw.plainText,
    fromName: metadata.fromName ?? null,
    threadId: metadata.threadId ?? null,
    replyTo: metadata.replyTo ?? null,
    scheduledFor: metadata.scheduledFor ?? null,
    signatureId: metadata.signatureId ?? null,
    signatureVariant,
    signatureOverrideText: metadata.signatureOverrideText ?? null,
    signatureOverrideHtml: metadata.signatureOverrideHtml ?? null,
    sendOptions,
  };
}

function decodeSendOptions(value: unknown): BackendDraftSendOptions | undefined {
  // Older v1 copies carried text only. New copies preserve delivery policy too.
  if (value === undefined) return {};
  if (!value || typeof value !== 'object') return undefined;
  const raw = value as Record<string, unknown>;
  const result: BackendDraftSendOptions = {};
  for (const key of ['reply_to_message_id', 'follow_up_at', 'smime_sign_with_cert_id', 'pgp_sign_with_key_id'] as const) {
    const field = raw[key];
    if (field === null || typeof field === 'string') result[key] = field;
    else if (field !== undefined) return undefined;
  }
  for (const key of ['smime_encrypt_to_recipients', 'pgp_encrypt_to_recipients'] as const) {
    const field = raw[key];
    if (typeof field === 'boolean') result[key] = field;
    else if (field !== undefined) return undefined;
  }
  if (raw.priority === null || raw.priority === 'high' || raw.priority === 'normal' || raw.priority === 'low') result.priority = raw.priority;
  else if (raw.priority !== undefined) return undefined;
  return result;
}

function decodeBackup(value: unknown): ComposerBackup | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const raw = value as Record<string, unknown>;
  const content = decodeContent(raw.content);
  if (
    !content
    || typeof raw.key !== 'string'
    || !(typeof raw.draftId === 'string' || raw.draftId === null)
    || typeof raw.savedAt !== 'string'
    || typeof raw.aliveAt !== 'number'
  ) {
    return undefined;
  }
  return { key: raw.key, draftId: raw.draftId, savedAt: raw.savedAt, aliveAt: raw.aliveAt, content };
}

const backupsSlot = createPersistedSlot<ComposerBackup[]>(COMPOSER_BACKUP_STORAGE_KEY, {
  fallback: () => [],
  decode: (raw) => {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return undefined;
    const { version, backups } = parsed as { version?: unknown; backups?: unknown };
    if (version !== 1 || !Array.isArray(backups)) return undefined;
    const cutoff = Date.now() - MAX_AGE_MS;
    return backups
      .map(decodeBackup)
      .filter((backup): backup is ComposerBackup => backup !== undefined)
      .filter((backup) => Date.parse(backup.savedAt) > cutoff);
  },
  encode: (backups) => JSON.stringify({ version: 1, backups }),
});

function readAll(): ComposerBackup[] {
  return persistence.read(backupsSlot);
}

function writeAll(backups: ComposerBackup[]): boolean {
  if (backups.length === 0) return persistence.remove(backupsSlot);
  return persistence.write(backupsSlot, backups);
}

function isAlive(backup: ComposerBackup, now: number): boolean {
  return now - backup.aliveAt < ALIVE_WINDOW_MS;
}

export const composerBackupPersistence = {
  /** Stores this composer's copy, newest first, capped. `false` when storage refused. */
  write(backup: ComposerBackup): boolean {
    const content = backup.content.bodyHtml && backup.content.bodyHtml.length > MAX_BACKUP_HTML_CHARS
      ? { ...backup.content, bodyHtml: null }
      : backup.content;
    const others = readAll().filter((entry) => entry.key !== backup.key);
    return writeAll([{ ...backup, content }, ...others].slice(0, MAX_BACKUPS));
  },

  /** Marks this composer as still open, so no other window offers its copy. */
  touch(key: string, now = Date.now()): void {
    const backups = readAll();
    const entry = backups.find((backup) => backup.key === key);
    if (!entry) return;
    entry.aliveAt = now;
    writeAll(backups);
  },

  remove(key: string): void {
    const backups = readAll();
    const remaining = backups.filter((backup) => backup.key !== key);
    if (remaining.length !== backups.length) writeAll(remaining);
  },

  /**
   * The newest copy no open composer owns. With `draftId` only that draft's,
   * otherwise any — a fresh composer offers what a crash left behind.
   */
  findOrphan(options: { draftId?: string | null } = {}, now = Date.now()): ComposerBackup | null {
    return readAll()
      .filter((backup) => !isAlive(backup, now))
      .filter((backup) => !options.draftId || backup.draftId === options.draftId)
      .sort((a, b) => b.savedAt.localeCompare(a.savedAt))[0] ?? null;
  },
};
