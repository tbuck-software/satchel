// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent, nextTick } from 'vue';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useQueueFailureToasts } from './useQueueFailureToasts';
import { useToast } from './useToast';
import { useOutboxStore } from '@/stores/outbox';
import type { BackendOutboxItem } from '@/types/outbox';

const push = vi.fn();
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }));
const toast = useToast();
const Host = defineComponent({ setup() { useQueueFailureToasts(); return () => null; } });
function failed(id: string, action_type = 'send_message'): BackendOutboxItem {
  return { id, account_id: 'a', action_type, status: 'failed', retry_count: 6,
    created_at: '', updated_at: '', recipients: [], last_error: 'invalid recipient' };
}
beforeEach(() => { setActivePinia(createPinia()); vi.useFakeTimers(); push.mockClear(); });
afterEach(() => { toast.clear(); vi.useRealTimers(); });

it('expires the announcement while keeping the persistent queue state and action', async () => {
  const outbox = useOutboxStore();
  outbox.items = [failed('send'), failed('move', 'remote_mutation')];
  const wrapper = mount(Host);
  expect(toast.state.toasts).toHaveLength(2);
  expect(toast.state.toasts.every((item) => item.timeout === 7000)).toBe(true);
  expect(toast.state.toasts[1]!.action!.label).toBe('Try again');
  await toast.state.toasts[0]!.action!.handler();
  expect(push).toHaveBeenCalledWith({ name: 'outbox' });
  vi.advanceTimersByTime(7500);
  expect(toast.state.toasts).toHaveLength(0);
  expect(outbox.items).toHaveLength(2);
  // Reloading the same queue must not re-announce its standing condition.
  outbox.items = outbox.items.map((item) => ({ ...item }));
  await nextTick();
  expect(toast.state.toasts).toHaveLength(0);
  wrapper.unmount();
});

it('announces new identities, not count decreases, and allows a later failure transition', async () => {
  const outbox = useOutboxStore();
  outbox.items = [failed('a'), failed('b')];
  const wrapper = mount(Host);
  toast.clear();
  outbox.items = [failed('b')];
  await nextTick();
  expect(toast.state.toasts).toHaveLength(0);
  outbox.items = [failed('c')];
  await nextTick();
  expect(toast.state.toasts).toHaveLength(1);
  toast.clear();
  outbox.items = [];
  await nextTick();
  outbox.items = [failed('c')];
  await nextTick();
  expect(toast.state.toasts).toHaveLength(1);
  wrapper.unmount();
});
