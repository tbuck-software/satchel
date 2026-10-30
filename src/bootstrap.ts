import type { DataRepairProgress } from './services/backend/generated/app/sync/maintenance/DataRepairProgress';
import { applyLocalePreference } from '@/i18n/runtime';
import { listen, type Event } from '@tauri-apps/api/event';
import { onBeforeUnmount, onMounted, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { bootstrapPersistence } from '@/persistence/bootstrap';
import {
  backendConsumePendingMobileNotification,
  backendDismissMobileNotifications,
  backendGetNotificationPreferences,
  backendImportIcs,
  backendImportIcsPath,
  backendMobileNotificationsFrontendReady,
  backendPendingCrashReports,
  backendUndoOutboxSend,
  backendRunImapSync,
  backendSetBackgroundSync,
  backendSetUiStrings,
  backendSyncGroupware,
  backendSupportsNativeCommands,
  backendCommandsAvailable,
  backendTakePendingOpenWith,
  backendTakePendingNotificationThread,
  backendAcknowledgeDraftFlush,
  backendAcknowledgeAppUpdateFlush,
  type BackendUiStrings,
} from '@/services/backend';
import {
  MAIL_ACTION_TYPE,
  MAIL_ACTIONS,
  notificationAction,
  notificationReplyText,
  notificationRouteThreadId,
  notificationThreadId,
  type NativeNotificationRoute,
} from '@/utils/notificationNavigation';
import { useAccountsStore } from '@/stores/accounts';
import { useAliasesStore } from '@/stores/aliases';
import { useConnectionStore } from '@/stores/connection';
import { useContactsStore } from '@/stores/contacts';
import { useDraftsStore } from '@/stores/drafts';
import { useInvitesStore } from '@/stores/invites';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useMessagesStore } from '@/stores/messages';
import { useOutboxStore } from '@/stores/outbox';
import { useSettingsStore } from '@/stores/settings';
import { useSignaturesStore } from '@/stores/signatures';
import { useSmartFoldersStore } from '@/stores/smartFolders';
import { useTemplatesStore } from '@/stores/templates';
import { useThreadsStore } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { useUiStore } from '@/stores/ui';
import { useWorkflowStore } from '@/stores/workflow';
import { useFollowUpsStore } from '@/stores/followUps';
import { useComposerStore } from '@/stores/composer';
import { useAppUpdateStore } from '@/stores/appUpdate';
import { useToast } from '@/composables/useToast';
import { useQueueFailureToasts } from '@/composables/useQueueFailureToasts';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { useKeyboard } from '@/composables/useKeyboard';
import { useThreadActions } from '@/composables/useThreadActions';
import { createMobileResume } from '@/mobile/resume';
import { useMailEvents } from '@/composables/useMailEvents';
import { ensureNotificationPermission } from '@/composables/useNotifications';
import { useOverlayBackButton } from '@/composables/useOverlayBackButton';
import { drainPendingIcsPaths } from '@/utils/icsOpenWith';
import { installMailtoOpenHandler } from '@/utils/mailtoOpenUrl';
import { installMessageLinkOpenHandler } from '@/utils/messageLinks';
import { useMessageLinks } from '@/composables/useMessageLinks';
import { createCoalescingRunner } from '@/utils/coalescingRunner';
import { createStartupEventBuffer } from '@/utils/startupEventBuffer';
import { scheduleIdleWork } from '@/utils/idleWork';
import { crashReportForClipboard } from '@/utils/crashReport';
import { loadOmniSearch } from '@/components/navigation/loadOmniSearch';
import { loadPublicSuffixTable } from '@/utils/externalLinks';

/** What an event registration hands back so it can be taken down again. */
type Disposer = () => void | Promise<void>;

/**
 * Keep the strings Rust draws — tray, dock menu, notification actions — in
 * the language the user picked.
 *
 * Every window that can change the language has to deliver them, so both
 * bootstrap entries install this.
 */
