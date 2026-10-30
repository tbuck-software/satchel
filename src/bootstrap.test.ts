// @vitest-environment jsdom

import { defineComponent } from 'vue';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendMobileNotificationsFrontendReady,
  backendDismissMobileNotifications,
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
  backendSyncGroupware,
  backendTransitionThreads,
} from '@/services/backend';
import { useAppBootstrap, useComposeWindowBootstrap } from '@/bootstrap';
import ComposerHost from '@/components/mail/ComposerHost.vue';
import type { Thread } from '@/types/mail';
import { useToast } from '@/composables/useToast';
import { useThreadsStore } from '@/stores/threads';
import * as threadActionModule from '@/composables/useThreadActions';
import { useComposerStore, type ComposerReplaceOutcome } from '@/stores/composer';
import { useConnectionStore } from '@/stores/connection';
import { emit, listen } from '@tauri-apps/api/event';
import { onAction, registerActionTypes } from '@tauri-apps/plugin-notification';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

const draftFlush = vi.hoisted(() => ({ result: true }));

/** The form the real ComposerHost talks to when a notification replaces its draft. */
const composeForm = vi.hoisted(() => ({
  persistDraft: vi.fn(async (): Promise<string | null> => null),
  shouldConfirmClose: vi.fn(() => false),
  closeWithoutSaving: vi.fn(async () => {}),
  currentDraftId: vi.fn((): string | null => null),
}));

vi.mock('@/views/ComposeView.vue', () => ({
  default: {
    setup(_props: unknown, { expose }: { expose: (handle: unknown) => void }) {
      expose(composeForm);
      return () => undefined;
    },
  },
}));

vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => ({ ask: async () => true }) }));

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
    backendDismissMobileNotifications: resolving(undefined),
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
    backendMobileNotificationsFrontendReady: vi.fn(async (id: number | null) => id ?? 1),
    backendPendingCrashReports: resolving([]),
    backendTakePendingOpenWith: resolving([]),
    backendTakePendingNotificationThread: resolving(null),
    backendImportIcsPath: resolving(null),
    backendSyncGroupware: resolving({}),
    backendTransitionThreads: vi.fn(async (input: { threads: { thread_id: string }[] }) => ({
      applied: input.threads.map(entry => entry.thread_id),
      failed: [],
    })),
  };
});

