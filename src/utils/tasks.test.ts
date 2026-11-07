import { describe, expect, it } from 'vitest';
import { compareTasks, dueState, formatDue, groupTasks, parseQuickAdd, priorityLevel, type TaskLike } from './tasks';

// Thursday, 6 November 2026, 10:00 local time.
const NOW = new Date(2026, 10, 6, 10, 0, 0);

function task(id: string, extra: Partial<TaskLike> = {}): TaskLike {
  return { id, title: id, due_at: null, priority: 0, completed_at: null, created_at: '2026-11-01T00:00:00Z', ...extra };
}

describe('parseQuickAdd', () => {
  it('takes a trailing date phrase as the due date', () => {
    expect(parseQuickAdd('Pay rent tomorrow', NOW)).toEqual({ title: 'Pay rent', dueAt: '2026-11-07' });
    expect(parseQuickAdd('Steuer machen morgen', NOW)).toEqual({ title: 'Steuer machen', dueAt: '2026-11-07' });
    expect(parseQuickAdd('Pay rent by friday', NOW)).toEqual({ title: 'Pay rent', dueAt: '2026-11-13' });
    expect(parseQuickAdd('Water plants today', NOW)).toEqual({ title: 'Water plants', dueAt: '2026-11-06' });
    expect(parseQuickAdd('Plan trip next week', NOW)).toEqual({ title: 'Plan trip', dueAt: '2026-11-09' });
    expect(parseQuickAdd('Rechnung prüfen 24.12.', NOW)).toEqual({ title: 'Rechnung prüfen', dueAt: '2026-12-24' });
  });

  it('keeps the time when one is named', () => {
    const parsed = parseQuickAdd('Call Anna tomorrow 15:00', NOW);
    expect(parsed.title).toBe('Call Anna');
    expect(new Date(parsed.dueAt!).getTime()).toBe(new Date(2026, 10, 7, 15, 0).getTime());
    const german = parseQuickAdd('Anna anrufen morgen um 9', NOW);
    expect(german.title).toBe('Anna anrufen');
    expect(new Date(german.dueAt!).getTime()).toBe(new Date(2026, 10, 7, 9, 0).getTime());
  });

  it('leaves titles alone that only end in a number or a word', () => {
    expect(parseQuickAdd('Call 3 people', NOW)).toEqual({ title: 'Call 3 people', dueAt: null });
    expect(parseQuickAdd('Read chapter 12', NOW)).toEqual({ title: 'Read chapter 12', dueAt: null });
    expect(parseQuickAdd('Tomorrow', NOW)).toEqual({ title: 'Tomorrow', dueAt: null });
    expect(parseQuickAdd('  Buy milk ', NOW)).toEqual({ title: 'Buy milk', dueAt: null });
  });
});

describe('due state and grouping', () => {
  it('tells overdue, today and upcoming apart, by date and by time', () => {
    expect(dueState(task('a', { due_at: '2026-11-05' }), NOW)).toBe('overdue');
    expect(dueState(task('b', { due_at: '2026-11-06' }), NOW)).toBe('today');
    expect(dueState(task('c', { due_at: new Date(2026, 10, 6, 9).toISOString() }), NOW)).toBe('overdue');
    expect(dueState(task('d', { due_at: new Date(2026, 10, 6, 17).toISOString() }), NOW)).toBe('today');
    expect(dueState(task('e', { due_at: '2026-11-07' }), NOW)).toBe('upcoming');
    expect(dueState(task('f'), NOW)).toBe('none');
  });

  it('groups in reading order and sorts by due, then priority', () => {
    const groups = groupTasks([
      task('someday-low', { priority: 9 }),
      task('someday-high', { priority: 1 }),
      task('later', { due_at: '2026-11-20' }),
      task('soon', { due_at: '2026-11-08' }),
      task('late', { due_at: '2026-11-01' }),
      task('done', { completed_at: '2026-11-05T12:00:00Z', due_at: '2026-11-01' }),
      task('today-timed', { due_at: new Date(2026, 10, 6, 18).toISOString() }),
      task('today-date', { due_at: '2026-11-06' }),
    ], NOW);
    expect(groups.map((group) => [group.group, group.tasks.map((item) => item.id)])).toEqual([
      ['overdue', ['late']],
      ['today', ['today-timed', 'today-date']],
      ['upcoming', ['soon', 'later']],
      ['someday', ['someday-high', 'someday-low']],
      ['completed', ['done']],
    ]);
  });

  it('puts dated tasks before undated ones', () => {
    expect(compareTasks(task('a', { due_at: '2026-12-01' }), task('b'))).toBeLessThan(0);
    expect(compareTasks(task('a'), task('b', { due_at: '2026-12-01' }))).toBeGreaterThan(0);
  });

  it('maps the iCalendar priority scale onto three steps', () => {
    expect([0, 1, 4, 5, 6, 9].map(priorityLevel)).toEqual(['none', 'high', 'high', 'medium', 'low', 'low']);
  });
});

describe('formatDue', () => {
  const labels = { today: 'Today', tomorrow: 'Tomorrow', yesterday: 'Yesterday' };

  it('names the near days and adds a time when there is one', () => {
    expect(formatDue('2026-11-06', 'en-GB', labels, NOW)).toBe('Today');
    expect(formatDue('2026-11-05', 'en-GB', labels, NOW)).toBe('Yesterday');
    expect(formatDue(new Date(2026, 10, 7, 15, 30).toISOString(), 'en-GB', labels, NOW)).toBe('Tomorrow 15:30');
    expect(formatDue('2026-11-20', 'en-GB', labels, NOW)).toBe('Fri 20 Nov');
    expect(formatDue('2027-01-04', 'en-GB', labels, NOW)).toContain('2027');
  });
});

describe('parseQuickAdd keeps what follows a date', () => {
  it('does not cut a title at a weekday in the middle', () => {
    expect(parseQuickAdd('Call Anna on monday about the contract', NOW)).toEqual({
      title: 'Call Anna on monday about the contract',
      dueAt: null,
    });
    expect(parseQuickAdd('Report in 3 days', NOW)).toEqual({ title: 'Report', dueAt: '2026-11-09' });
  });
});
