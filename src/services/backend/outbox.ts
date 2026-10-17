// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type { BackendOutboxItem } from '@/types/outbox';
import type { BackendDraft } from '@/types/composer';

export type { BackendOutboxItem } from '@/types/outbox';


export async function backendListOutbox(): Promise<BackendOutboxItem[]> {
  return invoke<BackendOutboxItem[]>('list_outbox');
}


export async function backendRetryOutboxAction(id: string): Promise<void> {
  return invoke<void>('retry_outbox_action', { id });
}


export async function backendRetryFailedSyncActions(): Promise<number> {
  return invoke<number>('retry_failed_sync_actions');
}


export async function backendCancelOutboxAction(id: string): Promise<void> {
  return invoke<void>('cancel_outbox_action', { id });
}

export async function backendUndoOutboxSend(id: string): Promise<BackendDraft> {
  return invoke<BackendDraft>('undo_outbox_send', { id });
}

export async function backendListFailedSyncActions(): Promise<BackendOutboxItem[]> {
  return invoke<BackendOutboxItem[]>('list_failed_sync_actions');
}

export async function backendDiscardFailedSyncAction(id: string): Promise<void> {
  return invoke<void>('discard_failed_sync_action', { id });
}