function inboxThread(id: string): Thread {
  return {
    id,
    accountId: 'account',
    subject: 'Subject',
    participants: [{ name: 'Sender', email: 'sender@example.com' }],
    snippet: 'Preview',
    lastMessageAt: '2026-10-30T08:00:00.000Z',
    lastMessageAtMs: Date.parse('2026-10-30T08:00:00.000Z'),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    isStarred: false,
    hasInvite: false,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

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
function boot(entry: () => void, pinia = createPinia()) {
  const wrapper = mount(defineComponent({ setup: () => entry(), template: '<div />' }), {
    global: { plugins: [pinia, router] },
  });
  mounted.push(wrapper);
  return wrapper;
}

describe('bootstrap entries', () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? 1);
    vi.mocked(listen).mockImplementation(async () => () => {});
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

  it.each(['tap', 'satchel.read', 'satchel.archive', 'satchel.reply'].flatMap(actionId =>
    ['cold', 'warm'].map(lifecycle => ({ actionId, lifecycle })),
  ))(
    'delivers the $lifecycle iOS action $actionId once through the registered listener', async ({ actionId, lifecycle }) => {
      const pinia = createPinia();
      const ensure = vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
      const realActions = threadActionModule.useThreadActions;
      const read = vi.fn(async () => true);
      const archive = vi.fn(async () => true);
      vi.spyOn(threadActionModule, 'useThreadActions').mockImplementation(() => ({
        ...realActions(), setRead: read, archive,
      }));
      const open = vi.spyOn(useComposerStore(pinia), 'openWithOutcome');
      const push = vi.spyOn(router, 'push');
      const deliver = async () => {
        const registration = vi.mocked(listen).mock.calls.find(([event]) => event === 'mobile:notification-route');
        expect(registration).toBeDefined();
        await registration![1]({ payload: {
          listenerId: 1, type: 'notification-tap', threadId: 'ios-thread', actionId, inputValue: '  reply text  ', responseId: 'ios-response',
        } } as never);
      };
      vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id, ready) => {
        if (ready && lifecycle === 'cold') await deliver();
        return id ?? 1;
      });
      const wrapper = boot(useAppBootstrap, pinia);
      await flushPromises();
      await flushPromises();
      if (lifecycle === 'warm') {
        await deliver();
        await flushPromises();
      }
      // Pinned iOS plugin actionPerformed has no extra. Even if received, it
      // must not duplicate the native mail route/action.
      const pluginListener = vi.mocked(onAction).mock.calls.at(-1)![0];
      pluginListener({ actionId, inputValue: '  reply text  ', notification: {
        id: 1, title: 'Mail', body: 'Preview', actionTypeId: 'satchel.mail', sound: '',
      } } as never);
      await flushPromises();
      if (actionId === 'satchel.read') expect(read).toHaveBeenCalledExactlyOnceWith(['ios-thread'], true);
      else if (actionId === 'satchel.archive') expect(archive).toHaveBeenCalledExactlyOnceWith(['ios-thread']);
      else expect(router.currentRoute.value.params.id).toBe('ios-thread');
      const navigations = push.mock.calls.filter(([route]) => typeof route === 'object' && 'name' in route && route.name === 'thread');
      expect(navigations).toHaveLength(actionId === 'tap' || actionId === 'satchel.reply' ? 1 : 0);
      expect(read).toHaveBeenCalledTimes(actionId === 'satchel.read' ? 1 : 0);
      expect(archive).toHaveBeenCalledTimes(actionId === 'satchel.archive' ? 1 : 0);
      if (actionId === 'satchel.reply') {
        expect(open).toHaveBeenCalledExactlyOnceWith({ mode: 'reply', threadId: 'ios-thread', body: 'reply text' });
      } else expect(open).not.toHaveBeenCalled();
      expect(ensure).toHaveBeenCalledTimes(actionId === 'tap' ? 0 : 1);
      expect(emit).toHaveBeenCalledExactlyOnceWith('mobile:notification-response-handled', { responseId: 'ios-response' });
      wrapper.unmount();
    },
  );

  it('acknowledges an iOS background action only after its mutation settles', async () => {
    const pinia = createPinia();
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    let finish!: (result: boolean) => void;
    const read = vi.fn(() => new Promise<boolean>(resolve => { finish = resolve; }));
    const realActions = threadActionModule.useThreadActions;
    vi.spyOn(threadActionModule, 'useThreadActions').mockImplementation(() => ({ ...realActions(), setRead: read }));
    const wrapper = boot(useAppBootstrap, pinia);
    await flushPromises();
    const registration = vi.mocked(listen).mock.calls.find(([event]) => event === 'mobile:notification-route')!;
    const pending = registration[1]({ payload: {
      listenerId: 1, type: 'notification-tap', threadId: 'ios-thread', actionId: 'satchel.read', responseId: 'background-response',
    } } as never);
    await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', expect.anything());
    finish(true);
    await pending;
    expect(emit).toHaveBeenCalledExactlyOnceWith('mobile:notification-response-handled', { responseId: 'background-response' });
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

  it('starts no listeners or sync once the root unmounted during the core load', async () => {
    let release!: (value: never[]) => void;
    vi.mocked(backendListAccounts).mockReturnValueOnce(new Promise(resolve => { release = resolve; }));
    const wrapper = boot(useAppBootstrap);
    await vi.waitFor(() => expect(backendListAccounts).toHaveBeenCalled());
    await flushPromises();

    wrapper.unmount();
    release([]);
    await flushPromises();
    await flushPromises();

    const events = vi.mocked(listen).mock.calls.map(([event]) => event);
    expect(events).not.toContain('mail:incoming');
    expect(backendRunImapSync).not.toHaveBeenCalled();
    expect(backendSyncGroupware).not.toHaveBeenCalled();
  });

  it('re-arms no sync timer when a sync settles after unmount', async () => {
    let finishSync!: () => void;
    vi.mocked(backendSyncGroupware).mockReturnValueOnce(new Promise(resolve => { finishSync = () => resolve({} as never); }));
    let finishImap!: () => void;
    vi.mocked(backendRunImapSync).mockReturnValueOnce(new Promise(resolve => {
      finishImap = () => resolve({
        accounts_synced: 0,
        accounts_failed: 0,
        folders_synced: 0,
        messages_synced: 0,
        backfill_pending_accounts: 0,
        backfill_remaining_messages: 0,
        backfill_total_messages: 0,
        failures: [],
      } as never);
    }));
    const wrapper = boot(useAppBootstrap);
    await vi.waitFor(() => expect(backendRunImapSync).toHaveBeenCalled());
    expect(backendSyncGroupware).toHaveBeenCalled();

    wrapper.unmount();
    const setTimeoutSpy = vi.spyOn(window, 'setTimeout');
    finishSync();
    finishImap();
    await flushPromises();
    await flushPromises();

    expect(setTimeoutSpy.mock.calls.filter(([, delay]) => (delay ?? 0) >= 5000)).toEqual([]);
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
    const actions = vi.mocked(registerActionTypes).mock.calls[0]![0][0]!.actions;
    expect(actions.find(action => action.id === 'satchel.reply')).toMatchObject({ foreground: true });
    expect(actions.find(action => action.id === 'satchel.read')?.foreground).not.toBe(true);
    expect(actions.find(action => action.id === 'satchel.archive')?.foreground).not.toBe(true);
    wrapper.unmount();
  });

  it.each([
    { outcome: 'replaced', acknowledged: true },
    { outcome: 'kept', acknowledged: true },
    { outcome: 'unsaved', acknowledged: false },
  ] as const)('waits for the reply replacement decision before ACK, outcome=$outcome', async ({ outcome, acknowledged }) => {
    const pinia = createPinia();
    const composer = useComposerStore(pinia);
    await composer.open({ mode: 'new', body: 'existing draft' });
    let release!: (value: ComposerReplaceOutcome) => void;
    const guard = vi.fn(() => new Promise<ComposerReplaceOutcome>(resolve => { release = resolve; }));
    const disposeGuard = composer.setReplaceGuard(guard);
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    const wrapper = boot(useAppBootstrap, pinia);
    await flushPromises();
    await flushPromises();
    const registration = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')!;
    const handled = registration[1]({ payload: {
      listenerId: 1, type: 'notification-tap', threadId: 'reply-thread', actionId: 'satchel.reply',
      inputValue: 'new reply', responseId: 'reply-response',
    } } as never);
    await vi.waitFor(() => expect(guard).toHaveBeenCalledOnce());
    try {
      expect(composer.request?.body).toBe('existing draft');
      expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', { responseId: 'reply-response' });
      expect(backendDismissMobileNotifications).not.toHaveBeenCalled();
      release(outcome);
      await handled;
      // A refusal is the user's answer; a failed save keeps the reply for a retry.
      expect(vi.mocked(emit).mock.calls).toEqual(acknowledged
        ? [['mobile:notification-response-handled', { responseId: 'reply-response' }]]
        : []);
      expect(composer.request?.body).toBe(outcome === 'replaced' ? 'new reply' : 'existing draft');
      expect(backendDismissMobileNotifications).toHaveBeenCalledTimes(outcome === 'replaced' ? 1 : 0);
    } finally {
      release(outcome);
      await handled;
      disposeGuard();
      wrapper.unmount();
    }
  });

  it.each([true, false])('joins a reply guard across recreation and honors cancellation, accepted=%s', async (accepted) => {
    const pinia = createPinia();
    const composer = useComposerStore(pinia);
    await composer.open({ mode: 'new', body: 'keep this draft' });
    let release!: (value: ComposerReplaceOutcome) => void;
    const guard = vi.fn(() => new Promise<ComposerReplaceOutcome>(resolve => { release = resolve; }));
    composer.setReplaceGuard(guard);
    const open = vi.spyOn(composer, 'openWithOutcome');
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    let generation = 0;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? ++generation);
    const first = boot(useAppBootstrap, pinia);
    await flushPromises();
    const oldCallback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const payload = { listenerId: 1, type: 'notification-tap', threadId: 'guard-thread',
      actionId: 'satchel.reply', inputValue: 'retained reply', responseId: 'guard-response' };
    const oldWork = oldCallback({ payload } as never);
    await vi.waitFor(() => expect(guard).toHaveBeenCalledOnce());
    first.unmount();
    const second = boot(useAppBootstrap, pinia);
    await flushPromises();
    const newCallback = vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route').at(-1)![1];
    // Stale callbacks cannot start work after disposal. A new delivery joins it.
    await oldCallback({ payload } as never);
    const replay = newCallback({ payload: { ...payload, listenerId: 2 } } as never);
    await flushPromises();
    expect(open).toHaveBeenCalledOnce();
    expect(guard).toHaveBeenCalledOnce();
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', expect.anything());
    expect(backendDismissMobileNotifications).not.toHaveBeenCalled();
    release(accepted ? 'replaced' : 'kept');
    await Promise.all([oldWork, replay]);
    expect(composer.request?.body).toBe(accepted ? 'retained reply' : 'keep this draft');
    expect(backendDismissMobileNotifications).toHaveBeenCalledTimes(accepted ? 1 : 0);
    await newCallback({ payload: { ...payload, listenerId: 2 } } as never);
    expect(open).toHaveBeenCalledOnce();
    expect(emit).toHaveBeenCalledWith('mobile:notification-response-handled', { responseId: 'guard-response' });
    expect(backendMobileNotificationsFrontendReady).toHaveBeenCalledWith(1, false);
    second.unmount();
    await flushPromises();
    expect(backendMobileNotificationsFrontendReady).toHaveBeenCalledWith(2, false);
  });

  it.each(['satchel.read', 'satchel.archive'])('joins an in-flight %s mutation across recreation', async (actionId) => {
    const pinia = createPinia();
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    let finish!: () => void;
    const mutation = vi.fn(() => new Promise<boolean>(resolve => { finish = () => resolve(true); }));
    const realActions = threadActionModule.useThreadActions;
    vi.spyOn(threadActionModule, 'useThreadActions').mockImplementation(() => ({
      ...realActions(), setRead: mutation, archive: mutation,
    }));
    let generation = 0;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? ++generation);
    const first = boot(useAppBootstrap, pinia);
    await flushPromises();
    const oldCallback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const payload = { listenerId: 1, type: 'notification-tap', threadId: 'pending-thread', actionId, responseId: 'pending-response' };
    const pending = oldCallback({ payload } as never);
    await vi.waitFor(() => expect(mutation).toHaveBeenCalledOnce());
    first.unmount();
    const second = boot(useAppBootstrap, pinia);
    await flushPromises();
    const newCallback = vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route').at(-1)![1];
    const replay = newCallback({ payload: { ...payload, listenerId: 2 } } as never);
    await flushPromises();
    expect(mutation).toHaveBeenCalledOnce();
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', expect.anything());
    finish();
    await Promise.all([pending, replay]);
    expect(mutation).toHaveBeenCalledOnce();
    expect(emit).toHaveBeenCalledWith('mobile:notification-response-handled', { responseId: 'pending-response' });
    second.unmount();
  });

  it.each(['satchel.read', 'satchel.archive', 'satchel.reply'])('re-ACKs completed work after event loss without repeating %s', async (actionId) => {
    const pinia = createPinia();
    const composer = useComposerStore(pinia);
    const open = vi.spyOn(composer, 'openWithOutcome');
    const realActions = threadActionModule.useThreadActions;
    const read = vi.fn(async () => true);
    const archive = vi.fn(async () => true);
    vi.spyOn(threadActionModule, 'useThreadActions').mockImplementation(() => ({ ...realActions(), setRead: read, archive }));
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    let generation = 0;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? ++generation);
    const first = boot(useAppBootstrap, pinia);
    await flushPromises();
    const callback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const payload = { listenerId: 1, type: 'notification-tap', threadId: 'lost-thread', actionId,
      inputValue: 'reply survives ACK loss', responseId: 'lost-ack' };
    vi.mocked(emit).mockRejectedValueOnce(new Error('ACK delivery lost'));
    await callback({ payload } as never);
    first.unmount();
    // A response received during disposal is not processed by the old callback.
    const later = { ...payload, responseId: 'while-disposed', actionId: 'tap' };
    await callback({ payload: later } as never);
    const second = boot(useAppBootstrap, pinia);
    await flushPromises();
    const newCallback = vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route').at(-1)![1];
    // Old-lease events arriving at a new callback are ignored as well.
    await newCallback({ payload } as never);
    expect(emit).toHaveBeenCalledTimes(1);
    await newCallback({ payload: { ...payload, listenerId: 2 } } as never);
    await newCallback({ payload: { ...later, listenerId: 2 } } as never);
    expect(read).toHaveBeenCalledTimes(actionId === 'satchel.read' ? 1 : 0);
    expect(archive).toHaveBeenCalledTimes(actionId === 'satchel.archive' ? 1 : 0);
    expect(open).toHaveBeenCalledTimes(actionId === 'satchel.reply' ? 1 : 0);
    if (actionId === 'satchel.reply') expect(composer.request?.body).toBe('reply survives ACK loss');
    expect(emit).toHaveBeenCalledTimes(3);
    expect(router.currentRoute.value.params.id).toBe('lost-thread');
    second.unmount();
  });

  it('retains failed reply work for replay and keeps opened text through later dismissal errors', async () => {
    const pinia = createPinia();
    const composer = useComposerStore(pinia);
    await composer.open({ mode: 'new', body: 'original' });
    const guard = vi.fn().mockRejectedValueOnce(new Error('draft save failed')).mockResolvedValue('replaced');
    composer.setReplaceGuard(guard);
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    let generation = 0;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? ++generation);
    const first = boot(useAppBootstrap, pinia);
    await flushPromises();
    const callback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const payload = { listenerId: 1, type: 'notification-tap', threadId: 'retry-thread', actionId: 'satchel.reply',
      inputValue: 'do not lose this text', responseId: 'failed-reply' };
    await callback({ payload } as never);
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', expect.anything());
    expect(composer.request?.body).toBe('original');
    expect(backendDismissMobileNotifications).not.toHaveBeenCalled();
    first.unmount();
    const second = boot(useAppBootstrap, pinia);
    await flushPromises();
    const newCallback = vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route').at(-1)![1];
    vi.mocked(backendDismissMobileNotifications).mockRejectedValueOnce(new Error('dismiss failed'));
    await newCallback({ payload: { ...payload, listenerId: 2 } } as never);
    expect(guard).toHaveBeenCalledTimes(2);
    expect(composer.request).toMatchObject({ mode: 'reply', body: 'do not lose this text' });
    expect(emit).toHaveBeenCalledWith('mobile:notification-response-handled', { responseId: 'failed-reply' });
    second.unmount();
  });

  it('keeps a reply unacknowledged while the open draft cannot be saved, and delivers it on replay', async () => {
    const pinia = createPinia();
    const composer = useComposerStore(pinia);
    vi.spyOn(useThreadsStore(pinia), 'ensureThreadLoaded').mockResolvedValue(undefined as never);
    composeForm.shouldConfirmClose.mockReturnValue(true);
    composeForm.persistDraft.mockResolvedValueOnce(null).mockResolvedValue('saved-draft');
    let generation = 0;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? ++generation);
    const first = boot(useAppBootstrap, pinia);
    mounted.push(mount(ComposerHost, { global: { plugins: [pinia, router] } }));
    await composer.open({ mode: 'new', body: 'unsaved original' });
    await flushPromises();
    const callback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const payload = { listenerId: 1, type: 'notification-tap', threadId: 'unsaved-thread', actionId: 'satchel.reply',
      inputValue: 'reply typed into the notification', responseId: 'unsaved-reply' };
    await callback({ payload } as never);
    expect(composeForm.persistDraft).toHaveBeenCalledOnce();
    expect(composer.request?.body).toBe('unsaved original');
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', { responseId: 'unsaved-reply' });
    expect(backendDismissMobileNotifications).not.toHaveBeenCalled();
    first.unmount();
    const second = boot(useAppBootstrap, pinia);
    await flushPromises();
    const replay = vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route').at(-1)![1];
    await replay({ payload: { ...payload, listenerId: 2 } } as never);
    expect(composeForm.persistDraft).toHaveBeenCalledTimes(2);
    expect(composer.request).toMatchObject({ mode: 'reply', body: 'reply typed into the notification' });
    expect(emit).toHaveBeenCalledExactlyOnceWith('mobile:notification-response-handled', { responseId: 'unsaved-reply' });
    second.unmount();
  });

  it.each(['satchel.read', 'satchel.archive'])('keeps a failed %s unacknowledged and retries it on replay', async (actionId) => {
    const pinia = createPinia();
    const threads = useThreadsStore(pinia);
    // The startup load replaces the list, so the row arrives the way a cold one does.
    vi.spyOn(threads, 'ensureThreadLoaded').mockImplementation(async (id) => {
      if (!threads.find(id)) threads.threads = [...threads.threads, inboxThread(id)];
      return undefined as never;
    });
    const read = vi.spyOn(threads, 'setThreadsRead')
      .mockRejectedValueOnce(new Error('database is locked'))
      .mockResolvedValue(undefined);
    const write = actionId === 'satchel.read' ? read : vi.mocked(backendTransitionThreads);
    if (write !== read) write.mockRejectedValueOnce(new Error('database is locked'));
    let generation = 0;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? ++generation);
    const first = boot(useAppBootstrap, pinia);
    await flushPromises();
    const callback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const payload = { listenerId: 1, type: 'notification-tap', threadId: 'failing-thread', actionId, responseId: 'failed-action' };
    await callback({ payload } as never);
    expect(write).toHaveBeenCalledOnce();
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', { responseId: 'failed-action' });
    first.unmount();
    const second = boot(useAppBootstrap, pinia);
    await flushPromises();
    const replay = vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route').at(-1)![1];
    await replay({ payload: { ...payload, listenerId: 2 } } as never);
    expect(write).toHaveBeenCalledTimes(2);
    expect(emit).toHaveBeenCalledExactlyOnceWith('mobile:notification-response-handled', { responseId: 'failed-action' });
    second.unmount();
  });

  it('ignores malformed routes without ACK or navigation', async () => {
    const wrapper = boot(useAppBootstrap);
    await flushPromises();
    const callback = vi.mocked(listen).mock.calls.find(([name]) => name === 'mobile:notification-route')![1];
    const valid = { listenerId: 1, type: 'notification-tap', threadId: 'thread', actionId: 'tap', responseId: 'response' };
    for (const payload of [null, {}, { ...valid, responseId: null }, { ...valid, responseId: '' }, { ...valid, responseId: ' ' },
      { ...valid, responseId: 'x'.repeat(129) }, { ...valid, threadId: 42 }, { ...valid, threadId: ' ' },
      { ...valid, type: 'other' }, { ...valid, actionId: 'unknown' }, { ...valid, actionId: 42 }, { ...valid, listenerId: 999 }]) {
      await callback({ payload } as never);
    }
    expect(router.currentRoute.value.name).toBe('inbox');
    expect(emit).not.toHaveBeenCalledWith('mobile:notification-response-handled', expect.anything());
    wrapper.unmount();
  });

  it.each(['reservation', 'activation', 'retirement'])(
    'reports %s failures until a new listener activates without clearing another owner', async (phase) => {
      const pinia = createPinia();
      const connection = useConnectionStore(pinia);
      const failure = new Error(JSON.stringify({ key: 'backend.errors.unexpected' }));
      let nextId = 0;
      let failed = false;
      vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id, ready) => {
        const operation = id === null ? 'reservation' : ready ? 'activation' : 'retirement';
        if (operation === phase && !failed) {
          failed = true;
          throw failure;
        }
        return id ?? ++nextId;
      });
      const first = boot(useAppBootstrap, pinia);
      await flushPromises();
      if (phase === 'retirement') first.unmount();
      await vi.waitFor(() => expect(connection.backendErrorFor('notifications')).toBe(failure.message));
      // A cleanup success must not hide a failed activation.
      await flushPromises();
      expect(connection.backendErrorFor('notifications')).toBe(failure.message);
      connection.reportBackendError('another-owner', new Error('independent failure'));
      first.unmount();
      const second = boot(useAppBootstrap, pinia);
      await flushPromises();
      await vi.waitFor(() => expect(connection.backendErrorFor('notifications')).toBeNull());
      expect(connection.backendErrorFor('another-owner')).toBe('independent failure');
      second.unmount();
    },
  );

  it('retires a reservation that resolves after disposal without disturbing a new listener', async () => {
    let reserve!: (id: number) => void;
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementation(async (id) => id ?? 2);
    vi.mocked(backendMobileNotificationsFrontendReady).mockImplementationOnce(() => new Promise(resolve => { reserve = resolve; }));
    const first = boot(useAppBootstrap);
    await vi.waitFor(() => expect(reserve).toBeDefined());
    first.unmount();
    const second = boot(useAppBootstrap);
    await flushPromises();
    reserve(1);
    await flushPromises();
    await flushPromises();
    expect(backendMobileNotificationsFrontendReady).toHaveBeenCalledWith(1, false);
    expect(backendMobileNotificationsFrontendReady).not.toHaveBeenCalledWith(1, true);
    expect(backendMobileNotificationsFrontendReady).toHaveBeenCalledWith(2, true);
    expect(vi.mocked(listen).mock.calls.filter(([name]) => name === 'mobile:notification-route')).toHaveLength(1);
    second.unmount();
  });

  it('removes a late route registration and never activates its retired lease', async () => {
    let registered!: (dispose: () => void) => void;
    const dispose = vi.fn();
    vi.mocked(listen).mockImplementation(async (name) => {
      if (name === 'mobile:notification-route') return new Promise(resolve => { registered = resolve; });
      return () => {};
    });
    const wrapper = boot(useAppBootstrap);
    await vi.waitFor(() => expect(registered).toBeDefined());
    wrapper.unmount();
    registered(dispose);
    await flushPromises();
    expect(dispose).toHaveBeenCalledOnce();
    expect(backendMobileNotificationsFrontendReady).toHaveBeenCalledWith(1, false);
    expect(backendMobileNotificationsFrontendReady).not.toHaveBeenCalledWith(1, true);
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
