// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { enableAutoUnmount, mount } from '@vue/test-utils';
import EventEditorDialog, { type EventEditorPayload } from './EventEditorDialog.vue';
import { draftFromItem, emptyDraft, itemFromEvent, toDateInput } from '@/utils/calendarGrid';

enableAutoUnmount(afterEach);
beforeEach(() => setActivePinia(createPinia()));

describe('all-day date-only boundary', () => {
  // Run this suite under TZ=America/Los_Angeles and TZ=Europe/Berlin too.
  for (const day of ['2026-03-08', '2026-03-29', '2026-10-25', '2026-11-01']) {
    it(`saves ${day} with an exclusive date end, without timezone conversion`, async () => {
      const draft = { ...emptyDraft(new Date(`${day}T00:00`)), allDay: true, title: 'Holiday' };
      const view = mount(EventEditorDialog, { props: { draft, saving: false }, global: { stubs: { Teleport: true } } });
      const dates = view.findAll('input[type="date"]');
      await dates[0]!.setValue(day);
      await dates[1]!.setValue(day);
      await view.find('.event-editor__actions .app-button--primary').trigger('click');
      const payload = view.emitted('save')![0]![0] as EventEditorPayload;
      expect(payload.startsAt).toBe(day);
      const next = new Date(`${day}T00:00`);
      next.setDate(next.getDate() + 1);
      expect(payload.endsAt).toBe(toDateInput(next));
    });
  }

  it('round-trips an existing UTC-midnight mirror as local calendar dates', async () => {
    const item = itemFromEvent({ id: 'event', uid: 'uid', account_id: 'a', title: 'Holiday',
      starts_at: '2026-03-28T00:00:00Z', ends_at: '2026-03-31T00:00:00Z', all_day: 1,
      attendees_json: '["guest@example.com"]', source: 'caldav', collection_href: null,
      description: '  <p>Agenda</p>  ', location: null, created_at: '', updated_at: 'revision-1' });
    const view = mount(EventEditorDialog, { props: { draft: draftFromItem(item), saving: false }, global: { stubs: { Teleport: true } } });
    const dates = view.findAll('input[type="date"]');
    expect((dates[0]!.element as HTMLInputElement).value).toBe('2026-03-28');
    expect((dates[1]!.element as HTMLInputElement).value).toBe('2026-03-30');
    await view.find('input').setValue('Updated holiday');
    await view.find('.event-editor__actions .app-button--primary').trigger('click');
    expect(view.emitted('save')![0]![0]).toMatchObject({ expectedUpdatedAt: 'revision-1', title: 'Updated holiday', description: '  <p>Agenda</p>  ', startsAt: '2026-03-28', endsAt: '2026-03-31', attendees: ['guest@example.com'], sendInvites: false });
  });
  it('reuses the open dialog for an all-day draft without parsing its dates as UTC', async () => {
    const view = mount(EventEditorDialog, { props: { draft: emptyDraft(new Date('2026-03-28T10:00')), saving: false }, global: { stubs: { Teleport: true } } });
    await view.setProps({ draft: { ...emptyDraft(new Date('2026-03-28T00:00')), allDay: true, title: 'Holiday', end: new Date('2026-03-31T00:00') } });
    const dates = view.findAll('input[type="date"]');
    expect((dates[0]!.element as HTMLInputElement).value).toBe('2026-03-28');
    expect((dates[1]!.element as HTMLInputElement).value).toBe('2026-03-30');
  });

  it('keeps timed events as instants', async () => {
    const draft = { ...emptyDraft(new Date('2026-03-28T10:00:37.123')), id: 'event', title: 'Call' };
    const view = mount(EventEditorDialog, { props: { draft, saving: false }, global: { stubs: { Teleport: true } } });
    await view.find('.event-editor__actions .app-button--primary').trigger('click');
    expect(view.emitted('save')![0]![0]).toMatchObject({ startsAt: draft.start.toISOString(), endsAt: draft.end.toISOString(), allDay: false });
  });

  it('includes a partially occupied final day when switching an overnight event to all-day', async () => {
    const draft = { ...emptyDraft(new Date(2026, 2, 28, 23)), title: 'Overnight', end: new Date(2026, 2, 29, 1) };
    const view = mount(EventEditorDialog, { props: { draft, saving: false }, global: { stubs: { Teleport: true } } });
    await view.get('input[role="switch"]').setValue(true);
    const dates = view.findAll('input[type="date"]');
    expect((dates[0]!.element as HTMLInputElement).value).toBe('2026-03-28');
    expect((dates[1]!.element as HTMLInputElement).value).toBe('2026-03-29');
    await view.find('.event-editor__actions .app-button--primary').trigger('click');
    expect(view.emitted('save')![0]![0]).toMatchObject({ startsAt: '2026-03-28', endsAt: '2026-03-30', allDay: true });
  });

});

