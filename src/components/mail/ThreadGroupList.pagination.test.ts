// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { computed, nextTick, defineComponent, h } from 'vue';
import { describe, expect, it, vi } from 'vitest';
import { backendListThreads } from '@/services/backend';
import { useThreadsStore } from '@/stores/threads';
import ThreadGroupList from './ThreadGroupList.vue';

vi.mock('@/services/backend', async (original) => ({
  ...await original<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  backendListThreads: vi.fn(),
  backendListCachedThreads: vi.fn(async () => null),
  backendListSmartInboxSummary: vi.fn(async () => null),
}));

const all = Array.from({ length: 3200 }, (_, index) => ({
  id: `th-${String(index).padStart(5, '0')}`, account_id: 'acc-work', subject: `Subject ${index}`, normalized_subject: `Subject ${index}`,
  snippet: '', last_message_date: new Date(Date.UTC(2026, 0, 1) - index * 60_000).toISOString(),
  message_count: 1, workflow_state: 'inbox' as const, unread_count: 0, has_attachments: false,
  priority: 1, tags: [], participants: '[]', mailbox_names: '["INBOX"]',
}));

describe('store and virtual list pagination', () => {
  it('preserves the viewport across a full window and returns to trimmed rows with a pinned thread', async () => {
    vi.stubGlobal('requestIdleCallback', vi.fn(() => 1));
    vi.stubGlobal('cancelIdleCallback', vi.fn());
    setActivePinia(createPinia());
    let first = true;
    const requests = vi.mocked(backendListThreads).mockImplementation(async input => {
      const cursor = input?.before_cursor ?? input?.cursor;
      const anchor = cursor ? all.findIndex(row => row.id === cursor.id) : -1;
      const backward = !!input?.before_cursor;
      const size = first ? 2000 : 40;
      first = false;
      const start = backward ? Math.max(0, anchor - size) : anchor + 1;
      const end = backward ? anchor : start + size;
      const threads = all.slice(start, end);
      const last = threads.at(-1)!;
      return { threads, has_more: backward ? start > 0 : end < all.length,
        next_cursor: { id: last.id, last_message_date: last.last_message_date } };
    });
    const store = useThreadsStore();
    await store.load();
    store.setSelection(['th-00002']);
    const items = computed(() => store.threads.map(thread => ({ kind: 'thread' as const, id: thread.id, thread })));
    const host = defineComponent(() => () => h(ThreadGroupList, {
      items: items.value, selectedThreadIds: store.selectedThreadIds,
      hasPreviousThreads: store.hasPreviousThreads, paginationBusy: store.loadingMoreThreads,
      senderReviewSetupComplete: true, bulkBusy: false,
      onViewportStart: store.setViewportStart,
      onLoadMore: () => { void store.loadMore(); },
      onLoadPrevious: () => { void store.loadPrevious(); },
    }));
    const viewportHeight = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(600);
    const contentHeight = vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(2000 * 72);
    const wrapper = shallowMount(host, { global: { stubs: { ThreadGroupList: false } } });
    const list = wrapper.getComponent(ThreadGroupList);
    const el = list.get('.thread-list-scroller').element as HTMLElement;
    const height = Number.parseFloat(el.style.getPropertyValue('--thread-row-h'));
    Object.defineProperties(el, {
      clientHeight: { value: 600 }, scrollHeight: { get: () => store.threads.length * height },
    });
    await nextTick();
    async function scroll(top: number) {
      el.scrollTop = top;
      await list.get('.thread-list-scroller').trigger('scroll');
      await new Promise(resolve => window.requestAnimationFrame(resolve));
      await vi.waitFor(() => expect(store.loadingMoreThreads).toBe(false));
      await nextTick();
    }
    await scroll(1975 * height + 7);
    expect(store.threads).toHaveLength(2000);
    expect(el.scrollTop).toBe(1935 * height + 7);
    expect(requests).toHaveBeenCalledTimes(2);
    expect(store.find('th-00001')).toBeUndefined();
    expect(store.find('th-00002')).toBeDefined();
    // Keep moving the window while its first row remains pinned at the head.
    for (let page = 0; page < 9; page += 1) await scroll(1975 * height + 7);
    expect(requests).toHaveBeenCalledTimes(11);
    await scroll(0);
    expect(el.scrollTop).toBe(0);
    expect(requests).toHaveBeenCalledTimes(12);
    expect(store.find('th-00001')).toBeUndefined();
    // At scrollTop zero the user cannot produce another upward scroll event.
    // Explicit page requests remain available until the actual head is loaded.
    expect(list.find('[data-load-previous]').exists()).toBe(true);
    let pages = 0;
    while (store.hasPreviousThreads) {
      expect(pages++).toBeLessThan(12);
      const calls = requests.mock.calls.length;
      await list.get('[data-load-previous]').trigger('click');
      await vi.waitFor(() => expect(store.loadingMoreThreads).toBe(false));
      await nextTick();
      expect(requests).toHaveBeenCalledTimes(calls + 1);
      expect(el.scrollTop).toBe(0);
      expect(store.threads).toHaveLength(2000);
    }
    expect(pages).toBeGreaterThan(1);
    expect(store.find('th-00001')).toBeDefined();
    expect(store.selectedThreadIds).toEqual(['th-00002']);
    expect(list.find('[data-load-previous]').exists()).toBe(false);
    wrapper.unmount();
    viewportHeight.mockRestore();
    contentHeight.mockRestore();
    vi.unstubAllGlobals();
  }, 20000);
});