function useNativeUiStrings(): void {
  const { locale, t } = useI18n();
  const settingsStore = useSettingsStore();
  const appToast = useToast();

  function translatedNativeUiStrings(): BackendUiStrings {
    return {
      lang: locale.value,
      follow_system_locale: settingsStore.localePreference === 'system',
      tray_show: t('nativeUi.tray.show'),
      tray_hide: t('nativeUi.tray.hide'),
      tray_quit: t('nativeUi.tray.quit'),
      tray_briefing: t('nativeUi.tray.briefing'),
      dock_compose: t('nativeUi.dock.compose'),
      dock_calendar: t('nativeUi.dock.calendar'),
      dock_sync: t('nativeUi.dock.sync'),
      dock_settings: t('nativeUi.dock.settings'),
      dock_close_other_windows: t('nativeUi.dock.closeOtherWindows'),
      notification_archive: t('nativeUi.notifications.archive'),
      notification_read: t('nativeUi.notifications.read'),
      notification_trash: t('nativeUi.notifications.trash'),
      notification_send_failed: t('nativeUi.notifications.sendFailed'),
      notification_sync_failed: t('nativeUi.notifications.syncFailed'),
      notification_follow_up_due: t('nativeUi.notifications.followUpDue'),
      notification_no_subject: t('nativeUi.notifications.noSubject'),
      notification_channel_name: t('nativeUi.notifications.channelName'),
      notification_channel_description: t('nativeUi.notifications.channelDescription'),
      notification_new_message: t('nativeUi.notifications.newMessage'),
      notification_syncing: t('nativeUi.notifications.syncing'),
      notification_reply: t('nativeUi.notifications.reply'),
      notification_summary: t('nativeUi.notifications.summary', { count: '{count}' }),
      notification_background_sync_channel: t('nativeUi.notifications.backgroundSyncChannel'),
      notification_background_sync_description: t('nativeUi.notifications.backgroundSyncDescription'),
      notification_background_sync_text: t('nativeUi.notifications.backgroundSyncText'),
      notification_action_failed_title: t('nativeUi.notifications.actionFailedTitle'),
      notification_action_failed_message: t('nativeUi.notifications.actionFailedMessage', {
        action: '{action}',
      }),
      notification_action_not_applied_message: t('nativeUi.notifications.actionNotAppliedMessage', {
        action: '{action}',
      }),
      compose_window_title: t('nativeUi.composeWindowTitle'),
      oauth_invalid_title: t('nativeUi.oauth.invalidTitle'),
      oauth_invalid_callback: t('nativeUi.oauth.invalidCallback'),
      oauth_invalid_state: t('nativeUi.oauth.invalidState'),
      oauth_cancelled_title: t('nativeUi.oauth.cancelledTitle'),
      oauth_cancelled_message: t('nativeUi.oauth.cancelledMessage'),
      oauth_missing_code: t('nativeUi.oauth.missingCode'),
      oauth_connected_title: t('nativeUi.oauth.connectedTitle'),
      oauth_connected_message: t('nativeUi.oauth.connectedMessage', { provider: '{provider}' }),
      oauth_failed_title: t('nativeUi.oauth.failedTitle'),
      oauth_failed_message: t('nativeUi.oauth.failedMessage'),
      oauth_close_hint: t('nativeUi.oauth.closeHint'),
      oauth_provider_fallback: t('nativeUi.oauth.providerFallback'),
    };
  }

  let nativeUiUpdate = Promise.resolve();
  let nativeUiRequest = 0;
  let lastNativeUiPreference = settingsStore.localePreference;
  watch(
    () => settingsStore.localePreference,
    (preference) => {
      if (!backendSupportsNativeCommands()) return;
      const request = ++nativeUiRequest;
      // Serialize rapid language changes so an older IPC response cannot win
      // after a newer selection.
      nativeUiUpdate = nativeUiUpdate
        .catch(() => {})
        .then(async () => {
          if (request !== nativeUiRequest) return;
          await applyLocalePreference(preference, false);
          if (request !== nativeUiRequest) return;
          try {
            await backendSetUiStrings(translatedNativeUiStrings());
            lastNativeUiPreference = preference;
          } catch (error) {
            if (request !== nativeUiRequest) return;
            const rolledBack = settingsStore.localePreference !== lastNativeUiPreference;
            if (rolledBack) {
              settingsStore.setLocalePreference(lastNativeUiPreference);
            }
            appToast.error(t(rolledBack
              ? 'app.language.nativeUpdateRolledBack'
              : 'app.language.nativeUpdateFailed'));
            console.warn('native UI translations unavailable', error);
          }
        });
    },
    { immediate: true },
  );
}

/**
 * Answer the backend's last question before it quits.
 *
 * Quitting through the tray menu or ⌘Q ended the process outright, so up to ten
 * seconds of typing — the autosave interval — went with it, without a word. The
 * backend now asks every window that can hold a draft to write, and leaves once
 * they have all answered or its grace period runs out. A window with no
 * composer open answers straight away: the backend waits for a count of
 * windows, not for content. A window whose draft could not be written says
 * so, and the backend stays open instead of exiting over it.
 */
function useDraftFlushOnExit(): void {
  if (!backendSupportsNativeCommands()) return;
  const { t } = useI18n();
  const toast = useToast();
  let unlisten: (() => void) | null = null;

  void (async () => {
    try {
      const mod = await import('@tauri-apps/api/event');
      const stop = await mod.listen('app:flush-drafts', async () => {
        let saved = false;
        try {
          const { flushOpenComposerDrafts } = await import('@/composables/useComposerDraftPersistence');
          saved = await flushOpenComposerDrafts();
        } finally {
          await backendAcknowledgeDraftFlush(saved);
          if (!saved) toast.error(t('compose.quit.blocked'));
        }
      });
      const stopUpdate = await mod.listen<string>('app-update:flush', async ({ payload }) => {
        let saved = false;
        try {
          const { flushOpenComposerDrafts } = await import('@/composables/useComposerDraftPersistence');
          saved = await flushOpenComposerDrafts();
        } finally {
          await backendAcknowledgeAppUpdateFlush(payload, saved);
          if (!saved) toast.error(t('compose.quit.blocked'));
        }
      });
      const stopUpdateError = await mod.listen<string>('app-update:failed', ({ payload }) => {
        toast.error(backendErrorLabel(payload));
      });
      if (unlisten === null) unlisten = () => { stop(); stopUpdate(); stopUpdateError(); };
      else { stop(); stopUpdate(); stopUpdateError(); }
    } catch (error) {
      console.warn('draft flush bridge failed', error);
    }
  })();

  onBeforeUnmount(() => {
    unlisten?.();
    // A non-null marker would let a listener that arrives after unmount live on.
    unlisten = () => {};
  });
}

/**
 * Install the root application's lifetime orchestration.
 *
 * Call once from the root component setup after Pinia, i18n, and the router
 * have been installed. Setup-time watchers/listeners are registered immediately;
 * asynchronous startup begins on mount and cleans up on unmount.
 */
