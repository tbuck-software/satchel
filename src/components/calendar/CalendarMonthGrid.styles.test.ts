import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/*
 * The month grid's layout lives in CSS, which jsdom does not lay out. Two
 * decisions are easy to undo without any test noticing, so they are pinned at
 * the source: the weekday header and the day number share one axis, and an
 * event chip never wraps — a two-line chip makes its week row taller.
 */

const read = (path: string) => readFileSync(join(process.cwd(), path), 'utf8');

/** The body of the rule whose whole selector is `selector`. */
function rule(source: string, selector: string): string {
  const start = source.indexOf(`\n${selector} {`);
  expect(start, `${selector} is styled`).toBeGreaterThanOrEqual(0);
  return source.slice(start, source.indexOf('}', start));
}

describe('CalendarMonthGrid styles', () => {
  const grid = read('src/components/calendar/CalendarMonthGrid.vue');

  it('aligns weekday headers and day numbers to the same edge', () => {
    expect(rule(grid, '.month__weekdays span')).toMatch(/text-align: right;/);
    expect(rule(grid, '.month__daynum')).toMatch(/align-self: flex-end;/);
  });

  it('insets the weekday header by the same amount as the day number', () => {
    expect(rule(grid, '.month__weekdays span')).toMatch(/padding: \S+ var\(--month-inset\);/);
  });

  it('keeps every event chip on one line', () => {
    const chip = read('src/components/calendar/CalendarEventChip.vue');

    expect(rule(chip, '.event-chip__title')).toMatch(/white-space: nowrap;/);
    expect(chip).not.toMatch(/line-clamp/);
  });
});
