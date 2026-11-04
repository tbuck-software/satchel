import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { formatMessageHeaderTime } from './messageHeaderTime';

describe('formatMessageHeaderTime', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 27, 10, 30));
  });
  afterEach(() => vi.useRealTimers());

  it('writes only the time for today', () => {
    expect(formatMessageHeaderTime(new Date(2026, 9, 27, 8, 5).toISOString())).toMatch(/^08:05$/);
  });

  it('names yesterday', () => {
    expect(formatMessageHeaderTime(new Date(2026, 9, 26, 21, 40).toISOString())).toMatch(/, 21:40$/);
    expect(formatMessageHeaderTime(new Date(2026, 9, 26, 21, 40).toISOString())).not.toMatch(/26/);
  });

  it('adds the date this year and the year before that', () => {
    expect(formatMessageHeaderTime(new Date(2026, 2, 3, 9, 0).toISOString())).toMatch(/3/);
    expect(formatMessageHeaderTime(new Date(2025, 2, 3, 9, 0).toISOString())).toMatch(/2025/);
  });
});
