import { describe, expect, it } from 'vitest';
import type { BackendCalendarEvent, BackendCalendarInvite } from '@/services/backend';
import {
  collectSources,
  descriptionToText,
  filterInvitesWithoutMirroredEvents,
  itemFromEvent,
  itemFromInvite,
  sourceColorMap,
  sourceLabel,
  isMultiDay,
  itemsForDay,
  layoutDay,
  minutesInDay,
  monthGridDays,
  overlapsDay,
  startOfWeek,
  weekDays,
  type CalendarItem,
} from './calendarGrid';

function makeItem(overrides: Partial<CalendarItem>): CalendarItem {
  return {
    key: 'event:x',
    kind: 'event',
    id: 'x',
    uid: 'uid-x',
    title: 'Test',
    location: null,
    description: null,
    organizer: null,
    attendees: [],
    start: new Date(2026, 6, 29, 9, 0),
    end: new Date(2026, 6, 29, 10, 0),
    allDay: false,
    status: 'own',
    source: 'local',
    sourceKey: 'local',
    color: null,
    ...overrides,
  };
}

describe('startOfWeek', () => {
  it('returns the Monday of the week', () => {
    // 2026-07-29 is a Wednesday.
    expect(startOfWeek(new Date(2026, 6, 29)).getDate()).toBe(27);
  });

  it('treats Sunday as the last day of the week', () => {
    // 2026-08-02 is a Sunday → Monday is 2026-07-27.
    const monday = startOfWeek(new Date(2026, 7, 2));
    expect(monday.getMonth()).toBe(6);
    expect(monday.getDate()).toBe(27);
  });

  it('keeps Monday itself', () => {
    expect(startOfWeek(new Date(2026, 6, 27, 23, 59)).getDate()).toBe(27);
  });
});

describe('monthGridDays', () => {
  it('starts on a Monday and covers whole weeks', () => {
    const days = monthGridDays(new Date(2026, 6, 15));
    expect(days[0]!.getDay()).toBe(1);
    expect(days.length % 7).toBe(0);
    expect(days.some((d) => d.getMonth() === 6 && d.getDate() === 1)).toBe(true);
    expect(days.some((d) => d.getMonth() === 6 && d.getDate() === 31)).toBe(true);
  });

  it('uses five rows when the month fits', () => {
    // July 2026: Jul 1 is a Wednesday, 31 days → fits in five weeks.
    expect(monthGridDays(new Date(2026, 6, 1)).length).toBe(35);
  });

  it('uses six rows when the month needs them', () => {
    // August 2026: Aug 1 is a Saturday, 31 days → needs six weeks.
    expect(monthGridDays(new Date(2026, 7, 1)).length).toBe(42);
  });
});

describe('weekDays', () => {
  it('returns seven consecutive days from Monday', () => {
    const days = weekDays(new Date(2026, 6, 29));
    expect(days.length).toBe(7);
    expect(days[0]!.getDate()).toBe(27);
    expect(days[6]!.getDate()).toBe(2);
  });
});

describe('overlapsDay', () => {
  it('excludes an event ending exactly at midnight from the next day', () => {
    const item = makeItem({
      start: new Date(2026, 6, 29, 22, 0),
      end: new Date(2026, 6, 30, 0, 0),
    });
    expect(overlapsDay(item, new Date(2026, 6, 29))).toBe(true);
    expect(overlapsDay(item, new Date(2026, 6, 30))).toBe(false);
  });

  it('includes every day a multi-day event touches', () => {
    const item = makeItem({
      start: new Date(2026, 6, 28, 18, 0),
      end: new Date(2026, 6, 30, 9, 0),
    });
    expect(overlapsDay(item, new Date(2026, 6, 28))).toBe(true);
    expect(overlapsDay(item, new Date(2026, 6, 29))).toBe(true);
    expect(overlapsDay(item, new Date(2026, 6, 30))).toBe(true);
    expect(overlapsDay(item, new Date(2026, 6, 31))).toBe(false);
  });
});

