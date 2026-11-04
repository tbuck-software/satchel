import { describe, expect, it } from 'vitest';
import { OUTAGE_AFTER_MS, isOutage } from './syncFailures';

describe('isOutage', () => {
  const now = Date.parse('2026-11-04T12:00:00Z');

  it('starts exactly at the hour', () => {
    expect(isOutage({ since: new Date(now - OUTAGE_AFTER_MS + 1).toISOString() }, now)).toBe(false);
    expect(isOutage({ since: new Date(now - OUTAGE_AFTER_MS).toISOString() }, now)).toBe(true);
  });

  it('is never an outage without a readable start', () => {
    expect(isOutage({ since: null }, now)).toBe(false);
    expect(isOutage({ since: 'yesterday' }, now)).toBe(false);
  });
});
