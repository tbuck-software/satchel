import { describe, expect, it, vi } from 'vitest';
import { createMobileResume, SERVER_REFRESH_COALESCE_MS, type MobileResumeDependencies } from './resume';

/*
 * ARCH-13. This used to be six regular expressions over `bootstrap.ts`. They
 * failed on a rename and passed a handler that no longer loaded anything, as
 * long as the text still read the same.
 */

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function harness(overrides: Partial<MobileResumeDependencies> = {}) {
  const calls: string[] = [];
  let clock = 10_000;
  const dependencies: MobileResumeDependencies = {
    isMobile: () => true,
    consumePendingNavigation: vi.fn(async () => { calls.push('navigation'); }),
    loadThreads: vi.fn(async () => { calls.push('threads'); }),
    activeThreadId: () => 'thread-1',
    loadMessages: vi.fn(async (threadId: string) => { calls.push(`messages:${threadId}`); }),
    loadSecondaryStores: vi.fn(() => {
      calls.push('secondary');
      return [];
    }),
    runServerSync: vi.fn(async () => { calls.push('sync'); }),
    now: () => clock,
    warn: vi.fn(),
    ...overrides,
  };
  return {
    calls,
    dependencies,
    resume: createMobileResume(dependencies),
    advance(ms: number) {
      clock += ms;
    },
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('mobile resume', () => {
  it('reloads the visible mail first and reconciles with the server behind it', async () => {
    const sync = deferred();
    const { calls, resume } = harness({
      runServerSync: vi.fn(() => sync.promise.then(() => { calls.push('sync'); })),
    });

    await resume.refreshMailState();
    expect(calls).toEqual(['navigation', 'threads', 'messages:thread-1', 'secondary']);

    // The network answers after the local reload has finished.
    sync.resolve();
    await flush();
    await flush();

    expect(calls.slice(4)).toEqual(['sync', 'navigation', 'threads', 'messages:thread-1', 'secondary']);
  });

  it('does not let a failed server sync stop the local reload', async () => {
    const warn = vi.fn();
    const { calls, resume } = harness({
      runServerSync: vi.fn(async () => { throw new Error('offline'); }),
      warn,
    });

    await resume.refreshMailState();
    await flush();

    expect(calls).toContain('threads');
    expect(warn).toHaveBeenCalledWith('mobile server sync failed', expect.any(Error));
  });

  it('keeps going when one of the visible reloads fails', async () => {
    const warn = vi.fn();
    const { calls, resume } = harness({
      loadThreads: vi.fn(async () => { throw new Error('database locked'); }),
      warn,
    });

    await resume.reloadMailStores();

    expect(calls).toContain('messages:thread-1');
    expect(calls).toContain('secondary');
    expect(warn).toHaveBeenCalledWith('mobile mailbox refresh failed', expect.any(Error));
  });

  it('coalesces the focus, visibility and resume signals Android fires together', async () => {
    const gate = deferred();
    const loadThreads = vi.fn(() => gate.promise);
    const runServerSync = vi.fn(async () => {});
    const { resume } = harness({ loadThreads, runServerSync });

    const first = resume.refreshMailState();
    const second = resume.refreshMailState();
    const third = resume.refreshMailState();
    gate.resolve();
    await Promise.all([first, second, third]);

    expect(loadThreads).toHaveBeenCalledTimes(1);
    expect(runServerSync).toHaveBeenCalledTimes(1);
  });

  it('syncs with the server again once the coalescing window has passed', async () => {
    const runServerSync = vi.fn(async () => {});
    const { resume, advance } = harness({ runServerSync });

    await resume.refreshMailState();
    await flush();
    advance(SERVER_REFRESH_COALESCE_MS);
    await resume.refreshMailState();
    await flush();

    expect(runServerSync).toHaveBeenCalledTimes(2);
  });

  it('only follows a pending notification outside the mobile shell', async () => {
    const { calls, resume, dependencies } = harness({ isMobile: () => false });

    await resume.refreshMailState();

    expect(calls).toEqual(['navigation']);
    expect(dependencies.runServerSync).not.toHaveBeenCalled();
  });

  /**
   * The failure the regex tests let through: a handler that still reads right
   * and never touches the list. The ordering check above is a function of what
   * actually ran, so the same check rejects it.
   */
  it('rejects a resume path that no longer reloads the thread list', async () => {
    const expected = ['navigation', 'threads', 'messages:thread-1', 'secondary'];
    const working = harness();
    const broken = harness();
    const skipsTheList = createMobileResume({
      ...broken.dependencies,
      loadThreads: () => Promise.resolve(),
    });

    await working.resume.refreshMailState();
    await skipsTheList.refreshMailState();

    const visible = (calls: string[]) => calls.filter((call) => call !== 'sync').slice(0, 4);
    expect(visible(working.calls)).toEqual(expected);
    expect(visible(broken.calls)).not.toEqual(expected);
  });
});