describe('isMultiDay', () => {
  it('is false for an event ending at midnight of the next day', () => {
    const item = makeItem({
      start: new Date(2026, 6, 29, 22, 0),
      end: new Date(2026, 6, 30, 0, 0),
    });
    expect(isMultiDay(item)).toBe(false);
  });

  it('is true when the event crosses into the next day', () => {
    const item = makeItem({
      start: new Date(2026, 6, 29, 22, 0),
      end: new Date(2026, 6, 30, 1, 0),
    });
    expect(isMultiDay(item)).toBe(true);
  });
});

describe('itemsForDay', () => {
  it('sorts all-day and multi-day items before timed ones', () => {
    const timed = makeItem({ key: 'event:timed', start: new Date(2026, 6, 29, 8, 0), end: new Date(2026, 6, 29, 9, 0) });
    const allDay = makeItem({
      key: 'event:allday',
      allDay: true,
      start: new Date(2026, 6, 29, 0, 0),
      end: new Date(2026, 6, 30, 0, 0),
    });
    const sorted = itemsForDay([timed, allDay], new Date(2026, 6, 29));
    expect(sorted.map((i) => i.key)).toEqual(['event:allday', 'event:timed']);
  });
});

describe('minutesInDay', () => {
  it('clamps a multi-day event to the visible day', () => {
    const item = makeItem({
      start: new Date(2026, 6, 28, 18, 0),
      end: new Date(2026, 6, 29, 9, 30),
    });
    expect(minutesInDay(item, new Date(2026, 6, 29))).toEqual({ startMin: 0, endMin: 570 });
  });
});

describe('layoutDay', () => {
  const day = new Date(2026, 6, 29);

  it('gives non-overlapping events the full width', () => {
    const a = makeItem({ key: 'event:a', start: new Date(2026, 6, 29, 9), end: new Date(2026, 6, 29, 10) });
    const b = makeItem({ key: 'event:b', start: new Date(2026, 6, 29, 10), end: new Date(2026, 6, 29, 11) });
    const layout = layoutDay([a, b], day);
    expect(layout.every((entry) => entry.overlapLevel === 0)).toBe(true);
  });

  it('stacks a later overlapping event above the earlier one', () => {
    const a = makeItem({ key: 'event:a', start: new Date(2026, 6, 29, 9), end: new Date(2026, 6, 29, 11) });
    const b = makeItem({ key: 'event:b', start: new Date(2026, 6, 29, 10), end: new Date(2026, 6, 29, 12) });
    const layout = layoutDay([a, b], day);
    expect(layout.map((entry) => [entry.item.key, entry.overlapLevel])).toEqual([
      ['event:a', 0],
      ['event:b', 1],
    ]);
  });

  it('only counts events that are still visible when the next one starts', () => {
    const a = makeItem({ key: 'event:a', start: new Date(2026, 6, 29, 9), end: new Date(2026, 6, 29, 12) });
    const b = makeItem({ key: 'event:b', start: new Date(2026, 6, 29, 9, 30), end: new Date(2026, 6, 29, 10) });
    const c = makeItem({ key: 'event:c', start: new Date(2026, 6, 29, 10, 30), end: new Date(2026, 6, 29, 11) });
    const layout = layoutDay([a, b, c], day);
    const byKey = Object.fromEntries(layout.map((entry) => [entry.item.key, entry]));
    expect(byKey['event:a']!.overlapLevel).toBe(0);
    expect(byKey['event:b']!.overlapLevel).toBe(1);
    expect(byKey['event:c']!.overlapLevel).toBe(1);
  });

  it('adds another stack level for simultaneous overlaps', () => {
    const a = makeItem({ key: 'event:a', start: new Date(2026, 6, 29, 9), end: new Date(2026, 6, 29, 12) });
    const b = makeItem({ key: 'event:b', start: new Date(2026, 6, 29, 9, 30), end: new Date(2026, 6, 29, 11) });
    const c = makeItem({ key: 'event:c', start: new Date(2026, 6, 29, 10), end: new Date(2026, 6, 29, 10, 30) });
    expect(layoutDay([a, b, c], day).map((entry) => entry.overlapLevel)).toEqual([0, 1, 2]);
  });

  it('does not accumulate indentation across a chain of five-minute overlaps', () => {
    const a = makeItem({
      key: 'event:a',
      start: new Date(2026, 6, 29, 9),
      end: new Date(2026, 6, 29, 10),
    });
    const b = makeItem({
      key: 'event:b',
      start: new Date(2026, 6, 29, 9, 55),
      end: new Date(2026, 6, 29, 11),
    });
    const c = makeItem({
      key: 'event:c',
      start: new Date(2026, 6, 29, 10, 55),
      end: new Date(2026, 6, 29, 12),
    });

    expect(layoutDay([a, b, c], day).map((entry) => entry.overlapLevel)).toEqual([0, 1, 1]);
  });

  it('keeps all-day items out of the timed grid', () => {
    const allDay = makeItem({
      allDay: true,
      start: new Date(2026, 6, 29, 0, 0),
      end: new Date(2026, 6, 30, 0, 0),
    });
    expect(layoutDay([allDay], day)).toEqual([]);
  });

  it('stretches very short events to a readable minimum height', () => {
    const short = makeItem({ start: new Date(2026, 6, 29, 9), end: new Date(2026, 6, 29, 9, 5) });
    const layout = layoutDay([short], day);
    expect(layout[0]!.endMin - layout[0]!.startMin).toBe(30);
  });
});

