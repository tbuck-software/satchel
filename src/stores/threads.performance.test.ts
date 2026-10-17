// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { computed, defineComponent, h, nextTick } from 'vue';
import { shallowMount } from '@vue/test-utils';
import { expect, it, vi } from 'vitest';
import { backendListThreads } from '@/services/backend';
import { useThreadsStore } from './threads';
import { toThreadModel } from '@/utils/threadList';
import ThreadGroupList from '@/components/mail/ThreadGroupList.vue';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: () => true,
  backendListThreads: vi.fn(),
  backendListCachedThreads: vi.fn(async () => null),
  backendListSmartInboxSummary: vi.fn(async () => ({ total_count: 2000, buckets: [] })),
  backendGetThreads: vi.fn(async (ids: string[]) => rows.filter(row => ids.includes(row.id))),
}));
const rows = Array.from({ length: 2000 }, (_, i) => ({
  id: `t${i}`, account_id: 'acc', subject: `Mail ${i}`, normalized_subject: `Mail ${i}`, snippet: 'Preview',
  participants: '[{"name":"Sender","email":"sender@example.com"}]',
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
