import type { App } from 'vue';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';
import { recordFrontendDiagnostic } from '@/utils/diagnostics';

/**
 * The one place an error nobody caught ends up: Vue's `errorHandler`, an
 * unhandled rejection or an uncaught `error` event. Every one is written to the
 * log; the user sees at most one toast per {@link TOAST_INTERVAL_MS}.
 */

export type FrontendErrorSource = 'vue' | 'unhandledrejection' | 'error';

export const TOAST_INTERVAL_MS = 30_000;
const MAX_DETAIL_CHARS = 2_000;
const MAX_STACK_FRAMES = 8;

export interface FrontendErrorReporterOptions {
  record: (event: string, detail: string) => void;
  notify: () => void;
  now?: () => number;
}

/** Name, message and the top of the stack; engines disagree on whether the stack repeats the message. */
export function describeError(error: unknown): string {
  if (error instanceof Error) {
    const head = `${error.name}: ${error.message}`;
    const frames = (error.stack ?? '')
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line && line !== head)
      .slice(0, MAX_STACK_FRAMES);
    return [head, ...frames].join('\n');
  }
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error) ?? String(error);
  } catch {
    return String(error);
  }
}

/** Intentional cancellations and a layout warning browsers raise as an error. */
function isBenign(error: unknown): boolean {
  if (error instanceof DOMException && error.name === 'AbortError') return true;
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return message.startsWith('ResizeObserver loop');
}

export function createFrontendErrorReporter(options: FrontendErrorReporterOptions) {
  const now = options.now ?? Date.now;
  let lastToastAt = Number.NEGATIVE_INFINITY;

  return function report(error: unknown, source: FrontendErrorSource, context?: string): void {
    if (isBenign(error)) return;
    const description = describeError(error);
    const detail = (context ? `${context}: ${description}` : description).slice(0, MAX_DETAIL_CHARS);
    options.record(`error.${source}`, detail);
    const at = now();
    if (at - lastToastAt < TOAST_INTERVAL_MS) return;
    lastToastAt = at;
    options.notify();
  };
}

export interface GlobalErrorHandlerOptions {
  /** Offered as the toast's action; the log entry is what a bug report needs. */
  openDiagnostics?: () => unknown;
  target?: Pick<Window, 'addEventListener'>;
}

export function installGlobalErrorHandlers(app: App, options: GlobalErrorHandlerOptions = {}) {
  const toast = useToast();
  const report = createFrontendErrorReporter({
    record: (event, detail) => recordFrontendDiagnostic(event, detail, undefined, 'error'),
    notify: () => {
      const openDiagnostics = options.openDiagnostics;
      toast.error(t('app.errors.unexpected'), openDiagnostics
        ? { action: { label: t('app.errors.showDiagnostics'), handler: openDiagnostics } }
        : {});
    },
  });

  app.config.errorHandler = (error, _instance, info) => {
    // Setting a handler silences Vue's own console output.
    console.error(error);
    report(error, 'vue', info);
  };

  const target = options.target ?? (typeof window === 'undefined' ? null : window);
  target?.addEventListener('unhandledrejection', (event) => {
    report((event as PromiseRejectionEvent).reason, 'unhandledrejection');
  });
  target?.addEventListener('error', (event) => {
    const { error, message } = event as ErrorEvent;
    report(error ?? message, 'error');
  });
  return report;
}