describe('itemFromInvite', () => {
  function makeInvite(overrides: Partial<BackendCalendarInvite>): BackendCalendarInvite {
    return {
      id: 'inv-1',
      account_id: 'acc-1',
      uid: 'uid-1',
      summary: 'Planung',
      all_day: 0,
      rsvp_status: 'needsAction',
      raw_ics: '',
      created_at: '2026-07-01T00:00:00Z',
      updated_at: '2026-07-01T00:00:00Z',
      ...overrides,
    };
  }

  it('returns null for invites without a start date', () => {
    expect(itemFromInvite(makeInvite({ starts_at: null }))).toBeNull();
  });

  it('defaults a missing end to one hour after the start', () => {
    const item = itemFromInvite(makeInvite({ starts_at: '2026-07-29T09:00:00Z', ends_at: null }));
    expect(item).not.toBeNull();
    expect(item!.end.getTime() - item!.start.getTime()).toBe(60 * 60 * 1000);
  });

  it('maps cancelled invites regardless of RSVP status', () => {
    const item = itemFromInvite(
      makeInvite({ starts_at: '2026-07-29T09:00:00Z', rsvp_status: 'accepted', cancelled: 1 }),
    );
    expect(item!.status).toBe('cancelled');
  });
});

describe('filterInvitesWithoutMirroredEvents', () => {
  const invite = {
    id: 'inv-1',
    account_id: 'attendee-account',
    uid: 'shared-meeting-uid',
    all_day: 0,
    rsvp_status: 'needs-action',
    raw_ics: '',
    created_at: '2026-07-01T00:00:00Z',
    updated_at: '2026-07-01T00:00:00Z',
  } satisfies BackendCalendarInvite;

  it('keeps an attendee invite when the organizer account mirrors the same UID', () => {
    const events = [{ account_id: 'organizer-account', uid: invite.uid }];
    expect(filterInvitesWithoutMirroredEvents(events, [invite])).toEqual([invite]);
  });

  it('hides an invite mirrored as an event in the same account', () => {
    const events = [{ account_id: invite.account_id, uid: invite.uid }];
    expect(filterInvitesWithoutMirroredEvents(events, [invite])).toEqual([]);
  });
});

