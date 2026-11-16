import { describe, expect, it } from 'vitest';
import { availabilityHtml, availabilityLines, availabilityPlainText, durationText } from './availabilityText';
import type { BackendFreeSlotsReport } from '@/services/backend/availability';

/** A translator that shows which key and values were used. */
function t(key: string, named: Record<string, unknown> = {}, plural?: number): string {
  const values = Object.entries(named).map(([name, value]) => `${name}=${String(value)}`).join(',');
  return `${key}${plural === undefined ? '' : `#${plural}`}(${values})`;
}

const report: BackendFreeSlotsReport = {
  time_zone: 'Europe/Berlin',
  unknown: [],
  days: [
    { date: '2026-11-09', windows: [
      { starts_at: '2026-11-09T08:00:00Z', ends_at: '2026-11-09T10:30:00Z' },
      { starts_at: '2026-11-09T13:00:00Z', ends_at: '2026-11-09T16:00:00Z' },
    ] },
    { date: '2026-11-10', windows: [] },
    { date: '2026-11-11', windows: [{ starts_at: '2026-11-11T08:00:00Z', ends_at: '2026-11-11T16:00:00Z' }] },
  ],
};

describe('availability text', () => {
  it('lists the days with free time in the report zone and names the zone', () => {
    const lines = availabilityLines(report, { t, locale: 'en-GB', durationMinutes: 30 })!;
    expect(lines.intro).toContain('availability.text.intro(');
    expect(lines.intro).toContain('duration=availability.text.minutes#30(count=30)');
    expect(lines.intro).toMatch(/zone=.+, Europe\/Berlin\)$/);
    expect(lines.days).toHaveLength(2);
    // Berlin is UTC+1 in November: 08:00Z reads 09:00.
    expect(lines.days[0]).toMatch(/^availability\.text\.day\(day=Mon,? 9 Nov,times=09:00\s*[–-]\s*11:30, 14:00\s*[–-]\s*17:00\)$/);
    expect(lines.days[1]).toMatch(/day=Wed,? 11 Nov,times=09:00\s*[–-]\s*17:00/);
  });

  it('writes plain spaces, never the thin or narrow ones ICU puts around ranges and AM/PM', () => {
    for (const locale of ['en', 'en-US', 'de']) {
      const lines = availabilityLines(report, { t, locale, durationMinutes: 30 })!;
      for (const line of [lines.intro, ...lines.days]) {
        expect(line).not.toMatch(/[\u00a0\u2009\u202f]/);
      }
    }
  });

  it('reads whole hours as hours', () => {
    expect(durationText(t, 60)).toBe('availability.text.hours#1(count=1)');
    expect(durationText(t, 90)).toBe('availability.text.minutes#90(count=90)');
  });

  it('gives nothing to insert when no day has a free window', () => {
    expect(availabilityLines({ ...report, days: [{ date: '2026-11-10', windows: [] }] }, { t, locale: 'en', durationMinutes: 30 })).toBeNull();
  });

  it('writes a plain list for plain text and an escaped list for the rich editor', () => {
    const lines = { intro: 'Free <then> & there:', days: ['Mon: 9–10', 'Tue: "all day"'] };
    expect(availabilityPlainText(lines)).toBe('Free <then> & there:\n- Mon: 9–10\n- Tue: "all day"');
    expect(availabilityHtml(lines)).toBe('<p>Free &lt;then&gt; &amp; there:</p><ul><li><p>Mon: 9–10</p></li><li><p>Tue: &quot;all day&quot;</p></li></ul>');
  });
});
