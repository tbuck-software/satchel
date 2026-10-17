import { describe, expect, it } from 'vitest';
import { parseReminderDate, reminderSuggestions } from './reminderDate';

const now = new Date(2026, 7, 12, 10, 30, 0, 0);

describe('reminderDate', () => {
  it.each([
    ['morgen 9', new Date(2026, 7, 13, 9, 0)],
    ['tomorrow at 4pm', new Date(2026, 7, 13, 16, 0)],
    ['übermorgen um 14:30', new Date(2026, 7, 14, 14, 30)],
    ['next monday 8', new Date(2026, 7, 17, 8, 0)],
    ['in 3 hours', new Date(2026, 7, 12, 13, 30)],
    ['in 2 Tagen', new Date(2026, 7, 14, 10, 30)],
    ['15.08. 12:15', new Date(2026, 7, 15, 12, 15)],
    ['2026-08-20 09:45', new Date(2026, 7, 20, 9, 45)],
  ])('parses %s across German and English', (input, expected) => {
    expect(parseReminderDate(input, now)?.getTime()).toBe(expected.getTime());
  });

  it('rolls a time-only reminder to tomorrow when that time has passed', () => {
    expect(parseReminderDate('9:00', now)?.getTime()).toBe(
      new Date(2026, 7, 13, 9, 0).getTime(),
    );
  });

  it('shows presets for an empty second step and one parsed result while typing', () => {
    expect(reminderSuggestions('', now)).toHaveLength(4);
    const parsed = reminderSuggestions('tomorrow 4pm', now);
    expect(parsed).toHaveLength(1);
    expect(parsed[0]!.label).toBe('Set reminder');
  });

  it('rejects unknown or past values', () => {
    expect(parseReminderDate('irgendwann vielleicht', now)).toBeNull();
    expect(parseReminderDate('heute 9', now)).toBeNull();
  });
});
