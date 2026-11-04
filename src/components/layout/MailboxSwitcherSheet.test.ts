// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterAll, describe, expect, it, vi } from 'vitest';
import { useUiStore } from '@/stores/ui';
import { useThreadsStore } from '@/stores/threads';
import MailboxSwitcherSheet from './MailboxSwitcherSheet.vue';

afterAll(() => vi.dynamicImportSettled());

const blank = { template: '<div />' };

async function mountSheet() {
  const pinia = createPinia();
  setActivePinia(pinia);
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
  return { wrapper, ui, router };
}

function row(wrapper: Awaited<ReturnType<typeof mountSheet>>['wrapper'], label: string) {
  const match = wrapper.findAll('.nav-item').find((item) => item.text().startsWith(label));
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
});
