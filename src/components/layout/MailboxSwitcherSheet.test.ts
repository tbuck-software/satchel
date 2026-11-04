// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterAll, describe, expect, it, vi } from 'vitest';
import { useUiStore } from '@/stores/ui';
import { useThreadsStore } from '@/stores/threads';
import { useAccountsStore } from '@/stores/accounts';
import MailboxSwitcherSheet from './MailboxSwitcherSheet.vue';

afterAll(() => vi.dynamicImportSettled());

const blank = { template: '<div />' };

async function mountSheet(withAccounts = false) {
  const pinia = createPinia();
  setActivePinia(pinia);
  if (withAccounts) {
    useAccountsStore().accounts = [
      { id: 'work', displayName: 'Work', email: 'tom@work.test', provider: 'imap', kind: 'email', color: '#4f8df7' },
      { id: 'personal', displayName: 'Personal', email: 'tom@home.test', provider: 'imap', kind: 'email', color: '#3fbf7f' },
    ];
  }
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['inbox', 'outbox'].map((name) => ({ path: `/${name}`, name, component: blank })),
  });
  await router.push('/inbox');
  await router.isReady();
  const ui = useUiStore();
  ui.isMobile = true;
  ui.isSinglePane = true;
  const wrapper = mount(MailboxSwitcherSheet, {
    attachTo: document.body,
    global: { plugins: [pinia, router] },
  });
  ui.openMailboxSwitcher();
  await nextTick();
  await flushPromises();
  // The account group is an async component; wait until it has rendered.
  for (let tries = 0; withAccounts && tries < 20 && !wrapper.find('.sidebar-accounts').exists(); tries += 1) {
    await vi.dynamicImportSettled();
    await flushPromises();
  }
  return { wrapper, ui, router };
}

function row(wrapper: Awaited<ReturnType<typeof mountSheet>>['wrapper'], label: string) {
  const match = wrapper.findAll('.nav-item').find((item) => item.find('.nav-item__label').text() === label);
  if (!match) throw new Error(`no row "${label}" in ${wrapper.findAll('.nav-item').map((item) => item.text()).join(' | ')}`);
  return match;
}

describe('MailboxSwitcherSheet', () => {
  it('offers the mail lists the drawer had, under a title, with the current one marked', async () => {
    const { wrapper } = await mountSheet();

    const dialog = wrapper.get('[role="dialog"]');
    expect(dialog.attributes('aria-label')).toBe('Mailboxes');
    expect(wrapper.get('.mailbox-sheet__title').text()).toBe('Mailboxes');
    // The grab handle that goes with the drag-to-dismiss gesture.
    expect(wrapper.find('.sheet-grip').exists()).toBe(true);
    for (const label of ['Inbox', 'Later', 'Set aside', 'Outbox', 'Done']) row(wrapper, label);
    expect(row(wrapper, 'Inbox').classes()).toContain('nav-item--active');
    expect(row(wrapper, 'Later').classes()).not.toContain('nav-item--active');
    wrapper.unmount();
  });

  it('switches the list and closes on a pick', async () => {
    const { wrapper, ui } = await mountSheet();
    const threads = useThreadsStore();

    await row(wrapper, 'Later').trigger('click');

    expect(threads.workflowFilter).toBe('later');
    expect(ui.mailboxSwitcherOpen).toBe(false);
    wrapper.unmount();
  });

  it('closes on the backdrop and on Escape/Back', async () => {
    const { wrapper, ui } = await mountSheet();

    await wrapper.get('.sheet-scrim').trigger('click');
    expect(ui.mailboxSwitcherOpen).toBe(false);

    ui.openMailboxSwitcher();
    await nextTick();
    // Escape and the Android Back gesture both go through the overlay stack.
    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.mailboxSwitcherOpen).toBe(false);
    wrapper.unmount();
  });

  describe('with several accounts', () => {
    async function reopen() {
      useUiStore().openMailboxSwitcher();
      await nextTick();
      await flushPromises();
    }
    const strongMarks = (wrapper: Awaited<ReturnType<typeof mountSheet>>['wrapper']) =>
      wrapper.findAll('.nav-item--active').map((item) => item.text());

    it('marks the view strongly and the account scope quietly, one strong mark at a time', async () => {
      const { wrapper } = await mountSheet(true);

      const all = row(wrapper, 'All accounts');
      expect(all.classes()).toContain('nav-item--scope');
      expect(all.attributes('aria-current')).toBe('true');
      expect(all.classes()).not.toContain('nav-item--active');
      expect(strongMarks(wrapper)).toEqual([expect.stringMatching(/^Inbox/)]);

      await row(wrapper, 'Later').trigger('click');
      await reopen();
      expect(strongMarks(wrapper)).toEqual([expect.stringMatching(/^Later/)]);
      expect(wrapper.findAll('.nav-item--scope').map((item) => item.text())).toEqual([expect.stringMatching(/^All accounts/)]);
      wrapper.unmount();
    });

    it('keeps the view and rescopes it on an account tap, with the sheet left open', async () => {
      const { wrapper, ui } = await mountSheet(true);
      const threads = useThreadsStore();
      const accounts = useAccountsStore();
      await row(wrapper, 'Later').trigger('click');
      await reopen();

      await row(wrapper, 'Personal').trigger('click');

      expect(accounts.activeAccountId).toBe('personal');
      expect(threads.workflowFilter).toBe('later');
      expect(ui.mailboxSwitcherOpen).toBe(true);
      expect(row(wrapper, 'Personal').classes()).toContain('nav-item--scope');
      expect(row(wrapper, 'All accounts').classes()).not.toContain('nav-item--scope');
      expect(strongMarks(wrapper)).toEqual([expect.stringMatching(/^Later/)]);

      // A view row still closes it.
      await row(wrapper, 'Done').trigger('click');
      expect(threads.workflowFilter).toBe('done');
      expect(accounts.activeAccountId).toBe('personal');
      expect(ui.mailboxSwitcherOpen).toBe(false);
      wrapper.unmount();
    });

    it('falls back to the Inbox when the folder on screen belongs to another account', async () => {
      const { wrapper, router } = await mountSheet(true);
      const threads = useThreadsStore();
      useAccountsStore().setActiveAccount('work');
      threads.setMailboxFilter({ kind: 'folder', value: 'Projects' });
      await nextTick();

      await row(wrapper, 'Personal').trigger('click');

      expect(threads.mailboxFilter).toEqual({ kind: 'section', value: 'inbox' });
      expect(router.currentRoute.value.name).toBe('inbox');
      wrapper.unmount();
    });
  });
});
