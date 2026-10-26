// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { computed, defineComponent, effectScope, h, nextTick, watchEffect } from 'vue';
import { shallowMount } from '@vue/test-utils';
import { expect, it, vi } from 'vitest';
import { backendListThreads } from '@/services/backend';
import { useThreadsStore } from './threads';
import { toThreadModel } from '@/utils/threadList';
import ThreadGroupList from '@/components/mail/ThreadGroupList.vue';

const { projections } = vi.hoisted(() => ({
  projections: { newSenders: 0, fullContacts: 0, topContacts: 0 },
}));

vi.mock('@/utils/threadList', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/utils/threadList')>();
  return {
    ...actual,
    collectNewSenderThreadIds: (...args: Parameters<typeof actual.collectNewSenderThreadIds>) => {
      projections.newSenders += 1;
      return actual.collectNewSenderThreadIds(...args);
    },
    buildContactSummaries: (...args: Parameters<typeof actual.buildContactSummaries>) => {
      projections.fullContacts += 1;
      return actual.buildContactSummaries(...args);
    },
    buildTopContactSummaries: (...args: Parameters<typeof actual.buildTopContactSummaries>) => {
      projections.topContacts += 1;
      return actual.buildTopContactSummaries(...args);
    },
  };
});

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: () => true,
  backendListThreads: vi.fn(),
  backendListCachedThreads: vi.fn(async () => null),
  backendListSmartInboxSummary: vi.fn(async () => ({ total_count: 2000, buckets: [] })),
  backendGetThreads: vi.fn(async (ids: string[]) => rows.filter(row => ids.includes(row.id))),
}));
const rows = Array.from({ length: 2000 }, (_, i) => ({
  id: `t${i}`, account_id: 'acc', subject: `Mail ${i}`, normalized_subject: `Mail ${i}`, snippet: 'Preview',
  participants: `[{"name":"Sender ${i % 300}","email":"sender${i % 300}@example.com"}]`,
  sent_participants: i % 3 === 0 ? `[{"name":"Sender ${i % 300}","email":"sender${i % 300}@example.com"}]` : undefined,
  last_message_date: new Date(Date.UTC(2026, 8, 1) - i * 60000).toISOString(),
  message_count: 1, unread_count: 1, has_attachments: false,
  workflow_state: 'inbox' as const, priority: 1, mailbox_names: '["INBOX"]', tags: [],
}));

it('measures a no-change refresh over 2,000 loaded threads', async () => {
  const height = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(900);
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useThreadsStore();
  store.threads = rows.map(row => toThreadModel(row, []));
  vi.mocked(backendListThreads).mockImplementation(async () => ({
    threads: structuredClone(rows.slice(0, 40)), next_cursor: null, has_more: true,
  }));
  let renders = 0;
  const Row = defineComponent({
    props: ['thread', 'active', 'selected', 'keyboardFocused', 'selectionMode', 'selectedIds', 'posInSet', 'setSize'],
    setup(props) { return () => { renders++; return h('div', props.thread.subject); }; },
  });
  const Host = defineComponent({
    setup() {
      const items = computed(() => store.visibleThreads.map(thread => ({ kind: 'thread' as const, id: thread.id, thread })));
      return () => h(ThreadGroupList, { items: items.value, selectedThreadIds: store.selectedThreadIds,
        activeThreadId: store.activeThreadId, senderReviewSetupComplete: true, bulkBusy: false });
    },
  });
  const wrapper = shallowMount(Host, { global: { plugins: [pinia], stubs: { ThreadGroupList: false, ThreadListItem: Row } } });
  await nextTick();
  const visibleRows = wrapper.findAllComponents(Row).length;
  const times: number[] = [];
  const counts: number[] = [];
  for (let run = 0; run < 25; run++) {
    renders = 0;
    const start = performance.now();
    await store.refreshHead();
    await nextTick();
    times.push(performance.now() - start);
    counts.push(renders);
  }
  times.sort((a, b) => a - b);
  renders = 0;
  store.selectedThreadIds = ['t0'];
  await nextTick();
  renders = 0;
  store.selectedThreadIds = ['t0', 't1'];
  await nextTick();
  console.log(JSON.stringify({ threads: 2000, visibleRows, refreshMedianMs: times[12], refreshP95Ms: times[23], rowRenders: counts, selectionExtensionRenders: renders }));
  expect(visibleRows).toBeGreaterThan(0);
  if (!process.env.SATCHEL_PERF_REPORT_ONLY) {
    expect(counts).toEqual(Array(25).fill(0));
    expect(renders).toBe(1);
  }
  wrapper.unmount();
  height.mockRestore();
});

it('patches one row of 2,000 without re-running the per-thread projections twice', async () => {
  const height = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(900);
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useThreadsStore();
  store.threads = rows.map(row => toThreadModel(row, []));
  let renders = 0;
  const Row = defineComponent({
    props: ['thread', 'active', 'selected', 'keyboardFocused', 'selectionMode', 'selectedIds', 'posInSet', 'setSize'],
    setup(props) { return () => { renders++; return h('div', props.thread.subject); }; },
  });
  // The whole smart-inbox pipeline, as the Inbox renders it.
  const Host = defineComponent({
    setup() {
      return () => h(ThreadGroupList, { items: store.smartListEntries, selectedThreadIds: store.selectedThreadIds,
        activeThreadId: store.activeThreadId, senderReviewSetupComplete: true, bulkBusy: false });
    },
  });
  const wrapper = shallowMount(Host, { global: { plugins: [pinia], stubs: { ThreadGroupList: false, ThreadListItem: Row } } });
  // What the always-mounted sidebar and the Inbox header read.
  const scope = effectScope();
  scope.run(() => watchEffect(() => {
    void store.frequentContactSummaries.length;
    void store.pinnedContactSummaries.size;
    void store.activeContact;
  }));
  await nextTick();
  const visible = wrapper.findAllComponents(Row).map(row => row.props('thread').id as string);
  expect(visible.length).toBeGreaterThan(0);

  const times: number[] = [];
  const counts: Array<typeof projections & { renders: number }> = [];
  for (let run = 0; run < 15; run++) {
    const target = visible[run % visible.length]!;
    renders = 0;
    projections.newSenders = 0;
    projections.fullContacts = 0;
    projections.topContacts = 0;
    const start = performance.now();
    store.markRead(target, store.find(target)!.unreadCount > 0);
    await nextTick();
    times.push(performance.now() - start);
    counts.push({ ...projections, renders });
  }
  times.sort((a, b) => a - b);
  console.log(JSON.stringify({ threads: 2000, singleRowPatchMedianMs: times[7], singleRowPatchP95Ms: times[14], perPatch: counts[0] }));
  if (!process.env.SATCHEL_PERF_REPORT_ONLY) {
    for (const count of counts) {
      // One new-sender split shared by the review stack and the buckets.
      expect(count.newSenders).toBe(1);
      // The sidebar's five contacts, not the full sorted projection.
      expect(count.fullContacts).toBe(0);
      expect(count.topContacts).toBe(1);
      // Only the patched row re-renders.
      expect(count.renders).toBeLessThanOrEqual(1);
    }
  }
  scope.stop();
  wrapper.unmount();
  height.mockRestore();
}, 30_000);
