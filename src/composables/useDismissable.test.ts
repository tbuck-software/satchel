// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { defineComponent, h, ref } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { useDismissable } from './useDismissable';
import { useUiStore } from '@/stores/ui';

const Overlay = defineComponent({
  props: { open: { type: Boolean, default: true } },
  setup(props) {
    const closed = ref(false);
    useDismissable(
      () => props.open && !closed.value,
      () => {
        closed.value = true;
      },
    );
    return () => h('div', closed.value ? 'zu' : 'offen');
  },
});

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('an overlay on the back stack', () => {
  it('counts towards the depth while it is on screen', async () => {
    const ui = useUiStore();
    expect(ui.overlayDepth).toBe(0);

    const wrapper = mount(Overlay);
    expect(ui.overlayDepth).toBe(1);

    ui.dismissTopmostOverlay();
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).toBe('zu');
    expect(ui.overlayDepth).toBe(0);
  });

  /**
   * The parent often mounts these only while they are open, so unmounting is
   * the normal way one goes away. A registration that outlived its component
   * would keep the gesture pointed at something no longer there.
   */
  it('stops counting once its component is gone', () => {
    const ui = useUiStore();
    const wrapper = mount(Overlay);
    expect(ui.overlayDepth).toBe(1);

    wrapper.unmount();
    expect(ui.overlayDepth).toBe(0);
  });

  it('does not count while closed', () => {
    const ui = useUiStore();
    mount(Overlay, { props: { open: false } });
    expect(ui.overlayDepth).toBe(0);
    expect(ui.dismissTopmostOverlay()).toBe(false);
  });

  /**
   * Two open at once — a link warning raised from inside the composer — must
   * cost one press each, innermost first.
   */
  it('dismisses the most recently registered first', async () => {
    const ui = useUiStore();
    const first = mount(Overlay);
    const second = mount(Overlay);
    expect(ui.overlayDepth).toBe(2);

    ui.dismissTopmostOverlay();
    await second.vm.$nextTick();
    expect(second.text()).toBe('zu');
    expect(first.text()).toBe('offen');
  });
});
