import type { VacationDto as WireVacationDto } from './generated/app/commands/vacation/VacationDto';
import type { OptionalWireFields } from './wireTypes';
import type { NotificationPreferences as WireNotificationPreferences } from './generated/app/commands/notifications/NotificationPreferences';
import type { UiStrings as WireUiStrings } from './generated/app/commands/ui_strings/UiStrings';
import type { BlockedSenderDto as WireBlockedSenderDto } from './generated/app/commands/blocked_senders/BlockedSenderDto';
import type { SmartFolderDto as WireSmartFolderDto } from './generated/app/commands/smart_folders/SmartFolderDto';
import type { MessageFileAttachment as WireMessageFileAttachment } from './generated/app/commands/message_file/MessageFileAttachment';
import type { MessageFilePreview as WireMessageFilePreview } from './generated/app/commands/message_file/MessageFilePreview';
import type { MessageFileAttachmentData as WireMessageFileAttachmentData } from './generated/app/commands/message_file/MessageFileAttachmentData';
import type { TrayPreferencesDto as WireTrayPreferencesDto } from './generated/app/commands/tray/TrayPreferencesDto';
import type { TrayThreadDto as WireTrayThreadDto } from './generated/app/commands/tray/TrayThreadDto';
import type { TraySnapshotDto as WireTraySnapshotDto } from './generated/app/commands/tray/TraySnapshotDto';
import type { ConnectionStateDto as WireConnectionStateDto } from './generated/app/commands/connection/ConnectionStateDto';
import type { CrashEntry as WireCrashEntry } from './generated/app/commands/crash/CrashEntry';
import type { BackgroundSyncStatus as WireBackgroundSyncStatus } from './generated/app/commands/background_sync/BackgroundSyncStatus';
import type { PendingMobileNotification as WirePendingMobileNotification } from './generated/app/commands/mobile_notifications/PendingMobileNotification';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';

export type BackendNotificationPreferences = WireNotificationPreferences;

export type BackendUiStrings = WireUiStrings;

export async function backendSetUiStrings(strings: BackendUiStrings): Promise<void> {
  return invoke<void>('set_ui_strings', { strings });
}


export async function backendOpenComposeWindow(draftId?: string | null): Promise<void> {
  return invoke<void>('open_compose_window', { draftId: draftId ?? null });
}


export async function backendOpenExternalUrl(url: string): Promise<void> {
  return invoke<void>('open_external_url', { url });
}

/** Reveal a saved file in Finder/Explorer; directories open directly. */
export async function backendShowInFolder(path: string): Promise<void> {
  return invoke<void>('show_in_folder', { path });
}


export type BackendBlockedSender = OptionalWireFields<WireBlockedSenderDto, 'account_id'>;


export async function backendListBlockedSenders(): Promise<BackendBlockedSender[]> {
  return invoke<BackendBlockedSender[]>('list_blocked_senders');
}


export type BackendSmartFolder = OptionalWireFields<WireSmartFolderDto, 'icon'>;


export async function backendListSmartFolders(): Promise<BackendSmartFolder[]> {
  return invoke<BackendSmartFolder[]>('list_smart_folders');
}


export async function backendUpsertSmartFolder(input: {
  id?: string | null;
  name: string;
  icon?: string | null;
  query: string;
  sortOrder?: number;
}): Promise<BackendSmartFolder> {
  return invoke<BackendSmartFolder>('upsert_smart_folder', {
    input: {
      id: input.id ?? null,
      name: input.name,
      icon: input.icon ?? null,
      query: input.query,
      sort_order: input.sortOrder ?? 0,
    },
  });
}


export async function backendDeleteSmartFolder(id: string): Promise<void> {
  return invoke<void>('delete_smart_folder', { id });
}


export type BackendVacationSettings = OptionalWireFields<WireVacationDto, 'subject' | 'starts_at' | 'ends_at'>;


export async function backendGetVacation(accountId: string): Promise<BackendVacationSettings | null> {
  return invoke<BackendVacationSettings | null>('get_vacation', { accountId });
}


export async function backendSetVacation(input: {
  accountId: string;
  enabled: boolean;
  subject?: string | null;
  body: string;
  startsAt?: string | null;
  endsAt?: string | null;
  onlyContacts?: boolean;
}): Promise<BackendVacationSettings> {
  return invoke<BackendVacationSettings>('set_vacation', {
    input: {
      account_id: input.accountId,
      enabled: input.enabled,
      subject: input.subject ?? null,
      body: input.body,
      starts_at: input.startsAt ?? null,
      ends_at: input.endsAt ?? null,
      only_contacts: input.onlyContacts ?? false,
    },
  });
}


export type BackendMessageFileAttachment = OptionalWireFields<WireMessageFileAttachment, 'content_id' | 'data_url'>;


export type BackendMessageFilePreview = OptionalWireFields<WireMessageFilePreview, 'from_name' | 'body_html' | 'body_text'>;

/**
 * Parse an `.eml`/`.msg` file for display. Read-only — nothing reaches the
 * database, which is what separates this from `backendImportMbox`.
 */

/**
 * Parse an `.eml`/`.msg` file for display. Read-only — nothing reaches the
 * database, which is what separates this from `backendImportMbox`.
 */
