// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useAppUpdateStore } from '@/stores/appUpdate';
import SidebarUpdateAction from './SidebarUpdateAction.vue';

describe('SidebarUpdateAction', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('stays out of the sidebar until an update is offered', () => {
    const wrapper = mount(SidebarUpdateAction);

    expect(wrapper.find('button').exists()).toBe(false);
  });

  it('shows download progress and becomes a restart action', async () => {
    let finishDownload: (() => void) | undefined;
    const update = useAppUpdateStore();
    update.offerUpdate({
      download: async (reportProgress) => {
        reportProgress(38);
        await new Promise<void>((resolve) => {
          finishDownload = resolve;
        });
      },
      restart: vi.fn(),
    });
    const wrapper = mount(SidebarUpdateAction);

    expect(wrapper.text()).toContain('Update available');

    await wrapper.get('button').trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('Downloading 38%');
    expect(wrapper.get('button').attributes('disabled')).toBeDefined();

    finishDownload?.();
    await flushPromises();
    expect(wrapper.text()).toContain('Restart to update');
    expect(wrapper.get('button').attributes('disabled')).toBeUndefined();
  });
});
