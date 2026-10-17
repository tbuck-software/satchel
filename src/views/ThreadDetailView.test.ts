// @vitest-environment jsdom

import { defineComponent, nextTick, reactive } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  route: undefined as { params: { id?: string } } | undefined,
  router: {
    back: vi.fn(),
    push: vi.fn(),
  },
  store: undefined as {
    threads: Array<{ id: string }>;
    selectThread: ReturnType<typeof vi.fn>;
    ensureThreadLoaded: ReturnType<typeof vi.fn>;
    releaseRetainedThread: ReturnType<typeof vi.fn>;
  } | undefined,
}));

vi.mock('vue-router', () => ({
  useRoute: () => harness.route,
  useRouter: () => harness.router,
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => harness.store,
}));

import ThreadDetailView from './ThreadDetailView.vue';

const ThreadViewStub = defineComponent({
  name: 'ThreadView',
  props: {
    threadId: { type: String, required: true },
    showBack: Boolean,
  },
  emits: ['back'],
  template: `
    <div data-test="thread-view" :data-thread-id="threadId" :data-show-back="String(showBack)">
      <button type="button" data-test="thread-view-back" @click="$emit('back')">Back</button>
    </div>
  `,
});

const ThreadPaneSkeletonStub = defineComponent({
  name: 'ThreadPaneSkeleton',
  template: '<div data-test="thread-skeleton" />',
});

function render() {
  return mount(ThreadDetailView, {
    global: {
      mocks: { $t: (key: string) => key },
      stubs: {
        ThreadView: ThreadViewStub,
        ThreadPaneSkeleton: ThreadPaneSkeletonStub,
      },
    },
  });
}

function resetHarness(id: string | undefined) {
  harness.route = reactive({ params: { id } });
  harness.router.back.mockReset();
  harness.router.push.mockReset().mockResolvedValue(undefined);
  harness.store = reactive({
    threads: [],
    selectThread: vi.fn(),
    ensureThreadLoaded: vi.fn().mockResolvedValue(undefined),
    releaseRetainedThread: vi.fn(),
  });
}

describe('ThreadDetailView', () => {
  beforeEach(() => {
    resetHarness('thread-1');
  });

  it('selects the routed thread and retains it while loading', async () => {
    const ensure = harness.store!.ensureThreadLoaded;
    const wrapper = render();

    await flushPromises();

    expect(harness.store!.selectThread).toHaveBeenCalledWith('thread-1');
    expect(ensure).toHaveBeenCalledWith('thread-1', { retainAcrossListLoads: true });
    expect(wrapper.find('[data-test="thread-skeleton"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it('shows a skeleton until an unavailable thread load settles', async () => {
    let resolveLoad!: () => void;
    harness.store!.ensureThreadLoaded.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        resolveLoad = resolve;
      }),
    );
    const wrapper = render();

    await nextTick();
    expect(wrapper.find('[data-test="thread-skeleton"]').exists()).toBe(true);

    resolveLoad();
    await flushPromises();

    expect(wrapper.get('.thread-state').text()).toContain('thread.detail.unavailable');
    expect(wrapper.get('.thread-back').text()).toBe('thread.detail.backToInbox');
    wrapper.unmount();
  });

  it('renders the loaded thread and passes the back action through', async () => {
    harness.store!.threads.push({ id: 'thread-1' });
    const wrapper = render();

    await flushPromises();

    expect(wrapper.get('[data-test="thread-view"]').attributes('data-thread-id')).toBe('thread-1');
    expect(wrapper.get('[data-test="thread-view"]').attributes('data-show-back')).toBe('true');

    await wrapper.get('[data-test="thread-view-back"]').trigger('click');

    expect(harness.router.push).toHaveBeenCalledWith({ name: 'inbox' });
    expect(harness.router.back).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('uses browser history when going back is available', async () => {
    const getLength = vi.spyOn(window.history, 'length', 'get').mockReturnValue(2);
    harness.store!.threads.push({ id: 'thread-1' });
    const wrapper = render();

    await flushPromises();
    await wrapper.get('[data-test="thread-view-back"]').trigger('click');

    expect(harness.router.back).toHaveBeenCalledTimes(1);
    expect(harness.router.push).not.toHaveBeenCalled();
    getLength.mockRestore();
    wrapper.unmount();
  });

  it('releases the previous retained thread on route changes and the current one on unmount', async () => {
    const wrapper = render();
    await flushPromises();

    harness.route!.params.id = 'thread-2';
    await nextTick();
    await flushPromises();

    expect(harness.store!.releaseRetainedThread).toHaveBeenCalledWith('thread-1');
    expect(harness.store!.selectThread).toHaveBeenLastCalledWith('thread-2');
    expect(harness.store!.ensureThreadLoaded).toHaveBeenLastCalledWith('thread-2', {
      retainAcrossListLoads: true,
    });

    wrapper.unmount();

    expect(harness.store!.releaseRetainedThread).toHaveBeenLastCalledWith('thread-2');
    expect(harness.store!.releaseRetainedThread).toHaveBeenCalledTimes(2);
  });

  it('does not let a stale load finish the newer route', async () => {
    let resolveFirst!: () => void;
    let resolveSecond!: () => void;
    harness.store!.ensureThreadLoaded
      .mockReturnValueOnce(new Promise<void>((resolve) => { resolveFirst = resolve; }))
      .mockReturnValueOnce(new Promise<void>((resolve) => { resolveSecond = resolve; }));
    const wrapper = render();

    await nextTick();
    harness.route!.params.id = 'thread-2';
    await nextTick();

    resolveFirst();
    await flushPromises();
    expect(wrapper.find('[data-test="thread-skeleton"]').exists()).toBe(true);

    resolveSecond();
    await flushPromises();
    expect(wrapper.find('.thread-state').exists()).toBe(true);
    expect(harness.store!.releaseRetainedThread).toHaveBeenCalledWith('thread-1');
    wrapper.unmount();
  });

  it('falls back to the inbox when no thread id is routed', async () => {
    resetHarness(undefined);
    const wrapper = render();

    await flushPromises();

    expect(harness.store!.selectThread).not.toHaveBeenCalled();
    expect(harness.store!.ensureThreadLoaded).not.toHaveBeenCalled();
    expect(wrapper.get('.thread-state').text()).toContain('thread.detail.unavailable');

    await wrapper.get('.thread-back').trigger('click');
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'inbox' });
    wrapper.unmount();
  });
});
