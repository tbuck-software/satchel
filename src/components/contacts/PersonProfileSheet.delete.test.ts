// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useToast } from '@/composables/useToast';
import { useContactsStore } from '@/stores/contacts';
import { answerConfirms, type DialogStub } from '@/test/dialogs';
import type { ConfirmRequest } from '@/composables/useConfirm';

const backend = vi.hoisted(() => ({
  backendDeleteContact: vi.fn(),
  backendListContacts: vi.fn(async () => []),
  backendBlockSender: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  ...backend,
}));
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

import PersonProfileSheet from './PersonProfileSheet.vue';

const PERSON = { name: 'Ada Lovelace', email: 'ada@example.test' };

function seedContact() {
  const contacts = useContactsStore();
  contacts.contacts = [{
    id: 'contact-1',
    email: PERSON.email,
    display_name: PERSON.name,
    avatar_url: null,
    avatar_source: null,
    avatar_locked: 0,
    avatar_updated_at: null,
    frequency: 3,
    last_contacted: null,
    notes: 'met at the conference',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
  }];
  return contacts;
}

/** The sheet teleports to body, so the button is not inside the wrapper. */
function deleteButton() {
  return Array.from(document.body.querySelectorAll<HTMLButtonElement>('button'))
    .find((button) => button.textContent?.trim() === 'Delete contact');
}

let confirms: DialogStub<ConfirmRequest, boolean>;

beforeEach(() => {
  confirms = answerConfirms(true);
  setActivePinia(createPinia());
  vi.clearAllMocks();
  document.body.innerHTML = '';
  useToast().clear();
});

afterEach(() => confirms.stop());

describe('deleting a contact from the profile sheet', () => {
  it('does not touch the backend when the confirmation is declined', async () => {
    seedContact();
    confirms.reply(false);
    const wrapper = mount(PersonProfileSheet, { props: { open: true, person: PERSON } });

    deleteButton()!.click();
    await vi.waitFor(() => expect(confirms.asked).toHaveBeenCalledTimes(1));

    expect(confirms.asked.mock.calls[0]![0].body).toContain(PERSON.email);
    expect(confirms.asked.mock.calls[0]![0].tone).toBe('danger');
    expect(backend.backendDeleteContact).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('deletes the confirmed contact and closes the sheet', async () => {
    const contacts = seedContact();
    backend.backendDeleteContact.mockResolvedValue(undefined);
    const wrapper = mount(PersonProfileSheet, { props: { open: true, person: PERSON } });

    deleteButton()!.click();
    await vi.waitFor(() => expect(backend.backendDeleteContact).toHaveBeenCalledWith('contact-1'));

    await vi.waitFor(() => expect(wrapper.emitted('dismiss')).toBeTruthy());
    expect(contacts.contacts).toEqual([]);
    wrapper.unmount();
  });

  it('puts a refused contact back and says why', async () => {
    const contacts = seedContact();
    backend.backendDeleteContact.mockRejectedValue(new Error('database is locked'));
    const wrapper = mount(PersonProfileSheet, { props: { open: true, person: PERSON } });

    deleteButton()!.click();
    await vi.waitFor(() => expect(useToast().state.toasts).toHaveLength(1));

    expect(useToast().state.toasts[0]?.tone).toBe('error');
    expect(useToast().state.toasts[0]?.message).toContain('database is locked');
    expect(contacts.contacts).toHaveLength(1);
    expect(wrapper.emitted('dismiss')).toBeFalsy();
    wrapper.unmount();
  });

  it('offers no delete for a person who is not a stored contact', () => {
    const wrapper = mount(PersonProfileSheet, {
      props: { open: true, person: { name: 'Stranger', email: 'nobody@example.test' } },
    });

    expect(deleteButton()).toBeUndefined();
    wrapper.unmount();
  });
});