export async function backendOpenMessageFile(
  path: string,
): Promise<BackendMessageFilePreview> {
  return invoke<BackendMessageFilePreview>('open_message_file', { path });
}


/**
 * A part read out of a standalone `.eml`/`.msg` file.
 *
 * Not a `BackendDownloadAttachmentResult`: there is no attachment cache behind
 * a file the app only opened, so no `cached_path` — and the payload is always
 * present. The two were declared as one type, which made the missing field
 * look available.
 */
/** The bytes of one attachment in a message file; mirrors `MessageFileAttachmentData`. */
export type BackendMessageFileAttachmentData = WireMessageFileAttachmentData;

export async function backendReadMessageFileAttachment(
  path: string,
  index: number,
): Promise<BackendMessageFileAttachmentData> {
  return invoke<BackendMessageFileAttachmentData>('read_message_file_attachment', {
    path,
    index,
  });
}

/** Show a message file in its own viewer window. */
export async function backendOpenMessageFileInWindow(path: string): Promise<void> {
  return invoke<void>('open_message_file_in_window', { path });
}


/**
 * Collect the "Open with Satchel" paths that reached the app before this
 * webview existed. Answers once: the second call returns an empty list.
 */
export async function backendTakePendingOpenWith(): Promise<string[]> {
  return invoke<string[]>('take_pending_open_with');
}


export async function backendGetNotificationPreferences(): Promise<BackendNotificationPreferences> {
  return invoke<BackendNotificationPreferences>('get_notification_preferences');
}


export async function backendSetNotificationPreferences(
  preferences: BackendNotificationPreferences,
): Promise<BackendNotificationPreferences> {
  return invoke<BackendNotificationPreferences>('set_notification_preferences', { preferences });
}


export type BackendTrayPreferences = WireTrayPreferencesDto;


export type BackendTrayThread = WireTrayThreadDto;


export type BackendTraySnapshot = WireTraySnapshotDto;


export async function backendTrayPreferences(): Promise<BackendTrayPreferences> {
  return invoke<BackendTrayPreferences>('tray_preferences');
}


export async function backendSetQuitOnClose(enabled: boolean): Promise<void> {
  return invoke<void>('set_quit_on_close', { enabled });
}


export async function backendSetAutostart(enabled: boolean): Promise<void> {
  return invoke<void>('set_autostart', { enabled });
}


export async function backendSetStartMinimized(enabled: boolean): Promise<void> {
  return invoke<void>('set_start_minimized', { enabled });
}


export async function backendTraySnapshot(): Promise<BackendTraySnapshot> {
  return invoke<BackendTraySnapshot>('tray_snapshot');
}


export async function backendOpenTrayThread(threadId: string): Promise<void> {
  return invoke<void>('open_tray_thread', { threadId });
}


export async function backendOpenTrayDestination(
  destination: 'inbox' | 'calendar' | 'compose' | 'settings' | 'calendar_settings',
): Promise<void> {
  return invoke<void>('open_tray_destination', { destination });
}


export async function backendActOnTrayThread(
  threadId: string,
  action: 'done' | 'waiting',
): Promise<void> {
  return invoke<void>('act_on_tray_thread', {
    input: { thread_id: threadId, action },
  });
}


export type BackendConnectionState = OptionalWireFields<WireConnectionStateDto, 'last_sync_at'>;


export async function backendCurrentConnectionState(): Promise<BackendConnectionState> {
  return invoke<BackendConnectionState>('current_connection_state');
}


export type BackendCrashEntry = WireCrashEntry;


export async function backendPendingCrashReports(): Promise<BackendCrashEntry[]> {
  return invoke<BackendCrashEntry[]>('pending_crash_reports');
}


export async function backendSetBackgroundSync(enabled: boolean): Promise<void> {
  return invoke<void>('set_background_sync', { enabled });
}


export type BackendBackgroundSyncStatus = WireBackgroundSyncStatus;


export async function backendBackgroundSyncStatus(): Promise<BackendBackgroundSyncStatus> {
  return invoke<BackendBackgroundSyncStatus>('background_sync_status');
}


export async function backendOpenBackgroundSyncSettings(): Promise<void> {
  return invoke<void>('open_background_sync_settings');
}


export type PendingMobileNotification = OptionalWireFields<WirePendingMobileNotification, 'threadId' | 'actionId'>;


/** The notification tapped while the app was not running, handed over once. */
export async function backendConsumePendingMobileNotification(): Promise<PendingMobileNotification> {
  return invoke<PendingMobileNotification>('consume_pending_mobile_notification');
}


export async function backendDismissMobileNotifications(threadIds: string[]): Promise<void> {
  return invoke<void>('dismiss_mobile_notifications', { threadIds });
}


/** Reserve, activate or dispose the iOS route listener lease. */
export async function backendMobileNotificationsFrontendReady(listenerId: number | null, ready: boolean): Promise<number | null> {
  return invoke<number | null>('mobile_notifications_frontend_ready', { listenerId, ready });
}

/** Consume the desktop notification that launched the app after listeners stand. */
export function backendTakePendingNotificationThread(): Promise<string | null> {
  return invoke<string | null>('take_pending_notification_thread');
}

export async function backendAcknowledgeDraftFlush(saved: boolean): Promise<void> {
  return invoke<void>('acknowledge_draft_flush', { saved });
}
