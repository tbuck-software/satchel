import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useToast } from './useToast';

const toast = useToast();

beforeEach(() => {
  // Date has to be faked explicitly: the expiry check reads Date.now(), and
  // advancing only the interval would run the ticker against a standing clock.
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'setTimeout', 'clearTimeout', 'Date'] });
});

afterEach(() => {
  toast.clear();
  vi.useRealTimers();
});

describe('useToast', () => {
  it('gives every toast its own id, even within the same millisecond', () => {
    const first = toast.info('eins');
    const second = toast.info('zwei');
    expect(first).not.toBe(second);
    expect(toast.state.toasts).toHaveLength(2);
  });

  it('drops a toast once its timeout has passed', () => {
    toast.info('kurz', { timeout: 1000 });
    vi.advanceTimersByTime(1500);
    expect(toast.state.toasts).toHaveLength(0);
  });

  it('keeps a persistent toast alive', () => {
    toast.error('bleibt', { timeout: 0 });
    vi.advanceTimersByTime(60_000);
    expect(toast.state.toasts).toHaveLength(1);
  });

  it('stops the countdown while paused', () => {
    const id = toast.info('lesen', { timeout: 1000 });
    toast.pause(id);
    vi.advanceTimersByTime(5000);
    expect(toast.state.toasts).toHaveLength(1);
  });

  it('keeps actionable undo on a hard wall-clock deadline', () => {
    const id = toast.success('Erledigt', {
      timeout: 1000,
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    toast.pause(id);
    vi.advanceTimersByTime(1500);
    expect(toast.state.toasts).toHaveLength(0);
  });

  it('keeps independent undo operations independent', () => {
    toast.success('Erledigt', {
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    toast.success('Beiseitegelegt', {
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    expect(toast.state.toasts).toHaveLength(2);
  });

  it('dismisses one toast group without touching unrelated notices', () => {
    toast.error('Outbox 1', { timeout: 0, groupKey: 'outbox' });
    toast.error('Outbox 2', { timeout: 0, groupKey: 'outbox' });
    toast.info('Andere Meldung');

    toast.dismissGroup('outbox');

    expect(toast.state.toasts.map((item) => item.message)).toEqual(['Andere Meldung']);
  });

  it('gives back the remaining time on resume rather than restarting', () => {
    const id = toast.info('lesen', { timeout: 1000 });
    vi.advanceTimersByTime(600);
    toast.pause(id);
    vi.advanceTimersByTime(10_000);
    toast.resume(id);

    // 400ms were left; 300 is not enough to expire it.
    vi.advanceTimersByTime(300);
    expect(toast.state.toasts).toHaveLength(1);

    vi.advanceTimersByTime(400);
    expect(toast.state.toasts).toHaveLength(0);
  });

  it('merges an equivalent grouped notice instead of stacking it', () => {
    const groupMessage = (count: number) => `Outbox (${count})`;
    toast.error('Outbox', { groupKey: 'outbox', count: 1, groupMessage });
    const first = toast.state.toasts[0]!.expiresAt;
    vi.advanceTimersByTime(1000);
    toast.error('Outbox', { groupKey: 'outbox', count: 1, groupMessage });

    expect(toast.state.toasts).toHaveLength(1);
    expect(toast.state.toasts[0]).toMatchObject({ count: 2, message: 'Outbox (2)' });
    // A pile-up of notices keeps the newest deadline.
    expect(toast.state.toasts[0]!.expiresAt - first).toBe(1000);
  });

  it('holds a merged undo group to the earliest deadline of its sources', () => {
    const action = () => ({
      label: 'Rückgängig',
      handler: vi.fn(),
      batch: { key: 'workflow:restore', items: ['a'], handler: vi.fn() },
    });
    toast.success('Erledigt', { groupKey: 'workflow:done', count: 1, timeout: 9000, action: action() });
    const first = toast.state.toasts[0]!.expiresAt;
    vi.advanceTimersByTime(1000);
    toast.success('Erledigt', { groupKey: 'workflow:done', count: 1, timeout: 9000, action: action() });

    expect(toast.state.toasts).toHaveLength(1);
    expect(toast.state.toasts[0]!.count).toBe(2);
    // Undo stops working once the backend picks the queued move up, so the
    // group must not push its deadline out.
    expect(toast.state.toasts[0]!.expiresAt).toBe(first);
    expect(toast.state.toasts[0]!.action?.batch?.items).toEqual(['a', 'a']);
  });

  it('keeps a grouped notice apart from an unrelated undo payload', () => {
    toast.success('Erledigt', {
      groupKey: 'workflow:done',
      action: { label: 'Rückgängig', handler: vi.fn() },
    });
    toast.success('Erledigt', {
      groupKey: 'workflow:done',
      action: {
        label: 'Rückgängig',
        handler: vi.fn(),
        batch: { key: 'workflow:restore', items: ['a'], handler: vi.fn() },
      },
    });

    // Merging would drop one of the two handlers without a trace.
    expect(toast.state.toasts).toHaveLength(2);
  });

  it('errors stay up longer than the rest by default', () => {
    const info = toast.state.toasts;
    toast.info('kurz');
    toast.error('lang');
    const [first, second] = info;
    expect(second!.timeout).toBeGreaterThan(first!.timeout);
  });
});
