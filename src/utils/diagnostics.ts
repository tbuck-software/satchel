import { backendRecordFrontendDiagnostic } from '@/services/backend/misc';

interface CacheStats {
  threadCount: number;
  messageCount: number;
  bodyChars: number;
}

interface MemoryPerformance extends Performance {
  memory?: {
    usedJSHeapSize?: number;
  };
}

export type DiagnosticLevel = 'info' | 'warn' | 'error';

export function recordFrontendDiagnostic(
  event: string,
  detail: string,
  cache?: CacheStats,
  level: DiagnosticLevel = 'info',
): void {
  const collectDom = event.startsWith('messages.') && typeof document !== 'undefined';
  const memory = (globalThis.performance as MemoryPerformance | undefined)?.memory;
  void backendRecordFrontendDiagnostic({
    event,
    detail,
    cacheThreads: cache?.threadCount ?? 0,
    cachedMessages: cache?.messageCount ?? 0,
    bodyChars: cache?.bodyChars ?? 0,
    domNodes: collectDom ? document.getElementsByTagName('*').length : 0,
    iframeCount: collectDom ? document.getElementsByTagName('iframe').length : 0,
    renderedMailCount: collectDom ? document.querySelectorAll('.satchel-email-document').length : 0,
    jsHeapBytes: memory?.usedJSHeapSize ?? 0,
    level,
  }).catch(() => {
    // Diagnostics must never interfere with mail navigation.
  });
}

export function installInteractionDiagnostics(): void {
  if (typeof document === 'undefined') {
    return;
  }

  document.addEventListener('click', (event) => {
    const target = event.target instanceof Element
      ? event.target.closest([
        'button',
        'a',
        'summary',
        'input',
        'select',
        'textarea',
        '[role="button"]',
        '[role="link"]',
        '[role="menuitem"]',
        '[role="tab"]',
      ].join(', '))
      : null;
    if (!target) {
      return;
    }

    const label = (
      target.getAttribute('aria-label')
      || target.getAttribute('title')
      || target.textContent
      || target.tagName.toLowerCase()
    ).replace(/\s+/g, ' ').trim().slice(0, 160);
    recordFrontendDiagnostic('ui.click', `label=${label || target.tagName.toLowerCase()}`);
  }, { capture: true });
}
