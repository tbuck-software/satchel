// @vitest-environment jsdom

import { describe, expect, it } from 'vitest';
import { installKeyboardInset, keyboardInset } from './keyboardInset';

describe('keyboardInset', () => {
  it('measures what the keyboard covers below the visual viewport', () => {
    expect(keyboardInset(852, { height: 516, offsetTop: 0, scale: 1 })).toBe(336);
    expect(keyboardInset(852, { height: 516, offsetTop: 100, scale: 1 })).toBe(236);
  });

  it('ignores toolbars, rounding and pinch zoom', () => {
    expect(keyboardInset(852, { height: 852, offsetTop: 0, scale: 1 })).toBe(0);
    expect(keyboardInset(852, { height: 800, offsetTop: 0, scale: 1 })).toBe(0);
    expect(keyboardInset(852, { height: 400, offsetTop: 0, scale: 2 })).toBe(0);
  });
});

describe('installKeyboardInset', () => {
  function fakeWindow() {
    const viewport = Object.assign(new EventTarget(), { height: 852, offsetTop: 0, scale: 1 });
    const root = document.createElement('html');
    const win = {
      visualViewport: viewport,
      innerHeight: 852,
      scrollY: 0,
      scrollTo: () => {},
      document: { documentElement: root },
    } as unknown as Window;
    return { win, viewport, root };
  }

  it('turns the keyboard into the bottom safe area and back', () => {
    const { win, viewport, root } = fakeWindow();
    const dispose = installKeyboardInset(win);
    expect(root.style.getPropertyValue('--satchel-safe-area-bottom')).toBe('');

    viewport.height = 516;
    viewport.dispatchEvent(new Event('resize'));
    expect(root.style.getPropertyValue('--satchel-safe-area-bottom')).toBe('336px');
    expect(root.dataset.keyboard).toBe('open');

    viewport.height = 852;
    viewport.dispatchEvent(new Event('resize'));
    expect(root.style.getPropertyValue('--satchel-safe-area-bottom')).toBe('');
    expect(root.dataset.keyboard).toBeUndefined();
    dispose();
  });
});
