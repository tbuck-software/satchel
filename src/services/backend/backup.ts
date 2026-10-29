import type { ExportResult as WireExportResult } from './generated/app/commands/backup/ExportResult';
import type { ImportResult as WireImportResult } from './generated/app/commands/backup/ImportResult';
import type { ImportProgress as WireImportProgress } from './generated/app/commands/backup/ImportProgress';
import type { PlatformDataResult as WirePlatformDataResult } from './generated/app/commands/backup/PlatformDataResult';
import type { BackupEntry as WireBackupEntry } from './generated/app/commands/backup/BackupEntry';
import type { RestoreStaged as WireRestoreStaged } from './generated/app/commands/backup/RestoreStaged';
import type { CredentialsStatus as WireCredentialsStatus } from './generated/app/commands/backup/CredentialsStatus';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';

export type BackendExportResult = WireExportResult;


export type BackendImportResult = WireImportResult;


/** Payload of `import:progress`, emitted by both importers. */
export type BackendImportProgress = WireImportProgress;


export type BackendPlatformDataResult = WirePlatformDataResult;


export async function backendExportMailbox(input: {
  path: string;
  accountId?: string | null;
}): Promise<BackendExportResult> {
  return invoke<BackendExportResult>('export_mailbox', {
    path: input.path,
    accountId: input.accountId ?? null,
  });
}


export async function backendExportMessage(input: { path: string; messageId: string }): Promise<void> {
  return invoke<void>('export_message', input);
}


export async function backendImportMbox(input: {
  path: string;
  accountId: string;
}): Promise<BackendImportResult> {
  return invoke<BackendImportResult>('import_mbox', {
    path: input.path,
    accountId: input.accountId,
  });
}


/** Stop the running import after the message it is on. */
export async function backendCancelImport(): Promise<void> {
  return invoke<void>('cancel_import', {});
}


export async function backendExportPlatformData(input: {
  path: string;
  redactSensitive?: boolean;
}): Promise<BackendPlatformDataResult> {
  return invoke<BackendPlatformDataResult>('export_platform_data', {
    path: input.path,
    redactSensitive: input.redactSensitive ?? true,
  });
}


export async function backendImportPlatformData(input: {
  path: string;
}): Promise<BackendPlatformDataResult> {
  return invoke<BackendPlatformDataResult>('import_platform_data', {
    path: input.path,
  });
}


export type BackendBackupEntry = WireBackupEntry;


export type BackendRestoreStaged = WireRestoreStaged;


export type BackendCredentialsStatus = WireCredentialsStatus;


/**
 * A save dialog opened by the backend. Export commands write only to a path
 * that came back from here, so a path the webview invents is refused.
 */
export async function backendChooseSavePath(input: {
  suggestedName: string;
  filterName: string;
  extensions: string[];
}): Promise<string | null> {
  return invoke<string | null>('choose_save_path', input);
}

/** The folder counterpart, for a backup target. */
export async function backendChooseDirectory(defaultPath?: string): Promise<string | null> {
  return invoke<string | null>('choose_directory', { defaultPath: defaultPath || null });
}

/**
 * VACUUM INTO a timestamped folder below `targetDir`, master key included.
 * Without a passphrase the key is written bare, which the backend only accepts
 * against an explicit acknowledgement.
 */
export async function backendCreateBackup(
  targetDir: string,
  options: { passphrase?: string; acknowledgeUnprotectedKey?: boolean } = {},
): Promise<BackendBackupEntry> {
  return invoke<BackendBackupEntry>('create_backup', {
    targetDir,
    passphrase: options.passphrase || null,
    acknowledgeUnprotectedKey: options.acknowledgeUnprotectedKey ?? false,
  });
}


export async function backendListBackups(dir: string): Promise<BackendBackupEntry[]> {
  return invoke<BackendBackupEntry[]>('list_backups', { dir });
}


/** Stages the backup; it replaces the database on the next start. */
export async function backendRestoreBackup(
  path: string,
  passphrase?: string,
): Promise<BackendRestoreStaged> {
  return invoke<BackendRestoreStaged>('restore_backup', {
    path,
    passphrase: passphrase || null,
  });
}


export async function backendCancelPendingRestore(): Promise<void> {
  return invoke<void>('cancel_pending_restore');
}


export async function backendPruneBackups(dir: string, keep: number): Promise<string[]> {
  return invoke<string[]>('prune_backups', { dir, keep });
}


export async function backendCredentialsStatus(): Promise<BackendCredentialsStatus> {
  return invoke<BackendCredentialsStatus>('credentials_status');
}
