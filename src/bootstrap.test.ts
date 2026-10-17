// @vitest-environment jsdom

import { defineComponent } from 'vue';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendListAccounts,
  backendTakePendingNotificationThread,
  backendListContacts,
  backendImportIcsPath,
  backendTakePendingOpenWith,
  backendListTemplates,
  backendListThreads,
  backendListTopics,
  backendRunImapSync,
  backendSetUiStrings,
  backendUndoOutboxSend,
  backendAcknowledgeDraftFlush,
} from '@/services/backend';
import { useAppBootstrap, useComposeWindowBootstrap } from '@/bootstrap';
import { useToast } from '@/composables/useToast';
import { useComposerStore } from '@/stores/composer';
import { emit, listen } from '@tauri-apps/api/event';
import { onAction, registerActionTypes } from '@tauri-apps/plugin-notification';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

const draftFlush = vi.hoisted(() => ({ result: true }));

vi.mock('@/composables/useComposerDraftPersistence', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/composables/useComposerDraftPersistence')>()),
  flushOpenComposerDrafts: vi.fn(async () => draftFlush.result),
}));

vi.mock('@tauri-apps/plugin-deep-link', () => ({
  onOpenUrl: vi.fn(async () => () => {}),
  getCurrent: vi.fn(async () => null),
}));

vi.mock('@tauri-apps/plugin-notification', () => ({
  onAction: vi.fn(async () => ({ unregister: async () => {} })),
  registerActionTypes: vi.fn(async () => {}),
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => 'granted'),
  sendNotification: vi.fn(),
}));

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  const resolving = <T>(value: T) => vi.fn(async () => value);
  return {
    ...actual,
    backendCommandsAvailable: () => true,
    backendSupportsNativeCommands: () => true,
    backendSetUiStrings: resolving(undefined),
    backendAcknowledgeDraftFlush: resolving(undefined),
    backendRunImapSync: resolving({
      accounts_synced: 0,
      accounts_failed: 0,
      folders_synced: 0,
      messages_synced: 0,
      backfill_pending_accounts: 0,
      backfill_remaining_messages: 0,
      backfill_total_messages: 0,
      failures: [],
    }),
    backendListThreads: resolving({ threads: [], next_cursor: null, has_more: false }),
    backendListCachedThreads: resolving(null),
    backendListSmartInboxSummary: resolving(null),
    backendListContacts: resolving([]),
    backendListTemplates: resolving([]),
    backendListTopics: resolving([]),
    backendListAccounts: resolving([]),
    backendGetSourceCapabilities: resolving({}),
    backendListMailboxes: resolving([]),
    backendListProviderLabels: resolving([]),
    backendListRecentTransitions: resolving([]),
    backendListAliases: resolving([]),
    backendListSignatures: resolving([]),
    backendListSignatureAssignments: resolving([]),
    backendListSignatureSuggestions: resolving([]),
    backendListSmartFolders: resolving([]),
    backendListDrafts: resolving([]),
    backendListInvites: resolving([]),
    backendListCalendarEvents: resolving([]),
    backendListCalendarSources: resolving([]),
    backendListOutbox: resolving([]),
    backendListFailedSyncActions: resolving([]),
    backendUndoOutboxSend: resolving({ id: 'restored-draft' }),
    backendListFollowUps: resolving([]),
    backendCurrentConnectionState: resolving({
      online: true,
      syncing: false,
      last_sync_at: null,
      pending_outbox: 0,
      pending_sends: 0,
      failed_outbox: 0,
      failed_sends: 0,
      failed_remote_mutations: 0,
    }),
    backendGetNotificationPreferences: resolving({ enabled: false }),
    backendSetBackgroundSync: resolving(undefined),
    backendConsumePendingMobileNotification: resolving({ threadId: null, actionId: null }),
    backendMobileNotificationsFrontendReady: resolving(undefined),
    backendPendingCrashReports: resolving([]),
    backendTakePendingOpenWith: resolving([]),
    backendTakePendingNotificationThread: resolving(null),
    backendImportIcsPath: resolving(null),
    backendSyncGroupware: resolving({}),
  };
});

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/inbox', name: 'inbox', component: { template: '<div />' } },
    { path: '/thread/:id', name: 'thread', component: { template: '<div />' } },
    { path: '/compose', name: 'compose', component: { template: '<div />' } },
  ],
});

/** The list loads a popout has no business starting. */
const listLoads = () => [
  vi.mocked(backendListThreads),
  vi.mocked(backendListContacts),
  vi.mocked(backendListTemplates),
  vi.mocked(backendListTopics),
];

