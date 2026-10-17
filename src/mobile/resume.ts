/**
 * What the mobile shell does when it comes back to the foreground.
 *
 * A background sync can move messages while the WebView is frozen. A head
 * refresh revalidates those cached rows too, preserving the paging cursor.
 * Android fires focus, visibility and resume together, hence the coalescing.
 *
 * Lives outside `bootstrap.ts` so it can be run with the stores it needs and
 * asserted on, rather than grepped for.
 */
export interface MobileResumeDependencies {
  isMobile(): boolean;
  consumePendingNavigation(): Promise<void>;
  loadThreads(): Promise<unknown>;
  activeThreadId(): string | null | undefined;
  loadMessages(threadId: string): Promise<unknown>;
  /** Sidebar counts and connection metadata: useful, never worth waiting for. */
  loadSecondaryStores(): Promise<unknown>[];
  runServerSync(): Promise<unknown>;
  now?: () => number;
  warn?: (message: string, error: unknown) => void;
}

export interface MobileResume {
  /** Replace the visible mailbox projection from the local database. */
  reloadMailStores(): Promise<void>;
  /** The resume handler: local reload now, server reconciliation behind it. */
  refreshMailState(): Promise<void>;
}

/** How long a burst of resume signals counts as one. */
export const SERVER_REFRESH_COALESCE_MS = 1_000;

export function createMobileResume(dependencies: MobileResumeDependencies): MobileResume {
  const now = dependencies.now ?? Date.now;
  const warn = dependencies.warn ?? ((message, error) => console.warn(message, error));
  let reloadInFlight: Promise<void> | null = null;
  let serverRefreshInFlight: Promise<void> | null = null;
  let lastServerRefreshAt = Number.NEGATIVE_INFINITY;

  function reloadMailStores(): Promise<void> {
    if (reloadInFlight) return reloadInFlight;
    reloadInFlight = (async () => {
      const visibleReloads: Promise<unknown>[] = [
        dependencies.consumePendingNavigation(),
        dependencies.loadThreads(),
      ];
      const activeThreadId = dependencies.activeThreadId();
      if (activeThreadId) visibleReloads.push(dependencies.loadMessages(activeThreadId));
      for (const result of await Promise.allSettled(visibleReloads)) {
        if (result.status === 'rejected') warn('mobile mailbox refresh failed', result.reason);
      }
      void Promise.allSettled(dependencies.loadSecondaryStores());
    })().finally(() => {
      reloadInFlight = null;
    });
    return reloadInFlight;
  }

  function refreshMailState(): Promise<void> {
    if (!dependencies.isMobile()) return dependencies.consumePendingNavigation();

    const immediateReload = reloadMailStores();
    const at = now();
    if (at - lastServerRefreshAt >= SERVER_REFRESH_COALESCE_MS && !serverRefreshInFlight) {
      lastServerRefreshAt = at;
      // The local reload above is the interactive path and never waits on the
      // network. Cross-device changes arrive through the sync, after which the
      // list is replaced once more from the resulting local projection.
      serverRefreshInFlight = dependencies.runServerSync()
        .then(() => reloadMailStores())
        .catch((error) => warn('mobile server sync failed', error))
        .finally(() => {
          serverRefreshInFlight = null;
        });
    }
    return immediateReload;
  }

  return { reloadMailStores, refreshMailState };
}
