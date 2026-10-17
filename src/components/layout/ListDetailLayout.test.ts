// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import ListDetailLayout from './ListDetailLayout.vue';
import { useUiStore } from '@/stores/ui';

function render(hasDetail: boolean, singlePane: boolean) {
  useUiStore().isSinglePane = singlePane;
  return mount(ListDetailLayout, {
    props: { hasDetail },
    slots: { list: 'LIST', detail: 'DETAIL', empty: 'EMPTY' },
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('list and detail by width', () => {
  it('shows both panes and the resize handle when there is room', () => {
    const wrapper = render(true, false);
    expect(wrapper.text()).toContain('LIST');
    expect(wrapper.text()).toContain('DETAIL');
    expect(wrapper.find('.split-layout__resize').exists()).toBe(true);
  });

  /**
   * The empty pane stays rather than the layout collapsing to one column —
   * otherwise selecting the first entry would shift the list sideways under
   * the pointer.
   */
  it('keeps the detail pane with nothing selected on a wide screen', () => {
    const wrapper = render(false, false);
    expect(wrapper.text()).toContain('LIST');
    expect(wrapper.text()).toContain('EMPTY');
    expect(wrapper.text()).not.toContain('DETAIL');
  });

  it('shows only the list when narrow and nothing is selected', () => {
    const wrapper = render(false, true);
    expect(wrapper.text()).toContain('LIST');
    expect(wrapper.text()).not.toContain('DETAIL');
  });

  /**
   * The whole point: narrow, the detail *replaces* the list. It is a separate
   * screen, so leaving it is an ordinary back step.
   */
  it('replaces the list with the detail when narrow', () => {
    const wrapper = render(true, true);
    expect(wrapper.text()).toContain('DETAIL');
    expect(wrapper.get('.split-layout__list').isVisible()).toBe(false);
  });

  it('preserves list state when returning from a narrow detail screen', async () => {
    const wrapper = render(false, true);
    const list = wrapper.get('.split-layout__list').element;
    await wrapper.setProps({ hasDetail: true });
    expect(wrapper.get('.split-layout__list').element).toBe(list);
    await wrapper.setProps({ hasDetail: false });
    expect(wrapper.get('.split-layout__list').element).toBe(list);
    expect(wrapper.get('.split-layout__list').isVisible()).toBe(true);
  });

  it('drops the resize handle when there is nothing beside the pane', () => {
    const wrapper = render(true, true);
    expect(wrapper.find('.split-layout__resize').exists()).toBe(false);
  });

  /**
   * Callers that always have a detail — the settings modal — must keep both
   * panes on a wide screen without passing anything.
   */
  it('defaults to having a detail', () => {
    useUiStore().isSinglePane = false;
    const wrapper = mount(ListDetailLayout, { slots: { list: 'LIST', detail: 'DETAIL' } });
    expect(wrapper.text()).toContain('DETAIL');
  });
});
