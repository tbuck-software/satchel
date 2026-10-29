// @vitest-environment jsdom
import { computed, ref, shallowRef } from 'vue';
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';

const harness = vi.hoisted(() => ({ push: vi.fn(), rows: null as unknown }));

vi.mock('vue-router', () => ({ useRouter: () => ({ push: harness.push }) }));
vi.mock('@/composables/useListSearch', () => ({
  useListSearch: () => {
    const rows = harness.rows as { value: Thread[] };
    return {
      entries: computed(() => rows.value.map((thread) => ({ kind: 'thread', id: `thread-${thread.id}`, thread }))),
      resultThreads: rows,
      loading: ref(false),
      showProgress: ref(false),
      loadingMore: ref(false),
      hasMore: ref(false),
      errorText: ref(''),
      warnings: ref([]),
      pendingCount: ref(0),
      refresh: vi.fn(),
      narrowed: ref(true),
      loadMore: vi.fn(),
    };
  },
}));

import ListSearchResults from './ListSearchResults.vue';
import { useListSearchStore } from '@/stores/listSearch';
import { useThreadsStore } from '@/stores/threads';

function thread(id: string, workflowState: Thread['workflowState'] = 'inbox'): Thread {
  return {
    id, accountId: 'acc-work', subject: id, participants: [], snippet: '', lastMessageAt: '2026-10-01T10:00:00Z',
    lastMessageAtMs: Date.parse('2026-10-01T10:00:00Z'), messageCount: 1, unreadCount: 0, hasAttachments: false,
    workflowState, priority: 0, isStarred: false, hasInvite: false, mailboxNames: ['INBOX'], tags: [],
  };
}

/** The list itself is not under test: a row is an avatar and a body. */
const ThreadGroupList = {
  name: 'ThreadGroupList',
  props: ['items', 'selectedThreadIds'],
  emits: ['check', 'activate'],
  template: `<div>
    <i v-for="item in items" :key="item.id" :data-row="item.thread.id"
       :data-checked="selectedThreadIds.includes(item.thread.id)"
       @click="$emit('check', { threadId: item.thread.id, shiftKey: $event.shiftKey, metaKey: false, ctrlKey: false })" />
  </div>`,
};

function render() {
  return mount(ListSearchResults, {
    props: { mailboxLabel: 'Inbox', mailbox: { accountId: '', workflowFilter: 'all', mailboxFilter: { kind: 'section', value: 'inbox' }, activeContactEmail: '' } },
    global: { stubs: { ThreadGroupList } },
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
  harness.push.mockReset();
  harness.rows = shallowRef([thread('th-hit-1'), thread('th-loaded'), thread('th-hit-2', 'todo')]);
  useThreadsStore().threads = [thread('th-loaded')];
});

describe('list search results and the selection', () => {
  it('deselects a result on another avatar click and keeps other results selected', async () => {
    const wrapper = render();
    await flushPromises();
    const threads = useThreadsStore();

    await wrapper.get('[data-row="th-hit-1"]').trigger('click');
    await wrapper.get('[data-row="th-hit-2"]').trigger('click');
    expect(threads.selectedThreadIds).toEqual(['th-hit-1', 'th-hit-2']);

    await wrapper.get('[data-row="th-hit-1"]').trigger('click');
    expect(wrapper.get('[data-row="th-hit-1"]').attributes('data-checked')).toBe('false');
    expect(threads.selectedThreadIds).toEqual(['th-hit-2']);

    await wrapper.get('[data-row="th-hit-2"]').trigger('click');
    expect(threads.selectedThreadIds).toEqual([]);
    expect(harness.push).not.toHaveBeenCalled();
    expect(useListSearchStore().openThreadId).toBe('');
    wrapper.unmount();
  });

  it('checks a result from its avatar instead of opening it', async () => {
    const wrapper = render();
    await flushPromises();
    const threads = useThreadsStore();

    await wrapper.get('[data-row="th-hit-1"]').trigger('click');
    await wrapper.get('[data-row="th-hit-2"]').trigger('click', { shiftKey: true });

    expect(threads.selectedThreadIds).toEqual(['th-hit-1', 'th-loaded', 'th-hit-2']);
    expect(wrapper.get('[data-row="th-hit-2"]').attributes('data-checked')).toBe('true');
    // Nothing opened: no route, no reader change.
    expect(harness.push).not.toHaveBeenCalled();
    expect(useListSearchStore().openThreadId).toBe('');
    // The selection bar's actions read the store, which now holds the hits.
    expect(threads.selectedThreads.map((row) => row.id).sort()).toEqual(['th-hit-1', 'th-hit-2', 'th-loaded']);
  });

  it('hands selection back to the list when the search closes', async () => {
    harness.rows = shallowRef([thread('th-loaded'), thread('th-hit-done', 'done')]);
    const wrapper = render();
    await flushPromises();
    const threads = useThreadsStore();
    await wrapper.get('[data-row="th-loaded"]').trigger('click');
    await wrapper.get('[data-row="th-hit-done"]').trigger('click', { shiftKey: true });
    expect(threads.selectedThreadIds).toEqual(['th-loaded', 'th-hit-done']);

    wrapper.unmount();
    await flushPromises();
    // The archived hit is not in the Inbox; the list keeps what it shows.
    expect(threads.selectedThreadIds).toEqual(['th-loaded']);
  });
});
