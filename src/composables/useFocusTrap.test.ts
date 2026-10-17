// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { defineComponent, h, nextTick, ref } from 'vue';
import { mount } from '@vue/test-utils';
import { useFocusTrap } from './useFocusTrap';

const Overlay = defineComponent({
  props: {
    open: { type: Boolean, default: undefined },
    moveFocusIn: { type: Boolean, default: true },
    withAutofocus: { type: Boolean, default: false },
  },
  setup(props) {
    const panel = ref<HTMLElement | null>(null);
    const trap = useFocusTrap(panel, {
      isOpen: props.open === undefined ? undefined : () => props.open === true,
      moveFocusIn: () => props.moveFocusIn,
    });
    return () =>
      props.open === false
        ? null
        : h('div', { ref: panel, tabindex: -1, onKeydown: onKey }, [
            h('button', { class: 'first' }, 'first'),
            h('input', { class: 'middle', ...(props.withAutofocus ? { autofocus: true } : {}) }),
            h('button', { class: 'last' }, 'last'),
          ]);

    function onKey(event: KeyboardEvent) {
      if (event.key === 'Tab') trap.onTab(event);
    }
  },
});

/** Focus moves in a tick after the overlay renders. */
async function settled() {
  await nextTick();
  await nextTick();
}

function opener() {
  const button = document.createElement('button');
  document.body.append(button);
  button.focus();
  return button;
}

function tab(wrapper: ReturnType<typeof mount>, shiftKey = false) {
  return wrapper.find('div').trigger('keydown', { key: 'Tab', shiftKey });
}

describe('useFocusTrap', () => {
  it.each([
    '<button style="display:none">hidden</button>',
    '<div hidden><button>hidden</button></div>',
    '<button style="visibility:hidden">hidden</button>',
    '<fieldset disabled><button>disabled</button></fieldset>',
    '<div inert><button>inert</button></div>',
    '<input type="hidden">',
    '<button tabindex="-2">programmatic</button>',
  ])('skips unavailable controls in both directions: %s', async (markup) => {
    const wrapper = mount(Overlay, { attachTo: document.body });
    await settled();
    wrapper.find('.first').element.insertAdjacentHTML('afterend', markup);
    await tab(wrapper);
    expect(document.activeElement).toBe(wrapper.find('.middle').element);
    await tab(wrapper, true);
    expect(document.activeElement).toBe(wrapper.find('.first').element);
    wrapper.unmount();
  });

  it('ignores autofocus on a hidden field when opening', async () => {
    const open = ref(false);
    const component = defineComponent({
      setup() {
        const panel = ref<HTMLElement | null>(null);
        useFocusTrap(panel, { isOpen: () => open.value });
        return () => h('div', { ref: panel, tabindex: -1 }, [
          h('input', { autofocus: true, type: 'hidden' }),
          h('button', { class: 'visible' }, 'visible'),
        ]);
      },
    });
    const wrapper = mount(component, { attachTo: document.body });
    open.value = true;
    await settled();
    expect(document.activeElement).toBe(wrapper.find('.visible').element);
    wrapper.unmount();
  });

  it('moves focus to the first focusable element when the overlay appears', async () => {
    const wrapper = mount(Overlay, { attachTo: document.body });
    await settled();

    expect(document.activeElement).toBe(wrapper.find('.first').element);
    wrapper.unmount();
  });

  it('prefers a field that asked for focus over the first one', async () => {
    const wrapper = mount(Overlay, { attachTo: document.body, props: { withAutofocus: true } });
    await settled();

    expect(document.activeElement).toBe(wrapper.find('.middle').element);
    wrapper.unmount();
  });

  /** Tab past the last element lands on the first, not on the page behind. */
  it('cycles forwards and backwards inside the overlay', async () => {
    const wrapper = mount(Overlay, { attachTo: document.body });
    await settled();
    const first = wrapper.find('.first').element;
    const last = wrapper.find('.last').element as HTMLElement;

    last.focus();
    await tab(wrapper);
    expect(document.activeElement).toBe(first);

    await tab(wrapper, true);
    expect(document.activeElement).toBe(last);
    wrapper.unmount();
  });

  it('gives the focus back to whatever opened it', async () => {
    const trigger = opener();
    const wrapper = mount(Overlay, { attachTo: document.body });
    await settled();
    expect(document.activeElement).not.toBe(trigger);

    wrapper.unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it('gives it back when the overlay closes without unmounting', async () => {
    const trigger = opener();
    const wrapper = mount(Overlay, { attachTo: document.body, props: { open: true } });
    await settled();
    expect(document.activeElement).toBe(wrapper.find('.first').element);

    await wrapper.setProps({ open: false });
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
    wrapper.unmount();
  });

  /**
   * A sheet a finger opened must not pull the on-screen keyboard up with it —
   * but it still owes the focus back.
   */
  it('can skip moving focus in and still hand it back', async () => {
    const trigger = opener();
    const wrapper = mount(Overlay, { attachTo: document.body, props: { moveFocusIn: false } });
    await settled();
    expect(document.activeElement).toBe(trigger);

    wrapper.unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  /**
   * The recipient field answers Tab by turning the typed address into a chip.
   * A trap that moved focus on anyway would take the field away mid-entry.
   */
  it('leaves a Tab a field inside already answered alone', async () => {
    const wrapper = mount(Overlay, { attachTo: document.body });
    await settled();
    const last = wrapper.find('.last').element as HTMLElement;
    last.focus();
    last.addEventListener('keydown', (event) => event.preventDefault(), { once: true });

    last.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(last);
    wrapper.unmount();
  });

  it('brings focus back to the near end when it escaped the overlay', async () => {
    const outside = opener();
    const wrapper = mount(Overlay, { attachTo: document.body });
    await settled();

    outside.focus();
    await tab(wrapper);
    expect(document.activeElement).toBe(wrapper.find('.first').element);

    wrapper.unmount();
    outside.remove();
  });
});
