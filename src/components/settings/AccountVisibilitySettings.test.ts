// @vitest-environment jsdom
import { mount, flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { expect, it, vi } from 'vitest';
import { useAccountsStore } from '@/stores/accounts';
import AccountVisibilitySettings from './AccountVisibilitySettings.vue';

it('shows and saves a schedule at minute precision, including legacy second values', async () => {
  setActivePinia(createPinia());
  const accounts = useAccountsStore();
  accounts.visibility = [{
    account_id: 'work', visible: true, local_date: '2026-10-27',
    policy: { mode: 'schedule', weekdays: [1], from: '09:00:00', to: '17:00:00', override_date: null, override_visible: null },
  }];
  accounts.visibilityReady = true;
  const save = vi.spyOn(accounts, 'setVisibility').mockResolvedValue(true);
  const wrapper = mount(AccountVisibilitySettings, { props: { accountId: 'work' } });
  const inputs = wrapper.findAll<HTMLInputElement>('input[type="time"]');
  expect(inputs.map(input => [input.element.value, input.element.step])).toEqual([
    ['09:00', '60'], ['17:00', '60'],
  ]);
  await inputs[0]!.setValue('10:30');
  await wrapper.get('.visibility-settings__actions button').trigger('click');
  await flushPromises();
  expect(save).toHaveBeenCalledWith('work', expect.objectContaining({ from: '10:30', to: '17:00' }));
  wrapper.unmount();
});
