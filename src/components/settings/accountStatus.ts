import type { Account } from '@/types/mail';
import { translateBackendError } from '@/services/backend';

type Translate = (key: string, params?: Record<string, unknown>) => string;

export function providerLabel(provider: string): string {
  if (provider === 'gmail') return 'Gmail';
  if (provider === 'outlook') return 'Outlook';
  if (provider === 'pop3') return 'POP3';
  return 'IMAP';
}

export function usesPassword(provider: string): boolean {
  return provider === 'imap' || provider === 'pop3';
}

export function usesOauth(provider: string): boolean {
  return provider === 'gmail' || provider === 'outlook';
}

export function syncStatusLabel(account: Account, t: Translate, locale: string): string {
  const status = account.syncStatus ?? 'idle';
  if (status === 'syncing') return t('settings.accounts.sync.running');
  if (status === 'error') return t('settings.accounts.sync.error');
  if (account.lastSyncAt) {
    return t('settings.accounts.sync.lastAt', {
      time: new Date(account.lastSyncAt).toLocaleString(locale, { dateStyle: 'medium', timeStyle: 'short', hour12: false }),
    });
  }
  return t('settings.accounts.sync.never');
}

export function syncProgressLabel(raw: string | null | undefined, t: Translate): string {
  if (!raw) return '';
  try {
    const parsed = JSON.parse(raw) as {
      phase?: string;
      folder?: string;
      folder_index?: number;
      folder_total?: number;
    };
    if (parsed.phase === 'folder' && parsed.folder) {
      return `${parsed.folder_index ?? '?'} / ${parsed.folder_total ?? '?'}: ${parsed.folder}`;
    }
    if (parsed.phase === 'complete') return t('settings.accounts.sync.progressComplete');
    if (parsed.phase === 'error') return t('common.error');
  } catch {}
  return '';
}

export function syncErrorText(account: Account): string | null {
  if (!account.lastSyncError) return null;
  return translateBackendError(account.lastSyncError) ?? account.lastSyncError;
}
