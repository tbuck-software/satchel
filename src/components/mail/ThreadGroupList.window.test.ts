// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { nextTick } from 'vue';
import { describe, expect, it, vi } from 'vitest';
import type { ThreadListEntry } from '@/utils/threadList';
import ThreadGroupList from './ThreadGroupList.vue';

function rows(start: number, count = 2000): ThreadListEntry[] {
  return Array.from({ length: count }, (_, offset) => {
    const id = `thread-${start + offset}`;
    return { kind: 'thread', id, thread: {
      id, accountId: 'a', subject: id, participants: [], snippet: '',
      lastMessageAt: '2026-01-01', lastMessageAtMs: 0, messageCount: 1,
      unreadCount: 0, hasAttachments: false, workflowState: 'done', priority: 1,
      mailboxNames: [], tags: [],
    } } as ThreadListEntry;
  });
}

describe('thread list sliding window', () => {
  it('keeps reverse page retry visible above the virtual rows without adding scroll height', async () => {
    const wrapper = shallowMount(ThreadGroupList, { props: {
      items: rows(40), selectedThreadIds: [], senderReviewSetupComplete: true, bulkBusy: false,
      loadMoreFailed: true, loadPreviousFailed: true,
    } });
    const alert = wrapper.get('[role="alert"]');
    expect(alert.element.parentElement?.classList.contains('sticky')).toBe(true);
    expect(alert.element.parentElement?.classList.contains('h-0')).toBe(true);
    expect(wrapper.findAll('[role="alert"]')).toHaveLength(1);
    await alert.get('app-button-stub').trigger('click');
    expect(wrapper.emitted('retryLoadMore')).toHaveLength(1);
    wrapper.unmount();
  });

  it('fills a short viewport when the first page arrives', async () => {
    const wrapper = shallowMount(ThreadGroupList, { props: {
      items: [], selectedThreadIds: [], senderReviewSetupComplete: true, bulkBusy: false,
    } });
    const el = wrapper.get('.thread-list-scroller').element as HTMLElement;
    Object.defineProperties(el, {
      clientHeight: { value: 600 }, scrollHeight: { value: 216 },
    });
    await nextTick();
    const before = wrapper.emitted('loadMore')?.length ?? 0;
    await wrapper.setProps({ items: rows(0, 3) });
    await nextTick();
    expect(wrapper.emitted('loadMore')).toHaveLength(before + 1);
    wrapper.unmount();
  });

  it('keeps the visible thread and its pixel offset and does not fetch again after trimming', async () => {
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
      callback(0);
      return 1;
    });
    const wrapper = shallowMount(ThreadGroupList, { props: {
      items: rows(0), selectedThreadIds: [], senderReviewSetupComplete: true, bulkBusy: false,
    } });
    const el = wrapper.get('.thread-list-scroller').element as HTMLElement;
    const height = Number.parseFloat(el.style.getPropertyValue('--thread-row-h'));
    Object.defineProperties(el, {
      clientHeight: { value: 600 },
      scrollHeight: { get: () => 2000 * height },
    });
    await nextTick();
    const initialRequests = wrapper.emitted('loadMore')?.length ?? 0;
    el.scrollTop = 1975 * height + 7;
    await wrapper.get('.thread-list-scroller').trigger('scroll');
    await nextTick();
    const requests = wrapper.emitted('loadMore')?.length ?? 0;
    expect(requests).toBe(initialRequests + 1);
    await wrapper.setProps({ items: rows(40) });
    await nextTick();
    expect(el.scrollTop).toBe(1935 * height + 7);
    expect(wrapper.emitted('loadMore')).toHaveLength(requests);
    wrapper.unmount();
    vi.restoreAllMocks();
  });
});
