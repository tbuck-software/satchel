// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/*
 * PERF-09. Every thread linked to a topic started a full load the moment the
 * timeline arrived — twenty threads, twenty loads, no limiter and no regard
 * for whether a card was anywhere near the screen.
 */

const harness = vi.hoisted(() => ({
  loadByThread: vi.fn(),
  loadAttachments: vi.fn(),
  timeline: [] as unknown[],
}));

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({
    getByThread: () => [],
    loadByThread: harness.loadByThread,
    loadAttachmentsForMessages: harness.loadAttachments,
  }),
}));
vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    items: [{ id: 'topic-1' }],
    byId: () => ({ id: 'topic-1', name: 'Quota', description: '', status: 'active', icon: null }),
    load: vi.fn(),
    fetchTimeline: vi.fn(async () => harness.timeline),
    filesFor: () => [],
    loadFiles: vi.fn(async () => {}),
  }),
}));
vi.mock('@/stores/composer', () => ({ useComposerStore: () => ({ openReply: vi.fn() }) }));
vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ success: vi.fn(), error: vi.fn(), info: vi.fn() }),
}));
vi.mock('@/utils/saveAttachment', () => ({ saveAttachmentWithDialog: vi.fn() }));

import TopicDetail from './TopicDetail.vue';

/** An observer the test drives: it reports what the test says is visible. */
class FakeIntersectionObserver {
  static instances: FakeIntersectionObserver[] = [];
  readonly observed: Element[] = [];
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeIntersectionObserver.instances.push(this);
  }
  observe(element: Element) {
    this.observed.push(element);
  }
  disconnect() {
    this.observed.length = 0;
  }
  unobserve() {}
  takeRecords() {
    return [];
  }
  reveal(count: number) {
    const entries = this.observed.slice(0, count).map((target) => ({
      target,
      isIntersecting: true,
    })) as unknown as IntersectionObserverEntry[];
    this.callback(entries, this as unknown as IntersectionObserver);
  }
}

function linkedThread(index: number) {
  return {
    item_type: 'thread',
    item_id: `thread-${index}`,
    thread_id: `thread-${index}`,
    sort_date: `2026-08-${String(index + 1).padStart(2, '0')}T08:00:00Z`,
    subject: `Thread ${index}`,
    snippet: 'Vorschau',
  };
}

beforeEach(() => {
  FakeIntersectionObserver.instances = [];
  vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
  vi.stubGlobal('scrollTo', vi.fn());
  Element.prototype.scrollTo = vi.fn();
  harness.loadByThread.mockReset();
  harness.loadAttachments.mockReset();
  harness.timeline = Array.from({ length: 20 }, (_, index) => linkedThread(index));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function mountFeed() {
  const wrapper = mount(TopicDetail, {
    props: { topicId: 'topic-1' },
    global: {
      plugins: [createPinia()],
      stubs: { TopicFeedItem: { template: '<article class="stub-card" />' } },
    },
  });
  await flushPromises();
  return wrapper;
}

describe('topic feed hydration', () => {
  it('loads nothing until a card is in view', async () => {
    harness.loadByThread.mockResolvedValue(undefined);
    await mountFeed();

    expect(FakeIntersectionObserver.instances.at(-1)?.observed.length).toBe(20);
    expect(harness.loadByThread).not.toHaveBeenCalled();
  });

  it('starts at most the visible cards, three at a time', async () => {
    const releases: Array<() => void> = [];
    harness.loadByThread.mockImplementation(() => new Promise<void>((resolve) => {
      releases.push(resolve);
    }));
    await mountFeed();

    // Three on screen and two in the overscan band.
    FakeIntersectionObserver.instances.at(-1)!.reveal(5);
    await flushPromises();
    expect(harness.loadByThread).toHaveBeenCalledTimes(3);

    for (let round = 0; round < 5; round += 1) {
      releases.splice(0).forEach((release) => release());
      await flushPromises();
    }
    expect(harness.loadByThread).toHaveBeenCalledTimes(5);
    expect(harness.loadAttachments).toHaveBeenCalledTimes(5);
  });

  it('does not start the same thread twice when its card is seen again', async () => {
    harness.loadByThread.mockResolvedValue(undefined);
    await mountFeed();

    const observer = FakeIntersectionObserver.instances.at(-1)!;
    observer.reveal(1);
    observer.reveal(1);
    await flushPromises();

    expect(harness.loadByThread).toHaveBeenCalledTimes(1);
  });
});
