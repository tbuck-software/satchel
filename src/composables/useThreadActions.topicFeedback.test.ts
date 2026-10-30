// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent, nextTick } from 'vue';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ToastStack from '@/components/ui/ToastStack.vue';
import UndoFab from '@/components/ui/UndoFab.vue';
import { useTopicsStore } from '@/stores/topics';
import { useThreadActions } from './useThreadActions';
import { useToast } from './useToast';

const backend = vi.hoisted(() => ({ remove: vi.fn(), undo: vi.fn(), list: vi.fn() }));
vi.mock('@/services/backend', async (original) => ({
  ...await original<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  backendRemoveTopicItem: backend.remove,
  backendUndoTopicMemberships: backend.undo,
  backendListTopicMemberships: backend.list,
}));
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => ({ visibilityKey: 'account' }) }));
vi.mock('@/stores/connection', () => ({ useConnectionStore: () => ({
  reportBackendError: vi.fn(), clearBackendError: vi.fn(), retryToken: 0,
}) }));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => ({}) }));
vi.mock('@/stores/mailboxes', () => ({ useMailboxesStore: () => ({}) }));
vi.mock('@/composables/useWorkflow', () => ({ useWorkflow: () => ({}) }));
vi.mock('@/composables/useListKeyboardNavigation', () => ({ focusListThread: vi.fn() }));
vi.mock('@/composables/useShortcutHints', () => ({ useShortcutHints: () => ({ hint: () => null }) }));
vi.mock('@/router/mailViews', () => ({ openView: vi.fn() }));
vi.mock('vue-router', () => ({ useRoute: () => ({}), useRouter: () => ({}) }));

const Presenters = defineComponent({
  components: { ToastStack, UndoFab },
  template: '<div><ToastStack /><UndoFab variant="floating" /></div>',
});
beforeEach(() => {
  setActivePinia(createPinia());
  useToast().clear();
  vi.resetAllMocks();
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query === '(max-width: 640px)', addEventListener() {}, removeEventListener() {},
  }));
  backend.list.mockResolvedValue([]);
  backend.undo.mockResolvedValue(['a']);
});
afterEach(() => {
  useToast().clear();
  vi.unstubAllGlobals();
});

it('shows partial-removal counts and reachable inline Undo on a compact touch screen', async () => {
  const topics = useTopicsStore();
  topics.memberships = { 'thread:a': ['topic'], 'thread:b': ['topic'] };
  backend.remove.mockImplementation(async ({ itemId }: { itemId: string }) => {
    if (itemId === 'b') throw new Error('remove refused');
    return { item_id: itemId, revision: 'removed-a' };
  });
  const wrapper = mount(Presenters);
  try {
    await useThreadActions().removeFromTopic(['a', 'b'], 'topic');
    await nextTick();
    const row = wrapper.get('.toast--warning');
    expect(row.isVisible()).toBe(true);
    expect(row.text()).toContain('1 removed from topic');
    expect(row.text()).toContain('1 could not be removed');
    expect(row.get('.toast__action').text()).toBe('Undo');
    expect(wrapper.find('.undo-fab__main').exists()).toBe(false);
    expect(topics.topicIdsFor('thread', 'b')).toEqual(['topic']);
    await row.get('.toast__action').trigger('click');
    await vi.waitFor(() => expect(useToast().state.toasts).toHaveLength(0));
    expect(backend.undo).toHaveBeenCalledExactlyOnceWith(
      'topic', 'thread', [{ item_id: 'a', revision: 'removed-a' }], true,
    );
  } finally {
    wrapper.unmount();
  }
});

it('keeps the existing floating Undo presentation for a fully successful removal', async () => {
  backend.remove.mockResolvedValue({ item_id: 'a', revision: 'removed-a' });
  const wrapper = mount(Presenters);
  try {
    await useThreadActions().removeFromTopic(['a'], 'topic');
    await nextTick();
    expect(wrapper.find('.toast').exists()).toBe(false);
    expect(wrapper.get('.undo-fab__main').isVisible()).toBe(true);
    await wrapper.get('.undo-fab__main').trigger('click');
    await vi.waitFor(() => expect(useToast().state.toasts).toHaveLength(0));
    expect(backend.undo).toHaveBeenCalledOnce();
  } finally {
    wrapper.unmount();
  }
});
