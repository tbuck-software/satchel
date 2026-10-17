// @vitest-environment jsdom

import { createPinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App.vue';
import { useAppBootstrap, useComposeWindowBootstrap } from '@/bootstrap';
import { useUiStore } from '@/stores/ui';
import { nextTick } from 'vue';

vi.mock('@/bootstrap', () => ({
  useAppBootstrap: vi.fn(),
  useComposeWindowBootstrap: vi.fn(),
}));

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/inbox', name: 'inbox', component: { template: '<div />' } },
    { path: '/compose', name: 'compose', component: { template: '<div />' } },
  ],
});

async function mountAppAt(location: string) {
  await router.push(location);
  await router.isReady();
  return mount(App, { shallow: true, global: { plugins: [createPinia(), router] } });
}

describe('App bootstrap entry', () => {
  beforeEach(() => vi.clearAllMocks());

  it('boots a standalone composer window with the lean entry', async () => {
    const wrapper = await mountAppAt('/compose?standalone=1');

    expect(useComposeWindowBootstrap).toHaveBeenCalledTimes(1);
    expect(useAppBootstrap).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it('boots the application entry everywhere else', async () => {
    const wrapper = await mountAppAt('/inbox');

    expect(useAppBootstrap).toHaveBeenCalledTimes(1);
    expect(useComposeWindowBootstrap).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it('keeps the composer route in the main window on the application entry', async () => {
    // Reachable in place on a phone, which has one window. Only the popout —
    // marked by `standalone` — may skip the mailbox.
    const wrapper = await mountAppAt('/compose');

    expect(useAppBootstrap).toHaveBeenCalledTimes(1);
    expect(useComposeWindowBootstrap).not.toHaveBeenCalled();

    wrapper.unmount();
  });

  it('keeps the mobile drawer open for internal controls and closes on its backdrop', async () => {
    const wrapper = await mountAppAt('/inbox');
    const ui = useUiStore();
    ui.isMobile = true;
    ui.openMobileSidebar();
    await nextTick();

    await wrapper.get('.mobile-sidebar-panel').trigger('click');
    expect(ui.mobileSidebarOpen).toBe(true);
    await wrapper.get('.mobile-sidebar-backdrop').trigger('click');
    expect(ui.mobileSidebarOpen).toBe(false);
    wrapper.unmount();
  });
});