const mounted: ReturnType<typeof mount>[] = [];
function boot(entry: () => void) {
  const wrapper = mount(defineComponent({ setup: () => entry(), template: '<div />' }), {
    global: { plugins: [createPinia(), router] },
  });
  mounted.push(wrapper);
  return wrapper;
}

describe('bootstrap entries', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    useToast().clear();
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    await router.push('/inbox');
    await router.isReady();
  });

  afterEach(() => {
    for (const wrapper of mounted.splice(0)) wrapper.unmount();
    useToast().clear();
    vi.restoreAllMocks();
  });

  it('runs the mailbox loads and the auto sync from the application entry', async () => {
    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await flushPromises();

    await vi.waitFor(() => expect(backendRunImapSync).toHaveBeenCalled());
    for (const load of listLoads()) expect(load).toHaveBeenCalled();

    wrapper.unmount();
  });

  it('registers critical listeners before a slow core load and replays buffered events', async () => {
    let release!: (value: never[]) => void;
    vi.mocked(backendListAccounts).mockReturnValueOnce(new Promise(resolve => { release = resolve; }));
    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await vi.waitFor(() => expect(vi.mocked(listen).mock.calls.some(([event]) => event === 'account:auth-required')).toBe(true));
    const callback = (name: string) => vi.mocked(listen).mock.calls.find(([event]) => event === name)![1];
    for (const name of ['mail:focus-thread', 'tray:navigate', 'outbox:undo-window', 'account:auth-required']) {
      expect(vi.mocked(listen).mock.calls.some(([event]) => event === name)).toBe(true);
    }
    callback('mail:focus-thread')({ payload: { thread_id: 'launch-thread' } } as never);
    callback('tray:navigate')({ payload: { destination: 'compose' } } as never);
    callback('outbox:undo-window')({ payload: { queued_id: 'queued', undo_seconds: 10 } } as never);
    callback('account:auth-required')({ payload: { account_id: 'account' } } as never);
    expect(router.currentRoute.value.name).toBe('inbox');
    expect(useToast().state.toasts.some(item => item.action?.present === 'fab')).toBe(false);
    release([]);
    await flushPromises();
    await flushPromises();
    expect(router.currentRoute.value.params.id).toBe('launch-thread');
    expect(useToast().state.toasts.some(item => item.action?.present === 'fab')).toBe(true);
    expect(backendListAccounts).toHaveBeenCalledTimes(2);
    expect(useComposerStore().isOpen).toBe(true);
    wrapper.unmount();
    useToast().clear();
  });

  it('opens the notification retained by the native cold-start catch-up', async () => {
    vi.mocked(backendTakePendingNotificationThread).mockResolvedValueOnce('cold-start');
    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await flushPromises();
    await vi.waitFor(() => expect(router.currentRoute.value.params.id).toBe('cold-start'));
    wrapper.unmount();
  });

  it('starts no sync and loads no lists from the composer window entry', async () => {
    const wrapper = boot(useComposeWindowBootstrap);
    await flushPromises();
    await flushPromises();

    // The application entry runs the first pass immediately and only then arms
    // the timer, so an untouched spy means no loop was started at all.
    expect(backendRunImapSync).not.toHaveBeenCalled();
    for (const load of listLoads()) expect(load).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it('ignores a claimed invitation path and reads only the OS-owned queue', async () => {
    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    const registration = vi.mocked(listen).mock.calls.find(([event]) => event === 'ics:open-with');
    expect(registration).toBeDefined();
    vi.mocked(backendTakePendingOpenWith).mockClear();
    vi.mocked(backendImportIcsPath).mockClear();
    await (registration![1] as (event: unknown) => Promise<void>)({ payload: { path: '/private/forged.ics' } });
    expect(backendTakePendingOpenWith).toHaveBeenCalledOnce();
    expect(backendImportIcsPath).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  /**
   * Quitting used to end the process outright, between two autosave ticks. Both
   * entries answer, because a popout is exactly where the unsaved draft is.
   */
  it('answers the backend\'s flush call from both entries', async () => {
    for (const entry of [useAppBootstrap, useComposeWindowBootstrap]) {
      vi.mocked(listen).mockClear();
      vi.mocked(emit).mockClear();
      vi.mocked(backendAcknowledgeDraftFlush).mockClear();
      const wrapper = boot(entry);
      await flushPromises();

      const registration = vi
        .mocked(listen)
        .mock.calls.find(([event]) => event === 'app:flush-drafts');
      expect(registration, entry.name).toBeDefined();

      await (registration![1] as (event: unknown) => Promise<void>)({});
      expect(backendAcknowledgeDraftFlush, entry.name).toHaveBeenCalledWith(true);

      wrapper.unmount();
    }
  });

  it('reports a draft that could not be saved, so the quit stops', async () => {
    draftFlush.result = false;
    try {
      const wrapper = boot(useComposeWindowBootstrap);
      await flushPromises();
      const registration = vi
        .mocked(listen)
        .mock.calls.find(([event]) => event === 'app:flush-drafts');

      await (registration![1] as (event: unknown) => Promise<void>)({});

      expect(backendAcknowledgeDraftFlush).toHaveBeenCalledWith(false);
      expect(useToast().state.toasts.map((toast) => toast.message)).toContain(
        'Satchel stayed open: a draft could not be saved.',
      );
      wrapper.unmount();
    } finally {
      draftFlush.result = true;
    }
  });

  /*
   * The ten registrations used to be sequential `await`s in one `try`. A
   * platform without the notification plugin — or a user who declined the
   * permission — made `onAction` throw, and everything after it silently never
   * registered: the expired-authentication banner and the `.ics` handler.
   */
  it('registers every listener even when one of them fails', async () => {
    vi.mocked(onAction).mockRejectedValueOnce(new Error('notifications unavailable'));

    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await flushPromises();

    const events = vi.mocked(listen).mock.calls.map(([event]) => event);
    await vi.waitFor(() => expect(vi.mocked(listen).mock.calls.map(([event]) => event)).toContain('account:auth-required'));
    expect(events).toContain('ics:open-with');

    const named = vi.mocked(console.warn).mock.calls
      .map(([message]) => message)
      .filter((message): message is string => typeof message === 'string')
      .filter((message) => message.startsWith('event listener '));
    expect(named).toEqual(['event listener notification:action failed to register']);

    wrapper.unmount();
  });

  it('takes down every listener that did register', async () => {
    const disposers: (() => void)[] = [];
    vi.mocked(listen).mockImplementation(async () => {
      const dispose = vi.fn();
      disposers.push(dispose);
      return dispose;
    });
    vi.mocked(onAction).mockRejectedValueOnce(new Error('notifications unavailable'));

    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await flushPromises();
    await vi.waitFor(() => expect(disposers.length).toBeGreaterThan(9));

    wrapper.unmount();
    expect(disposers.filter((dispose) => vi.mocked(dispose).mock.calls.length === 0)).toEqual([]);
  });

  /** The buttons only exist if the type was registered before the handler. */
  it('registers the notification action type before its handler', async () => {
    const order: string[] = [];
    vi.mocked(registerActionTypes).mockImplementation(async () => {
      order.push('registerActionTypes');
    });
    vi.mocked(onAction).mockImplementation(async () => {
      order.push('onAction');
      // Only `unregister` is ever called; the rest of PluginListener is noise.
      return { unregister: async () => {} } as unknown as Awaited<ReturnType<typeof onAction>>;
    });

    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await flushPromises();

    await vi.waitFor(() => expect(order).toEqual(['registerActionTypes', 'onAction']));
    wrapper.unmount();
  });

  it('restores Undo Send into a draft and reopens the composer', async () => {
    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    await flushPromises();
    const registration = vi.mocked(listen).mock.calls.find(([name]) => name === 'outbox:undo-window');
    expect(registration).toBeDefined();
    const handler = registration![1] as (event: unknown) => void;
    handler({ payload: { queued_id: 'queued-send', undo_seconds: 5 } });
    // Startup buffers app events until the stores are ready.
    await vi.waitFor(() => expect(
      useToast().state.toasts.some((toast) => toast.action?.present === 'fab'),
    ).toBe(true));
    const undo = useToast().state.toasts.find((toast) => toast.action?.present === 'fab');
    await undo!.action!.handler();
    expect(backendUndoOutboxSend).toHaveBeenCalledWith('queued-send');
    expect(useComposerStore().request?.draftId).toBe('restored-draft');
    wrapper.unmount();
  });

  it('delivers the native UI strings from both entries', async () => {
    const app = boot(useAppBootstrap);
    await flushPromises();
    expect(backendSetUiStrings).toHaveBeenCalled();
    app.unmount();

    vi.mocked(backendSetUiStrings).mockClear();

    const popout = boot(useComposeWindowBootstrap);
    await flushPromises();
    expect(backendSetUiStrings).toHaveBeenCalled();
    popout.unmount();
  });
});
