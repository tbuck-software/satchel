// @vitest-environment jsdom
import { afterEach, expect, it } from 'vitest';
import { measureToastPlacement } from './toastPlacement';

function element(className: string, left: number, top: number, width: number, height: number) {
  const el = document.createElement('div');
  el.className = className;
  el.getBoundingClientRect = () => ({ left, top, width, height, right: left + width,
    bottom: top + height, x: left, y: top, toJSON: () => ({}) });
  document.body.append(el);
  return el;
}
afterEach(() => { document.body.innerHTML = ''; });

it('wraps into the free corner of a narrower desktop before reserving canvas space', () => {
  element('composer-window', 322, 32, 760, 650);
  const initial = measureToastPlacement(element('stack', 24, 560, 440, 116),
    { clearance: 0, reserved: false, availableWidth: null }, 8);
  expect(initial).toEqual({ clearance: 0, reserved: false, availableWidth: 290, remeasure: true });
  const wrapped = measureToastPlacement(element('stack', 24, 480, 290, 196), initial, 8);
  expect(wrapped.reserved).toBe(false);
  expect(wrapped.clearance).toBe(0);
});

it('releases an earlier reservation once the outside corner has enough room', () => {
  element('composer-window', 322, 138, 760, 544);
  const next = measureToastPlacement(element('stack', 24, 8, 440, 116),
    { clearance: 0, reserved: true, availableWidth: null }, 8);
  expect(next.reserved).toBe(false);
  expect(next.availableWidth).toBe(290);
});

it('reserves a lane above a full-screen composer', () => {
  element('composer-window composer-window--maximized', 0, 0, 390, 844);
  const next = measureToastPlacement(element('stack', 12, 680, 366, 72),
    { clearance: 0, reserved: false, availableWidth: null }, 8);
  expect(next.reserved).toBe(true);
  expect(next.clearance).toBe(0);
});