export function useAppBootstrap(): void {
  useKeyboard();
  useOverlayBackButton();

  const { t } = useI18n();
  const router = useRouter();
  const accountsStore = useAccountsStore();
  const mailboxesStore = useMailboxesStore();
  const threadsStore = useThreadsStore();
  const messagesStore = useMessagesStore();
  const workflowStore = useWorkflowStore();
  const followUpsStore = useFollowUpsStore();
  const aliasesStore = useAliasesStore();
  const signaturesStore = useSignaturesStore();
  const templatesStore = useTemplatesStore();
  const smartFoldersStore = useSmartFoldersStore();
  const topicsStore = useTopicsStore();
  const draftsStore = useDraftsStore();
  const invitesStore = useInvitesStore();
  const outboxStore = useOutboxStore();
  const connectionStore = useConnectionStore();
  const contactsStore = useContactsStore();
  const settingsStore = useSettingsStore();
  const ui = useUiStore();
  const composer = useComposerStore();
  const appUpdates = useAppUpdateStore();
  onMounted(() => appUpdates.start());
  onBeforeUnmount(() => appUpdates.stop());
  const threadActions = useThreadActions();
  const messageLinks = useMessageLinks();

  let imapAutoSyncTimer: number | null = null;
  let imapAutoSyncRunning = false;
  let groupwareSyncTimer: number | null = null;

  useNativeUiStrings();
  useDraftFlushOnExit();

  function openWaitingForReply() {
    threadsStore.workflowFilter = 'later';
    threadsStore.setMailboxFilter({ kind: 'all', value: null });
    void router.push({ name: 'inbox' });
  }

  const IMAP_SYNC_FAST_MS = 5000;
  // Fast Inbox freshness is handled by the native per-account supervisors. This
  // slower complete pass reconciles secondary folders without continuously
  // competing with the Inbox poll for the account lock.
  const IMAP_SYNC_IDLE_MS = 5 * 60_000;
  const SIGNATURE_DISCOVERY_INTERVAL_MS = 7 * 24 * 60 * 60_000;
  const GROUPWARE_SYNC_MS = 5 * 60_000;
  useQueueFailureToasts();

  let icsDragOverHandler: ((event: DragEvent) => void) | null = null;
  let icsDropHandler: ((event: DragEvent) => void) | null = null;
  /*
   * Every event registration that succeeded, in one place. Thirteen named
   * variables and thirteen `if` blocks in the teardown meant a listener could
   * be added and its disposal forgotten with nothing to notice.
   */
  const eventDisposers: Disposer[] = [];
  let mobileNavigationInFlight: Promise<void> | null = null;
  let imapSyncInFlight: ReturnType<typeof backendRunImapSync> | null = null;
  let mobileVisibilityHandler: (() => void) | null = null;
  let cancelInteractionPrefetch: (() => void) | null = null;

  async function loadCoreStores() {
    await Promise.all([
      accountsStore.load(),
      mailboxesStore.load(),
      threadsStore.load(),
      workflowStore.load(),
    ]);
    if (threadsStore.activeThreadId) {
      await messagesStore.loadByThread(threadsStore.activeThreadId);
    }
  }

  /*
   * Sync events arrive in bursts: the IDLE supervisor fires `mail:incoming`
   * repeatedly during a first sync while the 5 s auto-sync runs beside it, and
   * each event used to pull the whole head page, the mailbox list and the open
   * conversation again. The runner shares an in-flight refresh between all
   * callers; the window behind it folds a burst into one follow-up run. The
   * first event of a burst is never delayed.
   */
  const MAIL_REFRESH_WINDOW_MS = 300;
  let mailRefreshWindowTimer: number | null = null;
  let mailRefreshRequestedInWindow = false;
  const mailRefreshRunner = createCoalescingRunner(
    async () => {
      const reloads: Promise<unknown>[] = [threadsStore.refreshHead(), mailboxesStore.load()];
      // Also refresh the currently open conversation — without this a reply
      // arriving in the open thread stays invisible until re-selection. Only
      // its previews: bodies and inline images already on screen stay.
      const openThreadId = threadsStore.activeThreadId;
      messagesStore.markThreadsStale(openThreadId);
      if (openThreadId) {
        reloads.push(messagesStore.refreshThread(openThreadId));
      }
      await Promise.all(reloads);
    },
    () => true,
  );

  function requestMailRefresh() {
    if (mailRefreshWindowTimer !== null) {
      mailRefreshRequestedInWindow = true;
      return;
    }
    mailRefreshWindowTimer = window.setTimeout(() => {
      mailRefreshWindowTimer = null;
      if (!mailRefreshRequestedInWindow) return;
      mailRefreshRequestedInWindow = false;
      void mailRefreshRunner.run();
    }, MAIL_REFRESH_WINDOW_MS);
    void mailRefreshRunner.run();
  }

  function runImapSyncOnce(): ReturnType<typeof backendRunImapSync> {
    if (imapSyncInFlight) return imapSyncInFlight;
    imapSyncInFlight = backendRunImapSync().finally(() => {
      imapSyncInFlight = null;
    });
    return imapSyncInFlight;
  }

  // Backend-side changes (notification actions and moves discovered during mail
  // sync) emit `thread:updated`; refresh the exact cached row before merging the
  // head page so threads that left the Inbox do not survive in Smart Folders.
  const pendingThreadIds = new Set<string>();
  let threadBatchTimer: ReturnType<typeof setTimeout> | null = null;
  let threadBatchRunning = false;
  let threadResyncPending = false;
  async function flushThreadUpdates() {
    threadBatchTimer = null;
    if (threadBatchRunning || bootstrapDisposed) return;
    threadBatchRunning = true;
    const ids = [...pendingThreadIds];
    pendingThreadIds.clear();
    const resync = threadResyncPending;
    threadResyncPending = false;
    try {
      if (resync) await Promise.all([threadsStore.refreshHead({ revalidateCachedRows: true }), connectionStore.refresh()]);
      else await threadsStore.refreshThreads(ids);
      requestMailRefresh();
    } finally {
      threadBatchRunning = false;
      if (pendingThreadIds.size || threadResyncPending) scheduleThreadUpdates();
    }
  }
  function scheduleThreadUpdates() {
    if (!threadBatchTimer && !threadBatchRunning) threadBatchTimer = setTimeout(() => void flushThreadUpdates(), 100);
  }
  useMailEvents({
    onThreadsUpdated: ids => {
      ids.forEach(id => pendingThreadIds.add(id));
      scheduleThreadUpdates();
    },
    onResync: () => {
      threadResyncPending = true;
      scheduleThreadUpdates();
    },
  });
  onBeforeUnmount(() => {
    if (threadBatchTimer) clearTimeout(threadBatchTimer);
    pendingThreadIds.clear();
  });

  watch(() => connectionStore.retryToken, async () => {
    if (!backendCommandsAvailable() || connectionStore.retrying) return;
    connectionStore.retrying = true;
    try {
      // Auxiliary lists report into the same banner; its retry must reach them.
      await Promise.all([
        loadCoreStores(),
        outboxStore.load(),
        connectionStore.backendErrorFor('drafts') ? draftsStore.load() : undefined,
        connectionStore.backendErrorFor('templates') ? templatesStore.load() : undefined,
        connectionStore.backendErrorFor('smartFolders') ? smartFoldersStore.load() : undefined,
        connectionStore.refresh(),
      ]);
    } finally {
      connectionStore.retrying = false;
    }
  });

  /**
   * Follow a notification that was tapped while the app was not running.
   *
   * Android hands the thread id over exactly once and clears it, so a later
   * launch does not jump somewhere the user never asked to go.
   */
  function consumePendingMobileNavigation(): Promise<void> {
    if (mobileNavigationInFlight) return mobileNavigationInFlight;

    mobileNavigationInFlight = (async () => {
      const pending = await backendConsumePendingMobileNotification();
      const threadId = pending.threadId?.trim();
      if (threadId) {
        await handleNotificationAction({ actionId: pending.actionId }, threadId);
      }
    })().finally(() => {
      mobileNavigationInFlight = null;
    });
    return mobileNavigationInFlight;
  }

  const mobileResume = createMobileResume({
    isMobile: () => document.documentElement.dataset.runtimeMobile === 'true',
    consumePendingNavigation: consumePendingMobileNavigation,
    loadThreads: () => threadsStore.refreshHead({ revalidateCachedRows: true }),
    activeThreadId: () => threadsStore.activeThreadId,
    loadMessages: (threadId) => messagesStore.refreshThread(threadId),
    loadSecondaryStores: () => [
      accountsStore.load(),
      mailboxesStore.load(),
      workflowStore.load(),
      connectionStore.refresh(),
    ],
    runServerSync: runImapSyncOnce,
  });
  const refreshMobileMailState = mobileResume.refreshMailState;

  /**
   * Act on a notification without making the user open the app first.
   *
   * Filing and marking read finish in place; only replying needs the composer,
   * because a reply the user cannot see before it leaves is not a favour. The
   * text they typed into the notification is carried into it.
   */
  async function handleNotificationAction(
    notification: Parameters<typeof notificationThreadId>[0],
    threadId: string,
  ) {
    const action = notificationAction(notification);

    if (action === MAIL_ACTIONS.archive) {
      await threadsStore.ensureThreadLoaded(threadId);
      await threadActions.archive([threadId]);
      await backendDismissMobileNotifications([threadId]).catch(() => {});
      return;
    }
    if (action === MAIL_ACTIONS.read) {
      await threadsStore.ensureThreadLoaded(threadId);
      await threadActions.setRead([threadId], true);
      return;
    }

    if (action === MAIL_ACTIONS.reply) {
      await threadsStore.ensureThreadLoaded(threadId);
    }
    // Reply, or a plain tap on the body: both land in the conversation. The
    // route renders immediately and loads a missing thread by id itself.
    await router.push({ name: 'thread', params: { id: threadId } });
    if (action === MAIL_ACTIONS.reply) {
      // Opened rather than sent: a reply the user cannot re-read before it
      // leaves is not a favour, and the notification field has no formatting,
      // no recipient list and no signature.
      composer.open({ mode: 'reply', threadId, body: notificationReplyText(notification) });
      await backendDismissMobileNotifications([threadId]).catch(() => {});
    }
  }

  /**
   * Restore notification delivery before loading the mailbox UI.
   *
   * A large local mailbox can make the first store load take a while. The IMAP
   * foreground service must not wait behind that work or Android can background
   * the activity before the service ever starts.
   */
  async function restoreBackgroundNotifications() {
    try {
      const prefs = await backendGetNotificationPreferences();
      await backendSetBackgroundSync(prefs.enabled);
      if (prefs.enabled) {
        void ensureNotificationPermission().catch((error) => {
          console.warn('notification permission unavailable', error);
        });
      }
    } catch (error) {
      console.warn('background sync unavailable', error);
    }
  }

  const startupEvents = createStartupEventBuffer();
  let bootstrapDisposed = false;

  /**
   * Keep a listener for the mounted app, or release it at once when the root
   * unmounted while its registration was still awaiting.
   */
  function adoptDisposer(dispose: Disposer) {
    if (bootstrapDisposed) void dispose();
    else eventDisposers.push(dispose);
  }

  async function registerStartupEvents() {
    if (!backendSupportsNativeCommands()) return;
    const toast = useToast();
    const bufferStartupEvent = <T>(handler: (event: Event<T>, receivedAt: number) => unknown) =>
      (event: Event<T>) => {
        const receivedAt = Date.now();
        startupEvents.push(() => handler(event, receivedAt));
      };
    const registrations: { name: string; register: () => Promise<Disposer> }[] = [
      {
        name: 'outbox:undo-window',
        register: () => listen<{
          queued_id: string;
          undo_seconds?: number;
        }>('outbox:undo-window', bufferStartupEvent(async (event, receivedAt) => {
          const id = event.payload?.queued_id;
          const seconds = Math.max(1, event.payload?.undo_seconds ?? 5);
          if (!id) return;
          const remaining = receivedAt + seconds * 1000 - Date.now();
          if (remaining <= 0) return;
          toast.info(t('app.mail.undoWindow', { seconds }), {
            timeout: remaining,
            action: {
              label: t('app.mail.undo'),
              present: 'fab',
              handler: async () => {
                try {
                  const draft = await backendUndoOutboxSend(id);
                  await Promise.all([draftsStore.load(), outboxStore.load()]);
                  await composer.openDraft(draft.id);
                  toast.success(t('app.mail.sendCancelled'));
                } catch (err) {
                  toast.error(t('app.mail.cancelFailed', {
                    error: err instanceof Error ? err.message : String(err),
                  }));
                  return false;
                }
              },
            },
          });
        })),
      },
      {
        // Emitted by the native notification delegate when a notification is
        // clicked — it carries the thread id the notification was created with,
        // so this is exact rather than a guess about which mail was meant.
        name: 'mail:focus-thread',
        register: () => listen<{ thread_id: string }>('mail:focus-thread', bufferStartupEvent(async (event) => {
          const id = event.payload?.thread_id;
          if (!id) return;
          await router.push({ name: 'thread', params: { id } });
        })),
      },
      {
        name: 'tray:navigate',
        register: () => listen<{
          destination?: 'inbox' | 'calendar' | 'compose' | 'settings' | 'calendar_settings';
        }>('tray:navigate', bufferStartupEvent(async (event) => {
          switch (event.payload?.destination) {
            case 'calendar':
              await router.push({ name: 'calendar' });
              break;
            case 'compose':
              composer.openNew();
              break;
            case 'settings':
              ui.openSettings();
              break;
            case 'calendar_settings':
              ui.openSettings('calendar');
              break;
            case 'inbox':
            default:
              await router.push({ name: 'inbox' });
              break;
          }
        })),
      },
      {
        name: 'account:auth-required',
        register: () => listen<{ account_id: string; error?: string }>(
          'account:auth-required',
          bufferStartupEvent(async (event) => {
            const accountId = event.payload?.account_id;
            if (!accountId) return;
            // The persistent account banner is the single source of truth for
            // expired authentication. Sync can emit this event repeatedly; a
            // persistent toast for every retry quickly covers the entire UI.
            await accountsStore.load();
          }),
        ),
      },
    ];
    const outcomes = await Promise.allSettled(registrations.map((entry) => entry.register()));
    outcomes.forEach((outcome, index) => {
      if (outcome.status === 'fulfilled') adoptDisposer(outcome.value);
      else console.warn(`event listener ${registrations[index]!.name} failed to register`, outcome.reason);
    });
    if (bootstrapDisposed) return;
    // The native delegate retains a cold-start click until this pull marks us ready.
    const threadId = await backendTakePendingNotificationThread();
    if (threadId) startupEvents.push(() => router.push({ name: 'thread', params: { id: threadId } }));
  }

  onMounted(async () => {
    await registerStartupEvents().catch((error) => console.warn('startup event registration failed', error));
    if (bootstrapDisposed) return;
    // Start the foreground-service restore first on mobile, then begin the
    // visible mailbox path in the same mounted tick. They run concurrently:
    // cached mail no longer waits for notification IPC to finish.
    const notificationRestore = backendSupportsNativeCommands()
      ? (async () => { await restoreBackgroundNotifications(); })()
      : Promise.resolve();
    const coreStoresLoad = (async () => { await loadCoreStores(); })().catch((error) => {
      console.error('core stores failed to load', error);
      connectionStore.reportBackendError('threads', error);
    });
    void notificationRestore;

    if (backendSupportsNativeCommands()) {
      void (async () => {
        try {
          const { listen } = await import('@tauri-apps/api/event');
          // iOS: a tap the native side held back until this listener existed.
          adoptDisposer(await listen<NativeNotificationRoute>('mobile:notification-route', ({ payload }) => {
            const threadId = notificationRouteThreadId(payload);
            if (threadId) void router.push({ name: 'thread', params: { id: threadId } });
          }));
          adoptDisposer(await listen('mobile:resumed', () => {
            void refreshMobileMailState();
          }));
          adoptDisposer(await listen('mobile:notification-navigation', () => {
            void consumePendingMobileNavigation();
          }));
          if (bootstrapDisposed) return;
          mobileVisibilityHandler = () => {
            if (document.visibilityState === 'visible') {
              void refreshMobileMailState();
            }
          };
          window.addEventListener('focus', mobileVisibilityHandler);
          document.addEventListener('visibilitychange', mobileVisibilityHandler);
          await backendMobileNotificationsFrontendReady();
        } catch (error) {
          console.warn('mobile notification bridge failed', error);
        }
      })();
    }

    // Follow a notification before the full Inbox query. A large local mailbox
    // must not delay opening the conversation the user explicitly selected.
    await consumePendingMobileNavigation().catch((error) => {
      console.warn('pending mobile notification action failed', error);
    });

    // The load already ran alongside the bridge work above. Waiting here keeps
    // the remainder of startup ordered without putting that work in front of
    // the first Inbox rows.
    await coreStoresLoad;
    await startupEvents.flush();
    // Every await below can outlive the root; nothing may start after unmount.
    if (bootstrapDisposed) return;
    cancelInteractionPrefetch = scheduleIdleWork(() => {
      // The reader and command palette are the two most common first actions.
      // Warm their code only after the Inbox is usable, so the initial download
      // stays lean while the first click no longer waits on a chunk round trip.
      // The reader's link assessment wants the public-suffix table as well.
      void Promise.allSettled([
        import('@/components/mail/ThreadView.vue'),
        loadOmniSearch(),
        loadPublicSuffixTable(),
      ]);
    });
    if (!settingsStore.onboardingComplete && accountsStore.accounts.length > 0) {
      settingsStore.completeOnboarding();
    }
    // Initialise auxiliary stores in parallel; failures are non-fatal.
    await Promise.allSettled([
      aliasesStore.load(),
      signaturesStore.load(),
      templatesStore.load(),
      smartFoldersStore.load(),
      topicsStore.load(),
      contactsStore.load(),
      draftsStore.load(),
      invitesStore.load(),
      outboxStore.start(),
      connectionStore.start(),
    ]);
    if (bootstrapDisposed) return;
    void contactsStore.refreshLocalAvatars();

    if (backendSupportsNativeCommands()) {
      const lastDiscovery = bootstrapPersistence.lastSignatureDiscoveryAt.read();
      if (!Number.isFinite(lastDiscovery) || Date.now() - lastDiscovery >= SIGNATURE_DISCOVERY_INTERVAL_MS) {
        const scans = accountsStore.accounts.flatMap((account) => {
          const identities = [
            account.email,
            ...aliasesStore.aliasesFor(account.id).map((alias) => alias.email),
          ];
          return identities.map((identityEmail) =>
            signaturesStore.discover(account.id, identityEmail),
          );
        });
        const results = await Promise.allSettled(scans);
        if (bootstrapDisposed) return;
        const suggestionIds = new Set(
          results.flatMap((result) =>
            result.status === 'fulfilled' ? result.value.map((item) => item.id) : [],
          ),
        );
        // Only start the quiet period once something was found. An empty scan is
        // usually a first launch whose sent folder has not synced yet — stamping
        // it would silence discovery for a week exactly when it matters most.
        if (suggestionIds.size > 0) {
          bootstrapPersistence.lastSignatureDiscoveryAt.write(Date.now());
          useToast().info(
            t('app.signatures.discovered', { count: suggestionIds.size }, suggestionIds.size),
            { action: { label: t('common.show'), handler: () => ui.openSettings('signatures') } },
          );
        }
      }
    }

    // DAV sync is independent from mail transport. Keep it on a slower timer so
    // CalDAV / CardDAV changes arrive without delaying or coupling them to mail sync.
    if (backendSupportsNativeCommands()) {
      const runGroupwareSync = async () => {
        try {
          await backendSyncGroupware();
          if (bootstrapDisposed) return;
          await Promise.all([contactsStore.load(), invitesStore.load()]);
        } catch {
          // Configuration and authentication errors are surfaced in account settings.
        } finally {
          // `onBeforeUnmount` already cleared the timer; do not arm a new one.
          if (!bootstrapDisposed) groupwareSyncTimer = window.setTimeout(() => {
            void runGroupwareSync();
          }, GROUPWARE_SYNC_MS);
        }
      };
      void runGroupwareSync();
    }

    // Surface any unhandled panic from the previous run as a one-time toast.
    if (backendSupportsNativeCommands()) {
      try {
        const crashes = await backendPendingCrashReports();
        if (crashes.length > 0 && !bootstrapDisposed) {
          const toast = useToast();
          const latestCrash = crashes.at(-1)!;
          const wasAnr = latestCrash.raw.startsWith('==== android anr at ');
          // The raw payload stays out of the message — it is a multi-line
          // technical dump that wraps over half the phone screen. The report
          // remains reachable through the copy action; the toast must stay
          // sticky because this read drains the backend's pending crashes.
          toast.error(
            wasAnr
              ? t('app.crash.notResponding')
              : t('app.crash.crashed'),
            {
              timeout: 0,
              action: {
                label: t('app.crash.copyReport'),
                handler: async () => {
                  try {
                    if (!navigator.clipboard) throw new Error('Clipboard unavailable');
                    await navigator.clipboard.writeText(crashReportForClipboard(latestCrash.raw));
                    toast.success(t('app.crash.copied'));
                  } catch {
                    toast.error(t('app.crash.copyFailed'));
                  }
                },
              },
            },
          );
        }
      } catch (err) {
        console.warn('crash report check failed', err);
      }
    }
    if (bootstrapDisposed) return;

    // Listen to mail-incoming events from the IMAP IDLE supervisor and refresh threads.
    // Failed outbox state is surfaced by the persistent store-backed toast above.
    if (backendSupportsNativeCommands()) {
      const toast = useToast();
      const mod = await import('@tauri-apps/api/event').catch((err) => {
        console.warn('event module unavailable', err);
        return null;
      });

      if (mod && !bootstrapDisposed) {
        const importIcsPath = async (path: string) => {
          try {
            const invite = await backendImportIcsPath(path);
            await invitesStore.load();
            toast.success(
              t('app.ics.imported', { name: invite.summary || invite.uid }),
              {
                action: {
                  label: t('common.show'),
                  handler: () => router.push({ name: 'inbox' }),
                },
              },
            );
          } catch (err) {
            toast.error(t('app.ics.importFailed', {
              error: err instanceof Error ? err.message : String(err),
            }));
          }
        };

        /*
         * One entry per listener, registered independently.
         *
         * These used to be ten sequential `await`s in one `try`. A throw in the
         * middle — a platform without the notification plugin, a user who said
         * no to notifications — silently dropped every registration after it,
         * and the registration order became an unwritten dependency between
         * features that have nothing to do with each other. The expired-auth
         * banner and the `.ics` handler sat behind exactly that throw.
         */
        const DATA_REPAIR_TOAST_GROUP = 'data-repair';
        let dataRepairNoticeShown = false;
        const registrations: { name: string; register: () => Promise<Disposer> }[] = [
          {
            name: 'mail:incoming',
            register: () => mod.listen('mail:incoming', () => {
              requestMailRefresh();
            }),
          },
          {
            name: 'outbox:sent',
            register: () => mod.listen<{ action_type?: string }>('outbox:sent', (event) => {
              // Archive/star mutations share the outbox queue. The worker keeps them
              // out of this event, but a stray emitter must not claim a send again.
              const type = event.payload?.action_type;
              if (type && type !== 'send_message') return;
              toast.success(t('app.mail.sent'));
            }),
          },
          {
            // A data repair a migration handed to the background. One quiet,
            // persistent note while it runs, gone when it is done — the old
            // alternative was a start that hung without saying why.
            name: 'data-repair:progress',
            register: () => mod.listen<DataRepairProgress>('data-repair:progress', (event) => {
              const running = event.payload?.running === true;
              if (running === dataRepairNoticeShown) return;
              dataRepairNoticeShown = running;
              if (!running) {
                toast.dismissGroup(DATA_REPAIR_TOAST_GROUP);
                return;
              }
              toast.info(t('app.dataRepair.running'), {
                timeout: 0,
                groupKey: DATA_REPAIR_TOAST_GROUP,
              });
            }),
          },
          {
            name: 'plugin:auto-disabled',
            register: () => mod.listen<{ plugin_id?: string }>('plugin:auto-disabled', (event) => {
              const plugin = event.payload?.plugin_id;
              if (!plugin) return;
              toast.warning(t('app.plugins.autoDisabled', { plugin }), { timeout: 0 });
            }),
          },
          {
            name: 'snooze:released',
            register: () => mod.listen('snooze:released', () => {
              requestMailRefresh();
            }),
          },
          {
            name: 'followup:due',
            register: () => mod.listen<{ count?: number }>('followup:due', (event) => {
              void threadsStore.refreshHead();
              void followUpsStore.load();
              const dueCount = event.payload?.count ?? 1;
              toast.info(t('app.followUps.due', { count: dueCount }, dueCount), {
                action: {
                  label: t('common.show'),
                  handler: openWaitingForReply,
                },
              });
            }),
          },
          {
            // The mobile half of the same story. `onAction` fires when the OS reports
            // a tap on Android and iOS, where the plugin does deliver actions; on the
            // desktop it never fires and the native delegate in `utils::mac_notify`
            // emits `mail:focus-thread` instead. Both end at the same route.
            name: 'notification:action',
            register: async () => {
              const { onAction, registerActionTypes } = await import('@tauri-apps/plugin-notification');
              // Registering the type is what makes the platform draw the buttons
              // on the notification; without it a notification is body-only. It
              // has to happen before `onAction`, which is why the two share one
              // entry instead of standing next to each other in the list.
              await registerActionTypes([
                {
                  id: MAIL_ACTION_TYPE,
                  actions: [
                    { id: MAIL_ACTIONS.archive, title: t('common.done') },
                    { id: MAIL_ACTIONS.read, title: t('app.notifications.read') },
                    {
                      id: MAIL_ACTIONS.reply,
                      title: t('common.reply'),
                      input: true,
                      inputButtonTitle: t('ui.send'),
                      inputPlaceholder: t('app.notifications.replyPlaceholder'),
                    },
                  ],
                },
              ]);
              const listener = await onAction((notification) => {
                const id = notificationThreadId(notification);
                if (!id) return;
                void handleNotificationAction(notification, id);
              });
              return () => listener.unregister();
            },
          },
          {
            // From a browser or another app; running or cold start alike.
            name: 'mailto:open',
            register: () => installMailtoOpenHandler((target) => {
              composer.openEmail(target.recipients, { subject: target.subject, body: target.body });
            }),
          },
          {
            // `satchel://message/…` from T3, notes, a browser; running or cold start alike.
            name: 'satchel-link:open',
            register: () => installMessageLinkOpenHandler((link) => void messageLinks.open(link)),
          },
          {
            name: 'ics:open-with',
            register: () => mod.listen('ics:open-with', async () => {
              await drainPendingIcsPaths(backendTakePendingOpenWith, importIcsPath);
            }),
          },
        ];

        const outcomes = await Promise.allSettled(registrations.map((entry) => entry.register()));
        outcomes.forEach((outcome, index) => {
          const name = registrations[index]!.name;
          if (outcome.status === 'fulfilled') adoptDisposer(outcome.value);
          // Named, because "event listeners failed" told nobody which feature
          // just stopped existing.
          else console.warn(`event listener ${name} failed to register`, outcome.reason);
        });

        // Only after the listener stands: a launch from the Finder emits that
        // event long before any of this runs, and Tauri does not replay it.
        if (!bootstrapDisposed) {
          try {
            await drainPendingIcsPaths(backendTakePendingOpenWith, importIcsPath);
          } catch (err) {
            console.warn('pending ics paths could not be drained', err);
          }
        }
      }
    }
    if (bootstrapDisposed) return;

    // Global .ics drop handler — drag any .ics file into the app to import it as a RSVP card.
    if (typeof window !== 'undefined') {
      const toast = useToast();
      icsDragOverHandler = (event: DragEvent) => {
        if (!event.dataTransfer) return;
        const items = Array.from(event.dataTransfer.items ?? []);
        if (items.some((it) => it.type === 'text/calendar')) {
          event.preventDefault();
        }
      };
      icsDropHandler = async (event: DragEvent) => {
        const files = Array.from(event.dataTransfer?.files ?? []);
        const ics = files.filter(
          (f) => f.type === 'text/calendar' || f.name.toLowerCase().endsWith('.ics'),
        );
        if (ics.length === 0) return;
        event.preventDefault();
        for (const file of ics) {
          try {
            const raw = await file.text();
            const invite = await backendImportIcs({ raw, sourceFilename: file.name });
            await invitesStore.load();
            toast.success(
              t('app.ics.imported', { name: invite.summary || invite.uid }),
              {
                action: {
                  label: t('common.show'),
                  handler: () => router.push({ name: 'inbox' }),
                },
              },
            );
          } catch (err) {
            toast.error(t('app.ics.fileFailed', {
              name: file.name,
              error: err instanceof Error ? err.message : String(err),
            }));
          }
        }
      };
      window.addEventListener('dragover', icsDragOverHandler);
      window.addEventListener('drop', icsDropHandler);
    }

    if (backendSupportsNativeCommands()) {
      const scheduleNext = (delayMs: number) => {
        if (bootstrapDisposed) return;
        if (imapAutoSyncTimer !== null) {
          window.clearTimeout(imapAutoSyncTimer);
        }
        imapAutoSyncTimer = window.setTimeout(() => {
          void runAutoSync();
        }, delayMs);
      };

      const runAutoSync = async () => {
        if (imapAutoSyncRunning) {
          scheduleNext(IMAP_SYNC_FAST_MS);
          return;
        }

        imapAutoSyncRunning = true;
        try {
          const summary = await runImapSyncOnce();
          if (bootstrapDisposed) return;
          connectionStore.applySyncSummary(summary);
          if (summary.messages_synced > 0) {
            await Promise.all([threadsStore.refreshHead({ revalidateCachedRows: true }), mailboxesStore.load()]);
          }
          // A per-account failure does not reject the call — it rides along in
          // `failures`. Swallowing it made an account that had stopped syncing
          // (expired credentials, an unreachable host, a rate-limit block that
          // outlives a restart) indistinguishable from an empty mailbox.
          connectionStore.reportSyncFailures(summary.failures);
          if (summary.backfill_pending_accounts > 0) {
            scheduleNext(IMAP_SYNC_FAST_MS);
          } else {
            scheduleNext(IMAP_SYNC_IDLE_MS);
          }
        } catch (err) {
          // Auto sync should never interrupt app usage, but it must not vanish
          // either — this is the only signal the user gets that mail stopped.
          connectionStore.reportBackendError('sync', err);
          scheduleNext(IMAP_SYNC_FAST_MS);
        } finally {
          imapAutoSyncRunning = false;
        }
      };

      void runAutoSync();
    }
  });

  onBeforeUnmount(() => {
    bootstrapDisposed = true;
    startupEvents.dispose();
    cancelInteractionPrefetch?.();
    cancelInteractionPrefetch = null;
    if (mailRefreshWindowTimer !== null) {
      window.clearTimeout(mailRefreshWindowTimer);
      mailRefreshWindowTimer = null;
    }
    mailRefreshRunner.cancelPending();
    if (imapAutoSyncTimer !== null) {
      window.clearTimeout(imapAutoSyncTimer);
      imapAutoSyncTimer = null;
    }
    if (groupwareSyncTimer !== null) {
      window.clearTimeout(groupwareSyncTimer);
      groupwareSyncTimer = null;
    }
    outboxStore.stop();
    connectionStore.stop();
    while (eventDisposers.length > 0) {
      try {
        void eventDisposers.pop()?.();
      } catch (err) {
        console.warn('event listener could not be removed', err);
      }
    }
    if (mobileVisibilityHandler) {
      window.removeEventListener('focus', mobileVisibilityHandler);
      document.removeEventListener('visibilitychange', mobileVisibilityHandler);
      mobileVisibilityHandler = null;
    }
    if (typeof window !== 'undefined') {
      if (icsDragOverHandler) {
        window.removeEventListener('dragover', icsDragOverHandler);
        icsDragOverHandler = null;
      }
      if (icsDropHandler) {
        window.removeEventListener('drop', icsDropHandler);
        icsDropHandler = null;
      }
    }
  });
}

/**
 * Install the lifetime orchestration of a standalone composer window.
 *
 * A popout exists to write one message. The mailbox, its stores, the global
 * shortcuts and the sync timers belong to the window that shows the mailbox:
 * n popouts next to the main window must still add up to one auto-sync loop,
 * one set of event listeners and one owner of the thread cache. Everything the
 * composer itself needs — accounts, aliases, signatures, templates, the draft —
 * `ComposeView` loads on mount, in every window it runs in.
 *
 * Only the native UI strings are shared: the language can be changed from the
 * settings modal, which any window can open.
 */
export function useComposeWindowBootstrap(): void {
  useNativeUiStrings();
  // The one shared lifetime concern a popout does have: it is where the
  // unsaved draft lives when the user quits.
  useDraftFlushOnExit();
}
