// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import SplitButton from './SplitButton.vue';

function render(disabled = false) {
  return mount(SplitButton, {
    props: {
      secondaryLabel: 'Send later…', expanded: false, popup: 'dialog', disabled,
      descriptionId: 'blocked-hint',
      shortcut: { label: 'Ctrl+Enter', keys: ['Ctrl', 'Enter'], aria: 'Control+Enter' },
    },
    slots: { default: 'Send', icon: '<svg aria-hidden="true" />' },
  });
}

describe('SplitButton', () => {
  it('provides two separately focusable buttons with a named popup trigger', async () => {
    const wrapper = render();
    const [primary, secondary] = wrapper.findAll('button');
    expect(primary!.text()).toBe('Send');
    expect(primary!.attributes('aria-keyshortcuts')).toBe('Control+Enter');
    expect(primary!.find('svg').exists()).toBe(true);
    expect(secondary!.attributes()).toMatchObject({
      type: 'button', 'aria-label': 'Send later…',
      'aria-haspopup': 'dialog', 'aria-expanded': 'false',
    });
    expect(secondary!.attributes('aria-keyshortcuts')).toBeUndefined();
    for (const button of [primary!, secondary!]) {
      expect(button.attributes('tabindex')).toBeUndefined();
      expect(button.attributes('aria-describedby')).toBe('blocked-hint');
    }
    await wrapper.setProps({ expanded: true, controls: 'picker' });
    expect(secondary!.attributes('aria-expanded')).toBe('true');
    expect(secondary!.attributes('aria-controls')).toBe('picker');
  });

  it('routes the primary action and popup action independently', async () => {
    const wrapper = render();
    const [primary, secondary] = wrapper.findAll('button');
    await primary!.trigger('click');
    expect(wrapper.emitted('click')).toHaveLength(1);
    expect(wrapper.emitted('open')).toBeUndefined();
    await secondary!.trigger('click');
    await secondary!.trigger('keydown', { key: 'ArrowDown' });
    await secondary!.trigger('keydown', { key: 'ArrowUp' });
    expect(wrapper.emitted('open')).toHaveLength(3);
    expect(wrapper.emitted('open')![0]![0]).toBeInstanceOf(MouseEvent);
    expect(wrapper.emitted('open')![1]![0]).toBeInstanceOf(KeyboardEvent);
    expect(wrapper.emitted('click')).toHaveLength(1);
  });

  it('disables both actions and refuses popup keyboard activation', async () => {
    const wrapper = render(true);
    for (const button of wrapper.findAll('button')) {
      expect(button.attributes('disabled')).toBeDefined();
      await button.trigger('click');
      await button.trigger('keydown', { key: 'ArrowDown' });
    }
    expect(wrapper.emitted('click')).toBeUndefined();
    expect(wrapper.emitted('open')).toBeUndefined();
    await wrapper.setProps({ disabled: false });
    await wrapper.findAll('button')[1]!.trigger('click');
    expect(wrapper.emitted('open')).toHaveLength(1);
  });
});


it('allows repairing a blocked primary action through an enabled popup', async () => {
  const wrapper = render(true);
  await wrapper.setProps({ secondaryDisabled: false });
  const [primary, secondary] = wrapper.findAll('button');
  expect(primary!.attributes('disabled')).toBeDefined();
  expect(secondary!.attributes('disabled')).toBeUndefined();
  await secondary!.trigger('click');
  await secondary!.trigger('keydown', { key: 'ArrowDown' });
  expect(wrapper.emitted('open')).toHaveLength(2);
  expect(wrapper.emitted('click')).toBeUndefined();
  wrapper.unmount();
});
