// @vitest-environment jsdom

import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, nextTick, ref } from 'vue';
import ToastStack from '@/components/ui/ToastStack.vue';
import { useToast } from './useToast';
import { clearanceAbove, useToastClearance, type ClearanceRect } from './useToastClearance';

function rect(top: number, bottom: number, left = 0, right = 1000): ClearanceRect {
  return { top, bottom, left, right };
}

describe('clearanceAbove', () => {
  it('leaves a stack alone that covers nothing', () => {
    expect(clearanceAbove(rect(600, 650, 500, 900), [rect(100, 200)], 8)).toBe(0);
    expect(clearanceAbove(rect(600, 650, 500, 900), [rect(600, 650, 0, 400)], 8)).toBe(0);
  });

  it('rises above the surface it would cover, with a gap', () => {
    // Stack 600–650, send row from 620: the stack ends 8px above it.
    expect(clearanceAbove(rect(600, 650, 500, 900), [rect(620, 680)], 8)).toBe(38);
  });

  it('clears a second surface the first lift lands it on', () => {
    // A composer footer at 620 and the reply bar it sits over at 560.
    const lift = clearanceAbove(rect(600, 650, 500, 900), [rect(620, 680), rect(560, 610)], 8);
    expect(lift).toBe(98);
  });
});

describe('ToastStack with a registered surface', () => {
  const toast = useToast();

  beforeEach(() => {
    vi.stubGlobal('matchMedia', vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    // Asynchronous like the real one, just without waiting for a frame.
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      queueMicrotask(() => callback(0));
      return 1;
    });
  });

  afterEach(() => {
    toast.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  /** The composer's send row in the corner the stack rests in. */
  const SendRow = defineComponent({
    setup() {
      const el = ref<HTMLElement | null>(null);
      useToastClearance(el);
      return () => h('footer', { ref: el, class: 'send-row' });
    },
  });

  function box(left: number, top: number, width: number, height: number): DOMRect {
    const value = { left, top, width, height, x: left, y: top, right: left + width, bottom: top + height };
    return { ...value, toJSON: () => value } as DOMRect;
  }

  async function settle() {
    await nextTick();
    await nextTick();
    await nextTick();
  }

  it('lifts an error toast off the send row and drops back once it closes', async () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains('send-row')) return box(300, 640, 600, 60);
      if (this.classList.contains('stack')) return box(500, 610, 376, 66);
      return box(0, 0, 0, 0);
    });
    const wrapper = mount(defineComponent({ render: () => [h(SendRow), h(ToastStack)] }), {
      global: { plugins: [createPinia()] },
      attachTo: document.body,
    });
    const stack = () => wrapper.get('.stack').element as HTMLElement;

    toast.error('Senden fehlgeschlagen');
    await settle();
    // Stack bottom 676, send row top 640: 36px plus the 8px gap.
    expect(stack().style.getPropertyValue('--toast-clearance')).toBe('44px');

    toast.clear();
    await settle();
    expect(stack().style.getPropertyValue('--toast-clearance')).toBe('0px');
    wrapper.unmount();
  });
});
