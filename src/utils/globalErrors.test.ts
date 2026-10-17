// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApp, h } from 'vue';

const recorded = vi.hoisted(() => [] as Array<{ event: string; detail: string; level?: string }>);
vi.mock('@/utils/diagnostics', () => ({
  recordFrontendDiagnostic: (event: string, detail: string, _cache: unknown, level?: string) => {
    recorded.push({ event, detail, level });
  },
}));

import { useToast } from '@/composables/useToast';
import {
  TOAST_INTERVAL_MS,
  createFrontendErrorReporter,
  describeError,
  installGlobalErrorHandlers,
} from './globalErrors';

afterEach(() => {
  recorded.length = 0;
  useToast().clear();
  vi.restoreAllMocks();
});

describe('createFrontendErrorReporter', () => {
  it('records every error but toasts at most once per interval', () => {
    let now = 1_000;
    const record = vi.fn();
    const notify = vi.fn();
    const report = createFrontendErrorReporter({ record, notify, now: () => now });

    report(new Error('first'), 'unhandledrejection');
    report(new Error('second'), 'error');
    now += TOAST_INTERVAL_MS - 1;
    report(new Error('third'), 'vue', 'render function');

    expect(record).toHaveBeenCalledTimes(3);
    expect(record.mock.calls[2]).toEqual([
      'error.vue',
      expect.stringMatching(/^render function: Error: third/),
    ]);
    expect(notify).toHaveBeenCalledTimes(1);

    now += 1;
    report('fourth', 'error');
    expect(notify).toHaveBeenCalledTimes(2);
  });

  it('ignores cancellations and the ResizeObserver loop warning', () => {
    const record = vi.fn();
    const notify = vi.fn();
    const report = createFrontendErrorReporter({ record, notify });

    report(new DOMException('aborted', 'AbortError'), 'unhandledrejection');
    report('ResizeObserver loop completed with undelivered notifications.', 'error');

    expect(record).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });
});

describe('describeError', () => {
  it('keeps name, message and the top of the stack once', () => {
    const error = new TypeError('x is undefined');
    error.stack = 'TypeError: x is undefined\n    at a (app.js:1:1)\n    at b (app.js:2:2)';
    expect(describeError(error)).toBe('TypeError: x is undefined\nat a (app.js:1:1)\nat b (app.js:2:2)');
    expect(describeError({ code: 7 })).toBe('{"code":7}');
  });
});

describe('installGlobalErrorHandlers', () => {
  it('routes Vue errors, rejections and uncaught errors into the log and one toast', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const target = new EventTarget();
    const openDiagnostics = vi.fn();
    const app = createApp({
      render: () => h('div', [h({ setup: () => { throw new Error('render failed'); }, render: () => null })]),
    });
    installGlobalErrorHandlers(app, { openDiagnostics, target: target as unknown as Window });

    app.mount(document.createElement('div'));
    target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: new Error('lost promise') }));
    target.dispatchEvent(Object.assign(new Event('error'), { error: new Error('uncaught'), message: 'uncaught' }));

    expect(recorded.map((entry) => entry.event)).toEqual([
      'error.vue',
      'error.unhandledrejection',
      'error.error',
    ]);
    expect(recorded.every((entry) => entry.level === 'error')).toBe(true);
    expect(recorded[1]?.detail).toContain('lost promise');

    const toasts = useToast().state.toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0]?.tone).toBe('error');
    void toasts[0]?.action?.handler();
    expect(openDiagnostics).toHaveBeenCalledOnce();
  });
});
