import type { FollowUpDto as WireFollowUpDto } from './generated/app/commands/followups/FollowUpDto';
import type { OptionalWireFields } from './wireTypes';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';
import type { BackendContact } from '@/types/contacts';

export type { BackendContact } from '@/types/contacts';


export async function backendListContacts(): Promise<BackendContact[]> {
  return invoke<BackendContact[]>('list_contacts');
}

export async function backendSyncLocalContactAvatars(): Promise<number> {
  return invoke<number>('sync_local_contact_avatars');
}


export async function backendSyncContactAvatars(allowThirdParty = false): Promise<{
  accounts_checked: number;
  avatars_updated: number;
  fallback_avatars_updated: number;
  errors: string[];
}> {
  return invoke('sync_contact_avatars', { allowThirdParty });
}


export async function backendSearchContacts(query: string, limit = 12): Promise<BackendContact[]> {
  return invoke<BackendContact[]>('search_contacts', { query, limit });
}


export async function backendUpsertContact(input: {
  id?: string | null;
  email: string;
  displayName?: string | null;
  avatarUrl?: string | null;
  notes?: string | null;
}): Promise<BackendContact> {
  return invoke<BackendContact>('upsert_contact', {
    input: {
      id: input.id ?? null,
      email: input.email,
      display_name: input.displayName ?? null,
      avatar_url: input.avatarUrl ?? null,
      notes: input.notes ?? null,
    },
  });
}


export async function backendSetContactAvatar(input: {
  email: string;
  displayName?: string | null;
  avatarUrl?: string | null;
  locked?: boolean | null;
}): Promise<BackendContact> {
  return invoke<BackendContact>('set_contact_avatar', {
    input: {
      email: input.email,
      display_name: input.displayName ?? null,
      avatar_url: input.avatarUrl ?? null,
      locked: input.locked ?? null,
    },
  });
}


export async function backendDeleteContact(id: string): Promise<void> {
  return invoke<void>('delete_contact', { id });
}


export type BackendFollowUp = OptionalWireFields<WireFollowUpDto, 'message_id' | 'note' | 'subject' | 'snippet' | 'resolved_at'>;


export async function backendListFollowUps(status = 'pending'): Promise<BackendFollowUp[]> {
  return invoke<BackendFollowUp[]>('list_follow_ups', { status });
}


export async function backendUpsertFollowUp(input: {
  id?: string | null;
  threadId: string;
  dueAt: string;
  note?: string | null;
}): Promise<BackendFollowUp> {
  return invoke<BackendFollowUp>('upsert_follow_up', {
    input: {
      id: input.id ?? null,
      thread_id: input.threadId,
      due_at: input.dueAt,
      note: input.note ?? null,
    },
  });
}


export async function backendCancelFollowUp(id: string): Promise<void> {
  return invoke<void>('cancel_follow_up', { id });
}