describe('Outlook events', () => {
  const outlookEvent = (fields: Record<string, unknown> = {}) => itemFromEvent({
    id: 'outlook-event-1', uid: 'uid', account_id: 'outlook', title: 'Weekly sync',
    starts_at: '2026-07-14T14:00:00Z', ends_at: '2026-07-14T15:00:00Z', all_day: 0,
    attendees_json: '["alice@example.test"]', source: 'outlook', collection_href: 'cal-main',
    description: 'Join: Microsoft Teams', location: null, created_at: '', updated_at: 'revision-1',
    read_only: 0, recurrence_instance: 1, rich_description: 1, ...fields,
  });
  const open = (draft: ReturnType<typeof draftFromItem>, guestsNeedInvites = false) => mount(EventEditorDialog, {
    props: { draft, saving: false, guestsNeedInvites }, global: { stubs: { Teleport: true } },
  });

  it('keeps a formatted description as Outlook has it and says why', async () => {
    const view = open(draftFromItem(outlookEvent()));
    const description = view.get('textarea');
    expect(description.attributes('disabled')).toBeDefined();
    expect(view.text()).toContain('This description is formatted in Outlook');
    await view.find('input').setValue('Weekly sync (moved)');
    await view.find('.event-editor__actions .app-button--primary').trigger('click');
    expect(view.emitted('save')![0]![0]).toMatchObject({ title: 'Weekly sync (moved)', description: 'Join: Microsoft Teams' });
  });

  it('says an edit reaches one occurrence and the attendees', () => {
    const view = open(draftFromItem(outlookEvent({ rich_description: 0 })));
    expect(view.get('textarea').attributes('disabled')).toBeUndefined();
    expect(view.text()).toContain('Changes apply to this occurrence only.');
    expect(view.text()).toContain('Attendees will be notified of your changes.');
  });

  it('warns that guests of a new Outlook event need invitations', async () => {
    const view = open({ ...emptyDraft(new Date('2026-07-14T10:00')), title: 'Lunch' }, true);
    const hint = 'Guests are only saved with invitations on.';
    expect(view.text()).not.toContain(hint);
    const inputs = view.findAll('input');
    const guests = inputs.find((input) => input.attributes('placeholder')?.includes('@'))
      ?? inputs[inputs.length - 2]!;
    await guests.setValue('bob@example.test');
    expect(view.text()).toContain(hint);
    // The last switch is "Send invitations"; the first is "All day".
    const switches = view.findAll('input[role="switch"]');
    await switches[switches.length - 1]!.setValue(true);
    expect(view.text()).not.toContain(hint);
  });

  it('does not warn for calendars that keep guests without invitations', async () => {
    const view = open({ ...emptyDraft(new Date('2026-07-14T10:00')), title: 'Lunch', attendees: 'bob@example.test' }, false);
    expect(view.text()).not.toContain('Guests are only saved with invitations on.');
  });
});
