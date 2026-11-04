// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterAll, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import { useAccountsStore } from '@/stores/accounts';
import { useUiStore } from '@/stores/ui';
import type { Account } from '@/types/mail';
import ListFilterMenu from './ListFilterMenu.vue';

afterAll(() => vi.dynamicImportSettled());

function setup(accountCount = 2) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const all: Account[] = [
    { id: 'work', displayName: 'Work', email: 'tom@work.test', provider: 'imap', kind: 'email', color: '#4f8df7' },
    { id: 'personal', displayName: 'Personal', email: 'tom@home.test', provider: 'imap', kind: 'email', color: '#3fbf7f' },
  ];
  useAccountsStore().accounts = all.slice(0, accountCount);
  const wrapper = mount(ListFilterMenu, { attachTo: document.body, props: { showLayout: true }, global: { plugins: [pinia] } });
  return { wrapper, accounts: useAccountsStore(), ui: useUiStore() };
}

describe('ListFilterMenu scope chip', () => {
  it('names the chosen account only when one of several is chosen', async () => {
    const { wrapper, accounts } = setup();
    expect(wrapper.find('.list-filter-menu__scope').exists()).toBe(false);

    accounts.setActiveAccount('personal');
    await nextTick();
    const chip = wrapper.get('.list-filter-menu__scope');
    expect(chip.text()).toBe('Personal');
    expect(chip.attributes('aria-pressed')).toBeUndefined();
    expect(chip.attributes('aria-haspopup')).toBe('true');
    wrapper.unmount();

    const single = setup(1);
    single.accounts.setActiveAccount('work');
    await nextTick();
    expect(single.wrapper.find('.list-filter-menu__scope').exists()).toBe(false);
    single.wrapper.unmount();
  });

  it('opens the mailbox sheet on the phone', async () => {
    const { wrapper, accounts, ui } = setup();
    ui.isMobile = true;
    accounts.setActiveAccount('work');
    await nextTick();

    await wrapper.get('.list-filter-menu__scope').trigger('click');

    expect(ui.mailboxSwitcherOpen).toBe(true);
    wrapper.unmount();
  });

  it('opens the filter menu with its account section on the desktop', async () => {
    const { wrapper, accounts, ui } = setup();
    ui.isMobile = false;
    accounts.setActiveAccount('work');
    await nextTick();

    await wrapper.get('.list-filter-menu__scope').trigger('click');
    await vi.dynamicImportSettled();
    await flushPromises();

    expect(ui.mailboxSwitcherOpen).toBe(false);
    expect(document.body.querySelector('[role="menu"]')?.textContent).toContain('Personal');
    wrapper.unmount();
  });
});
