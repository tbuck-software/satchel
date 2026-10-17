// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { enableAutoUnmount, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useUiStore } from '@/stores/ui';
import ThreadMobileActions from './ThreadMobileActions.vue';

enableAutoUnmount(afterEach);

const baseProps = {
  visible: true,
  canReply: true,
  isUnread: true,
  isStarred: false,
  isAside: false,
  isDone: false,
};

describe('ThreadMobileActions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useUiStore().isSinglePane = true;
  });

  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('preserves mobile action labels, visibility, and disabled state', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: { ...baseProps, isStarred: true, isAside: true, isDone: true },
    });

    expect(wrapper.find('.bar-btn[title="More"]').exists()).toBe(true);
    expect(wrapper.find('.bar-btn[title="Reply"]').exists()).toBe(true);
    expect(wrapper.find('.bar-btn[title="Mark as read"]').exists()).toBe(true);
    expect(wrapper.find('.bar-btn[title="Snooze"]').exists()).toBe(true);
    expect(wrapper.get('.bar-done').attributes('disabled')).toBeDefined();

    await wrapper.get('.bar-btn[title="More"]').trigger('click');
    expect(wrapper.get('.bar-btn[title="More"]').attributes('aria-haspopup')).toBe('dialog');
    expect(wrapper.get('.bar-btn[title="More"]').attributes('aria-expanded')).toBe('true');
    expect(wrapper.get('[role="dialog"]').attributes('aria-label')).toBe('More actions');
    expect(wrapper.get('.sheet-panel').text()).toContain('Unpin');
    expect(wrapper.get('.sheet-panel').text()).toContain('Back to the inbox');
    expect(wrapper.get('.sheet-panel').text()).toContain('Forward');
    expect(wrapper.get('.sheet-panel').text()).toContain('Delete');
  });

  it('closes the sheet before emitting an intent and keeps it across resize', async () => {
    const ui = useUiStore();
    const wrapper = mount(ThreadMobileActions, {
      props: baseProps,
      global: { stubs: { Transition: { template: '<slot />' } } },
    });

    await wrapper.get('.bar-btn[title="More"]').trigger('click');
    expect(ui.overlayDepth).toBe(1);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    await wrapper.setProps({ visible: false });
    expect(wrapper.find('.mobile-bar').exists()).toBe(false);
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    ui.isSinglePane = false;
    await nextTick();
    expect(wrapper.find('.sheet-root--anchored').exists()).toBe(true);

    await wrapper.setProps({ visible: true });
    expect(wrapper.find('[role="dialog"]').exists()).toBe(true);

    await wrapper.get('.sheet-item').trigger('click');
    await nextTick();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(ui.overlayDepth).toBe(0);
    expect(wrapper.emitted('togglePin')).toEqual([[]]);
  });

  it('hides reply actions when replying is unavailable and emits bar intents', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: { ...baseProps, canReply: false, isUnread: false },
    });

    expect(wrapper.find('.bar-btn[title="Reply"]').exists()).toBe(false);
    expect(wrapper.find('.bar-btn[title="Mark as unread"]').exists()).toBe(true);

    await wrapper.get('.bar-btn[title="Mark as unread"]').trigger('click');
    await wrapper.get('.bar-btn[title="Snooze"]').trigger('click');
    expect(wrapper.emitted('toggleRead')).toEqual([[]]);
    expect(wrapper.emitted('snooze')).toEqual([[]]);

    await wrapper.get('.bar-btn[title="More"]').trigger('click');
    expect(wrapper.get('.sheet-panel').text()).not.toContain('Forward');
  });

  it('emits the remaining bar and sheet intents and dismisses the overlay', async () => {
    const wrapper = mount(ThreadMobileActions, {
      props: baseProps,
      global: { stubs: { Transition: { template: '<slot />' } } },
    });
    const clickSheetAction = async (label: string) => {
      await wrapper.get('.bar-btn[title="More"]').trigger('click');
      const action = wrapper.findAll('.sheet-item').find((button) => button.text() === label);
      expect(action).toBeDefined();
      await action!.trigger('click');
      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    };

    await wrapper.get('.bar-btn[title="Reply"]').trigger('click');
    await wrapper.get('.bar-done').trigger('click');
    await clickSheetAction('Set aside');
    await clickSheetAction('Forward');
    await clickSheetAction('Delete');

    expect(wrapper.emitted('reply')).toEqual([[]]);
    expect(wrapper.emitted('archive')).toEqual([[]]);
    expect(wrapper.emitted('setAside')).toEqual([[]]);
    expect(wrapper.emitted('forward')).toEqual([[]]);
    expect(wrapper.emitted('delete')).toEqual([[]]);

    await wrapper.get('.bar-btn[title="More"]').trigger('click');
    await wrapper.get('.sheet-scrim').trigger('click');
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it('delegates drag dismissal to the shared bottom sheet', async () => {
    const ui = useUiStore();
    const wrapper = mount(ThreadMobileActions, {
      props: baseProps,
      global: { stubs: { Transition: { template: '<slot />' } } },
    });

    await wrapper.get('.bar-btn[title="More"]').trigger('click');
    const sheet = wrapper.get('[role="dialog"]');

    await sheet.trigger('pointerdown', {
      pointerType: 'mouse', pointerId: 1, button: 0, clientX: 160, clientY: 220,
    });
    await sheet.trigger('pointermove', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });
    await sheet.trigger('pointerup', {
      pointerType: 'mouse', pointerId: 1, clientX: 160, clientY: 320,
    });

    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(ui.overlayDepth).toBe(0);
  });
});
