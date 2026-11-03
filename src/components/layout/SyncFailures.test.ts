// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent } from 'vue';
import { beforeEach, expect, it, vi } from 'vitest';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import SyncFailures from './SyncFailures.vue';

const push = vi.fn();
vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }));

const BottomSheetStub = defineComponent({ template: '<div><slot /></div>' });

beforeEach(() => {
  setActivePinia(createPinia());
  push.mockReset();
});

it('names a failed non-send change and leads to the Outbox, where it can be retried or discarded', async () => {
  const connection = useConnectionStore();
  connection.failedRemoteMutations = 1;
  const outbox = useOutboxStore();
  vi.spyOn(outbox, 'load').mockResolvedValue(undefined);
  outbox.items = [{
    id: 'flags', account_id: 'a', action_type: 'remote_mutation', mutation_kind: 'flags',
    status: 'failed', retry_count: 3, created_at: '', updated_at: '', recipients: [],
  }];
  const wrapper = mount(SyncFailures, {
    props: { open: true },
    global: { stubs: { teleport: true, BottomSheet: BottomSheetStub } },
  });
  await flushPromises();

  expect(wrapper.get('.sync-failures__list').text()).toContain('Read or star status');
  const buttons = wrapper.findAll('.sync-failures__actions button');
  expect(buttons.map((button) => button.text())).toContain('Try again');
  await buttons.find((button) => button.text() === 'Outbox')!.trigger('click');
  expect(push).toHaveBeenCalledWith({ name: 'outbox' });
  expect(wrapper.emitted('close')).toHaveLength(1);
});
