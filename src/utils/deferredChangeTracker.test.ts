import { effectScope, nextTick, ref } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { deferredChangeTracker } from './deferredChangeTracker';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function tracked() {
  const text = ref('a');
  const source = vi.fn(() => text.value.toUpperCase());
  const onChange = vi.fn();
  const scope = effectScope();
  const tracker = scope.run(() => deferredChangeTracker(source, onChange, 100))!;
  return { text, source, onChange, tracker, scope };
}

describe('deferredChangeTracker', () => {
  it('evaluates the source once per pause, not once per change', async () => {
    const { text, source, onChange } = tracked();
    expect(source).toHaveBeenCalledTimes(1);

    for (const value of ['ab', 'abc', 'abcd', 'abcde']) {
      text.value = value;
      await nextTick();
      vi.advanceTimersByTime(50);
    }
    expect(source).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(100);
    expect(source).toHaveBeenCalledTimes(2);
    expect(onChange).toHaveBeenCalledExactlyOnceWith('ABCDE');
  });

  it('answers at once when settled and stays quiet for an unchanged value', () => {
    const { text, onChange, tracker } = tracked();
    text.value = 'b';
    tracker.settle();
    expect(onChange).toHaveBeenCalledExactlyOnceWith('B');

    text.value = 'x';
    text.value = 'b';
    tracker.settle();
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it('absorbs pending changes on rebaseline and stops with its scope', () => {
    const { text, source, onChange, tracker, scope } = tracked();
    text.value = 'b';
    tracker.rebaseline();
    vi.advanceTimersByTime(200);
    expect(onChange).not.toHaveBeenCalled();
    expect(tracker.current()).toBe('B');

    scope.stop();
    const calls = source.mock.calls.length;
    text.value = 'c';
    vi.advanceTimersByTime(200);
    expect(source).toHaveBeenCalledTimes(calls);
  });
});
