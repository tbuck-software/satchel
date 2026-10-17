// @vitest-environment jsdom

import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { backendSearchContacts } from '@/services/backend';
import type { BackendContact } from '@/types/contacts';
import RecipientChips from './RecipientChips.vue';

vi.mock('@/services/backend', () => ({
  backendListContacts: vi.fn(),
  backendSearchContacts: vi.fn(),
  backendSetContactAvatar: vi.fn(),
  backendSyncContactAvatars: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendUpsertContact: vi.fn(),
}));

const searchMock = vi.mocked(backendSearchContacts);
function contact(overrides: Partial<BackendContact> = {}): BackendContact {
  return {
    id: 'alice',
    email: 'alice@example.com',
    display_name: 'Alice',
    avatar_locked: 0,
    frequency: 0,
    created_at: '2026-08-30T00:00:00Z',
    updated_at: '2026-08-30T00:00:00Z',
    ...overrides,
  };
}

const alice = contact();

function pasteEvent(text: string): ClipboardEvent {
  const event = new Event('paste') as ClipboardEvent;
  Object.defineProperty(event, 'clipboardData', { value: { getData: () => text } });
  return event;
}

describe('RecipientChips address parsing', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  async function paste(text: string, modelValue: string[] = []) {
    const wrapper = mount(RecipientChips, { props: { modelValue } });
    await wrapper.get('input').element.dispatchEvent(pasteEvent(text));
    const emitted = wrapper.emitted('update:modelValue');
    wrapper.unmount();
    return emitted?.at(-1)?.[0] as string[] | undefined;
  }

  it('keeps an Outlook token with a comma in the display name whole', async () => {
    expect(await paste('"Morgan, Alex" <a.morgan@example.com>')).toEqual([
      '"Morgan, Alex" <a.morgan@example.com>',
    ]);
  });

  it('splits two plain addresses into two chips', async () => {
    expect(await paste('a@x.de, b@y.de')).toEqual(['a@x.de', 'b@y.de']);
  });

  it('splits a mixed list and keeps the quoted name of the first entry', async () => {
    expect(await paste('"A, B" <a@x.de>, c@y.de')).toEqual(['"A, B" <a@x.de>', 'c@y.de']);
  });

  it('splits a semicolon-separated list and skips chips it already has', async () => {
    expect(await paste('a@x.de; b@y.de', ['a@x.de'])).toEqual(['a@x.de', 'b@y.de']);
  });

  it('marks a chip that holds no address as invalid', () => {
    const wrapper = mount(RecipientChips, {
      props: { modelValue: ['a@x.de', '"Buck'] },
    });
    const chips = wrapper.findAll('.chip');
    expect(chips[0]!.classes()).not.toContain('chip--invalid');
    expect(chips[1]!.classes()).toContain('chip--invalid');
    wrapper.unmount();
  });

  it('does not commit on a comma typed inside a quoted display name', async () => {
    const wrapper = mount(RecipientChips, { props: { modelValue: [] } });
    const input = wrapper.get('input');
    await input.setValue('"Buck');
    await input.trigger('keydown', { key: ',' });
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();

    await input.setValue('"Morgan, Alex" <t@x.de>');
    await input.trigger('keydown', { key: ',' });
    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toEqual([
      '"Morgan, Alex" <t@x.de>',
    ]);
    wrapper.unmount();
  });
});

describe('RecipientChips contact suggestions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('waits 120ms before asking the contacts store and renders its result', async () => {
    searchMock.mockResolvedValue([alice]);
    const wrapper = mount(RecipientChips, { props: { modelValue: [] } });
    const input = wrapper.get('input');

    await input.setValue('al');
    await vi.advanceTimersByTimeAsync(119);
    expect(searchMock).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    await flushPromises();
    expect(searchMock).toHaveBeenCalledWith('al', 8);
    expect(wrapper.get('[role="option"]').text()).toContain('Alice');
    wrapper.unmount();
  });

  it('ignores a stale response after the draft changes', async () => {
    let resolveAlice!: (value: BackendContact[]) => void;
    let resolveBob!: (value: BackendContact[]) => void;
    searchMock
      .mockImplementationOnce(() => new Promise<BackendContact[]>((resolve) => { resolveAlice = resolve; }))
      .mockImplementationOnce(() => new Promise<BackendContact[]>((resolve) => { resolveBob = resolve; }));
    const wrapper = mount(RecipientChips, { props: { modelValue: [] } });
    const input = wrapper.get('input');

    await input.setValue('al');
    await vi.advanceTimersByTimeAsync(120);
    await input.setValue('bo');
    await vi.advanceTimersByTimeAsync(120);

    resolveAlice([alice]);
    await flushPromises();
    expect(wrapper.find('[role="option"]').exists()).toBe(false);

    const bob = contact({ id: 'bob', email: 'bob@example.com', display_name: 'Bob' });
    resolveBob([bob]);
    await flushPromises();
    expect(wrapper.get('[role="option"]').text()).toContain('Bob');
    expect(wrapper.text()).not.toContain('Alice');
    wrapper.unmount();
  });

  it('clears short drafts and backend failures without an unhandled rejection', async () => {
    searchMock.mockRejectedValue(new Error('search failed'));
    const wrapper = mount(RecipientChips, { props: { modelValue: [] } });
    const input = wrapper.get('input');

    await input.setValue('a');
    await vi.advanceTimersByTimeAsync(200);
    expect(searchMock).not.toHaveBeenCalled();

    await input.setValue('al');
    await vi.advanceTimersByTimeAsync(120);
    await flushPromises();
    expect(wrapper.find('[role="option"]').exists()).toBe(false);
    wrapper.unmount();
  });
});
