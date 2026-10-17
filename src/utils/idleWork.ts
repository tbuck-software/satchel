/**
 * Run non-critical preparation after the browser has painted interactive UI.
 * The timeout fallback covers WebViews without requestIdleCallback.
 */
export function scheduleIdleWork(task: () => void, timeout = 2_500): () => void {
  const idleWindow = window as typeof window & {
    requestIdleCallback?: (callback: () => void, options: { timeout: number }) => number;
    cancelIdleCallback?: (handle: number) => void;
  };

  if (idleWindow.requestIdleCallback) {
    const handle = idleWindow.requestIdleCallback(task, { timeout });
    return () => idleWindow.cancelIdleCallback?.(handle);
  }

  const handle = window.setTimeout(task, Math.min(timeout, 600));
  return () => window.clearTimeout(handle);
}
