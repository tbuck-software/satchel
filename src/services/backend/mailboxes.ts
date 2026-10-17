import type { MailboxDto as WireMailboxDto } from './generated/app/commands/mailboxes/MailboxDto';
import type { Folder as WireFolder } from './generated/core/db/folders/Folder';
import type { ProviderLabelDto as WireProviderLabelDto } from './generated/app/commands/provider_features/ProviderLabelDto';
import type { OptionalWireFields } from './wireTypes';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';

export type BackendMailbox = WireMailboxDto;


export async function backendListMailboxes(): Promise<BackendMailbox[]> {
  return invoke<BackendMailbox[]>('list_mailboxes');
}


export type BackendFolder = WireFolder;


export async function backendCreateFolder(input: {
  accountId: string;
  name: string;
  parentPath?: string | null;
}): Promise<BackendFolder> {
  return invoke<BackendFolder>('create_folder', {
    input: {
      account_id: input.accountId,
      name: input.name,
      parent_path: input.parentPath ?? null,
    },
  });
}


export async function backendRenameFolder(input: {
  folderId: string;
  newName: string;
}): Promise<BackendFolder> {
  return invoke<BackendFolder>('rename_folder', {
    input: { folder_id: input.folderId, new_name: input.newName },
  });
}


export async function backendDeleteFolder(folderId: string): Promise<void> {
  return invoke<void>('delete_folder', { input: { folder_id: folderId } });
}


export type BackendProviderLabel = OptionalWireFields<WireProviderLabelDto, 'color' | 'kind'>;


export async function backendListProviderLabels(accountId?: string): Promise<BackendProviderLabel[]> {
  return invoke<BackendProviderLabel[]>('list_provider_labels', { accountId: accountId ?? null });
}


export async function backendRefreshProviderLabels(accountId: string): Promise<BackendProviderLabel[]> {
  return invoke<BackendProviderLabel[]>('refresh_provider_labels', { accountId });
}


export async function backendCreateProviderLabel(input: {
  accountId: string;
  name: string;
  color?: string | null;
}): Promise<BackendProviderLabel> {
  return invoke<BackendProviderLabel>('create_provider_label', {
    input: {
      account_id: input.accountId,
      name: input.name,
      color: input.color ?? null,
    },
  });
}


export async function backendMoveThreadsToFolder(input: {
  threadIds: string[];
  targetFolderId: string;
}): Promise<void> {
  return invoke<void>('move_threads_to_folder', {
    input: {
      thread_ids: input.threadIds,
      target_folder_id: input.targetFolderId,
    },
  });
}

