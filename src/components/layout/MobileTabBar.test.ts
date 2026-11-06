// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { registerListScrollTop } from '@/composables/listScrollTop';
import MobileTabBar from './MobileTabBar.vue';
import { useUiStore } from '@/stores/ui';

const blank = { template: '<div />' };

async function mountAt(location: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['inbox', 'outbox', 'calendar', 'tasks', 'contacts', 'settings'].map((name) => ({
      path: `/${name}`,
      name,
      component: blank,
    })),
  });
  await router.push(location);
  await router.isReady();
  const wrapper = mount(MobileTabBar, { global: { plugins: [createPinia(), router] } });
  return { wrapper, router };
}

function tab(wrapper: Awaited<ReturnType<typeof mountAt>>['wrapper'], id: string) {
  return wrapper.get(`[data-tab="${id}"]`);
}

let unregister: (() => void) | null = null;
afterEach(() => {
  unregister?.();
  unregister = null;
});

describe('MobileTabBar', () => {
  it('draws one icon per tab and puts the attention dot on Mail only', async () => {
    const { wrapper } = await mountAt('/inbox');
    useUiStore().drawerAttention = true;
    await wrapper.vm.$nextTick();
    for (const id of ['mail', 'calendar', 'tasks', 'contacts', 'settings']) {
      expect(tab(wrapper, id).findAll('svg')).toHaveLength(1);
    }
    expect(tab(wrapper, 'mail').find('.mobile-tab-bar__dot').exists()).toBe(true);
    expect(wrapper.findAll('.mobile-tab-bar__dot')).toHaveLength(1);
  });

  it('names the five areas and marks the one on screen', async () => {
    const { wrapper } = await mountAt('/contacts');

    expect(wrapper.findAll('button').map((button) => button.text())).toEqual(['Mail', 'Calendar', 'Tasks', 'Contacts', 'Settings']);
    expect(tab(wrapper, 'contacts').attributes('aria-current')).toBe('page');
    expect(tab(wrapper, 'mail').attributes('aria-current')).toBeUndefined();
  });

  it('counts every mail screen as Mail, the outbox included', async () => {
    const { wrapper } = await mountAt('/outbox');

    expect(tab(wrapper, 'mail').attributes('aria-current')).toBe('page');
  });

  it('switches areas by route', async () => {
    const { wrapper, router } = await mountAt('/inbox');

    await tab(wrapper, 'calendar').trigger('click');
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe('calendar'));
    await tab(wrapper, 'mail').trigger('click');
    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe('inbox'));
  });

  it('scrolls the list back to the top when Mail is tapped again', async () => {
    const scroll = vi.fn();
    unregister = registerListScrollTop(scroll);
    const { wrapper, router } = await mountAt('/inbox');

    await tab(wrapper, 'mail').trigger('click');

    expect(scroll).toHaveBeenCalledOnce();
    expect(router.currentRoute.value.name).toBe('inbox');
  });

  it('takes Mail tapped on the outbox back to the list instead', async () => {
    const scroll = vi.fn();
    unregister = registerListScrollTop(scroll);
    const { wrapper, router } = await mountAt('/outbox');

    await tab(wrapper, 'mail').trigger('click');

    await vi.waitFor(() => expect(router.currentRoute.value.name).toBe('inbox'));
    expect(scroll).not.toHaveBeenCalled();
  });

  it('takes a tap on the current area back to its top, as iOS does', async () => {
    const { wrapper, router } = await mountAt('/settings?section=accounts');

    await tab(wrapper, 'settings').trigger('click');

    await vi.waitFor(() => expect(router.currentRoute.value.fullPath).toBe('/settings'));
  });
});