describe('calendar sources', () => {
  it('labels the built-in sources in English and prettifies Google ids', () => {
    expect(sourceLabel('local')).toBe('Local');
    expect(sourceLabel('caldav')).toBe('GroupWise');
    expect(sourceLabel('invites')).toBe('Invitations');
    expect(sourceLabel('google:alex@example.com')).toBe('alex@example.com');
    expect(sourceLabel('google:de.german#holiday@group.v.calendar.google.com')).toBe('de german holiday');
    expect(sourceLabel('google:primary')).toBe('Main calendar');
  });

  it('assigns stable distinct colors to Google calendars', () => {
    const map = sourceColorMap(['google:b@x', 'google:a@x', 'local']);
    expect(map.get('google:a@x')).toBeDefined();
    expect(map.get('google:a@x')).not.toBe(map.get('google:b@x'));
    // Sorted assignment: order of appearance must not change the colors.
    const reordered = sourceColorMap(['local', 'google:a@x', 'google:b@x']);
    expect(reordered.get('google:a@x')).toBe(map.get('google:a@x'));
  });

  it('collects sources with counts, Google calendars first', () => {
    const items = [
      makeItem({ key: 'e:1', sourceKey: 'local' }),
      makeItem({ key: 'e:2', sourceKey: 'google:a@x' }),
      makeItem({ key: 'e:3', sourceKey: 'google:a@x' }),
      makeItem({ key: 'e:4', kind: 'invite', sourceKey: 'invites' }),
    ];
    const sources = collectSources(items);
    expect(sources.map((s) => s.key)).toEqual(['google:a@x', 'local', 'invites']);
    expect(sources[0]!.count).toBe(2);
  });

  it('keeps empty declared calendars and uses their display names', () => {
    const sources = collectSources([], [
      {
        key: 'google:fixture-family@group.calendar.google.com',
        label: 'Familie',
      },
    ]);
    expect(sources).toEqual([
      {
        key: 'google:fixture-family@group.calendar.google.com',
        label: 'Familie',
        color: 'var(--accent-pink)',
        count: 0,
      },
    ]);
  });
});

describe('descriptionToText', () => {
  it('passes plain text through untouched, including literal angle brackets', () => {
    expect(descriptionToText('Agenda: a < b und b > c')).toBe('Agenda: a < b und b > c');
  });

  it('converts break and paragraph tags to newlines and strips the rest', () => {
    const zoomStyle =
      '<p><br>——————<br><br>Stefan is inviting you.<br>Join Zoom Meeting<br>https://example.zoom.us/j/123?pwd=abc&amp;omn=68</p>';
    expect(descriptionToText(zoomStyle)).toBe(
      '——————\n\nStefan is inviting you.\nJoin Zoom Meeting\nhttps://example.zoom.us/j/123?pwd=abc&omn=68',
    );
  });

  it('decodes entities without double-decoding escaped markup', () => {
    expect(descriptionToText('<p>&amp;lt;wichtig&amp;gt; &quot;Zitat&quot;</p>')).toBe(
      '&lt;wichtig&gt; "Zitat"',
    );
  });

  it('returns null for empty or markup-only input', () => {
    expect(descriptionToText(null)).toBeNull();
    expect(descriptionToText('<p><br></p>')).toBeNull();
  });
});

describe('itemFromEvent', () => {
  it('parses attendee arrays of strings and objects', () => {
    const event: BackendCalendarEvent = {
      id: 'ev-1',
      account_id: 'acc-1',
      uid: 'uid-ev-1',
      title: 'Team-Termin',
      starts_at: '2026-07-29T09:00:00Z',
      ends_at: '2026-07-29T10:00:00Z',
      all_day: 0,
      attendees_json: '["a@example.com", {"email": "b@example.com"}]',
      source: 'local',
      created_at: '2026-07-01T00:00:00Z',
      updated_at: '2026-07-01T00:00:00Z',
    };
    expect(itemFromEvent(event).attendees).toEqual(['a@example.com', 'b@example.com']);
  });
});
