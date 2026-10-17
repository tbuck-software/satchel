import { afterEach, describe, expect, it, vi } from 'vitest';
import { perfEnabled, perfLog, perfStart } from './perf';

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('perf helpers', () => {
  it('reports disabled when no window is available', () => {
    expect(perfEnabled()).toBe(false);
    expect(perfStart('mailbox.load')).toBeNull();
  });

  it('reads the perf flag from localStorage', () => {
    const getItem = vi.fn().mockReturnValue('1');
    vi.stubGlobal('window', {
      localStorage: { getItem },
    });

    expect(perfEnabled()).toBe(true);
    expect(getItem).toHaveBeenCalledWith('satchel:perf');
  });

  it('logs start, end, and detail output when enabled', () => {
    vi.stubGlobal('window', {
      localStorage: { getItem: vi.fn().mockReturnValue('1') },
    });
    vi.spyOn(performance, 'now').mockReturnValueOnce(10).mockReturnValueOnce(27.4);
    const infoSpy = vi.spyOn(console, 'info').mockImplementation(() => undefined);

    const end = perfStart('thread.open');
    expect(end).not.toBeNull();
    end?.();
    perfLog('thread.open', 'hydrated');

    expect(infoSpy).toHaveBeenNthCalledWith(1, '[perf:start] thread.open');
    expect(infoSpy).toHaveBeenNthCalledWith(2, '[perf:end] thread.open 17.4ms');
    expect(infoSpy).toHaveBeenNthCalledWith(3, '[perf] thread.open hydrated');
  });
});
