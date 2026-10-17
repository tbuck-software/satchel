// @vitest-environment jsdom

import { defineComponent } from 'vue';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAppBootstrap } from '@/bootstrap';
import { listen } from '@tauri-apps/api/event';
import { backendRunImapSync } from '@/services/backend';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useMessagesStore } from '@/stores/messages';
import { useThreadsStore } from '@/stores/threads';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
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
    backendTakePendingNotificationThread: vi.fn(async () => null),
    backendTakePendingOpenWith: resolving([]),
    backendSyncGroupware: resolving({}),
  };
});

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/inbox', name: 'inbox', component: { template: '<div />' } },
    { path: '/compose', name: 'compose', component: { template: '<div />' } },
  ],
});

/*
 * PERF-07. Every sync event pulled the head page, the mailbox list and the
 * open conversation again. The IDLE supervisor fires `mail:incoming` in quick
 * succession during a first sync, so the burst paid for the full list and
 * contact projections once per event.
 */

function mailIncomingHandler(): () => void {
  const registration = vi
    .mocked(listen)
    .mock.calls.find(([event]) => event === 'mail:incoming');
  expect(registration).toBeDefined();
  return registration![1] as () => void;
}

describe('mail event coalescing', () => {
  let pinia: ReturnType<typeof createPinia>;

  beforeEach(async () => {
    vi.clearAllMocks();
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    pinia = createPinia();
    setActivePinia(pinia);
    await router.push('/inbox');
    await router.isReady();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  async function bootApp() {
    const wrapper = mount(
      defineComponent({ setup: () => useAppBootstrap(), template: '<div />' }),
      { global: { plugins: [pinia, router] } },
    );
    await flushPromises();
    await flushPromises();

    await vi.waitFor(() => expect(vi.mocked(listen).mock.calls.some(([event]) => event === 'ics:open-with')).toBe(true));
    const threads = useThreadsStore();
    const mailboxes = useMailboxesStore();
    const messages = useMessagesStore();
    threads.activeThreadId = 'th-open';
    const refreshHead = vi.spyOn(threads, 'refreshHead').mockResolvedValue(undefined);
    const loadMailboxes = vi.spyOn(mailboxes, 'load').mockResolvedValue(undefined);
    const refreshThread = vi.spyOn(messages, 'refreshThread').mockResolvedValue(undefined);
    return { wrapper, refreshHead, loadMailboxes, refreshThread };
  }

  it('uses a head refresh on mobile and skips unchanged auto-sync results', async () => {
    vi.useFakeTimers();
    document.documentElement.dataset.runtimeMobile = 'true';
    const { wrapper, refreshHead, loadMailboxes } = await bootApp();
    const load = vi.spyOn(useThreadsStore(), 'load').mockResolvedValue(undefined);
    const sync = vi.mocked(backendRunImapSync);
    sync.mockResolvedValueOnce({ accounts_synced: 1, accounts_failed: 0, folders_synced: 1,
      messages_synced: 1, backfill_pending_accounts: 0, backfill_remaining_messages: 0,
      backfill_total_messages: 0, failures: [] });
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    expect(refreshHead).toHaveBeenCalledTimes(1);
    expect(load).not.toHaveBeenCalled();
    expect(loadMailboxes).toHaveBeenCalledTimes(1);
    refreshHead.mockClear();
    loadMailboxes.mockClear();
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    expect(refreshHead).not.toHaveBeenCalled();
    expect(loadMailboxes).not.toHaveBeenCalled();
    wrapper.unmount();
    delete document.documentElement.dataset.runtimeMobile;
  });

  it('coalesces a burst of thread IDs into one row refresh', async () => {
    const { wrapper } = await bootApp();
    const refreshThreads = vi.spyOn(useThreadsStore(), 'refreshThreads').mockResolvedValue(undefined);
    const onUpdated = vi.mocked(listen).mock.calls.find(([event]) => event === 'thread:updated')![1] as (event: unknown) => void;
    vi.useFakeTimers();
    for (let id = 0; id < 300; id++) onUpdated({ payload: { thread_id: `t${id}` } });
    onUpdated({ payload: { thread_ids: ['t0', 't1'] } });
    expect(refreshThreads).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(100);
    expect(refreshThreads).toHaveBeenCalledTimes(1);
    expect(refreshThreads.mock.calls[0]![0]).toHaveLength(300);
    wrapper.unmount();
  });

  it('resynchronizes when the backend bridge reports missed events', async () => {
    const { wrapper, refreshHead } = await bootApp();
    const onResync = vi.mocked(listen).mock.calls.find(([event]) => event === 'mail:resync')![1] as () => void;
    vi.useFakeTimers();
    onResync();
    await vi.advanceTimersByTimeAsync(100);
    expect(refreshHead).toHaveBeenCalled();
    wrapper.unmount();
  });

  it('folds ten events in one window into two refreshes', async () => {
    const { wrapper, refreshHead, loadMailboxes, refreshThread } = await bootApp();
    const onMailIncoming = mailIncomingHandler();
    vi.useFakeTimers();

    for (let event = 0; event < 10; event += 1) {
      onMailIncoming();
      await vi.advanceTimersByTimeAsync(25);
    }
    await vi.advanceTimersByTimeAsync(400);

    // One immediately, one trailing. Was ten.
    expect(refreshHead).toHaveBeenCalledTimes(2);
    expect(loadMailboxes).toHaveBeenCalledTimes(2);
    // The open thread reloads previews only; bodies and images stay.
    expect(refreshThread).toHaveBeenCalledTimes(2);
    expect(refreshThread).toHaveBeenCalledWith('th-open');

    wrapper.unmount();
  });

  it('runs a single event immediately and only once', async () => {
    const { wrapper, refreshHead } = await bootApp();
    const onMailIncoming = mailIncomingHandler();
    vi.useFakeTimers();

    const before = performance.now();
    onMailIncoming();
    // No leading debounce: the first event of a burst is never delayed.
    expect(refreshHead).toHaveBeenCalledTimes(1);
    expect(performance.now() - before).toBeLessThan(50);

    await vi.advanceTimersByTimeAsync(1000);
    expect(refreshHead).toHaveBeenCalledTimes(1);

    wrapper.unmount();
  });

  it('does not lose an event that arrives while a refresh is running', async () => {
    const { wrapper, refreshHead } = await bootApp();
    let release: () => void = () => {};
    refreshHead.mockImplementation(() => new Promise<void>((resolve) => {
      release = resolve;
    }));
    const onMailIncoming = mailIncomingHandler();
    vi.useFakeTimers();

    onMailIncoming();
    expect(refreshHead).toHaveBeenCalledTimes(1);

    // Past the coalescing window, so only the in-flight run can absorb it.
    await vi.advanceTimersByTimeAsync(400);
    onMailIncoming();
    expect(refreshHead).toHaveBeenCalledTimes(1);

    release();
    await vi.advanceTimersByTimeAsync(400);
    expect(refreshHead).toHaveBeenCalledTimes(2);

    wrapper.unmount();
  });
});
