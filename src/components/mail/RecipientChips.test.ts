// @vitest-environment jsdom

import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { backendSearchContacts } from '@/services/backend';
import type { BackendContact } from '@/types/contacts';
import RecipientChips from './RecipientChips.vue';
import { RECIPIENT_DRAG_TYPE, THREAD_DRAG_TYPE, startAppDrag } from '@/utils/appDrag';
import { resetAppDragForTests } from '@/utils/appDragTarget';

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
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue } });
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
      props: { label: 'To', modelValue: ['a@x.de', '"Buck'] },
    });
    const chips = wrapper.findAll('.chip');
    expect(chips[0]!.classes()).not.toContain('chip--invalid');
    expect(chips[1]!.classes()).toContain('chip--invalid');
    wrapper.unmount();
  });

  it('does not commit on a comma typed inside a quoted display name', async () => {
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: [] } });
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
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: [] } });
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
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: [] } });
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
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: [] } });
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

describe('RecipientChips keyboard and screen readers', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('names the address on each remove button and says which chips are invalid', () => {
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: ['a@x.de', '"Buck'] } });
    const buttons = wrapper.findAll('.chip-remove');
    expect(buttons[0]!.attributes('aria-label')).toBe('Remove a@x.de');
    expect(buttons[1]!.attributes('aria-label')).toBe('Remove "Buck');
    const chips = wrapper.findAll('.chip');
    expect(chips[0]!.find('.sr-only').exists()).toBe(false);
    expect(chips[1]!.get('.sr-only').text()).toBe('(not a valid address)');
    wrapper.unmount();
  });

  it('selects the last chip on the first Backspace and removes it on the second', async () => {
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: ['a@x.de', 'b@y.de'] } });
    const input = wrapper.get('input');

    await input.trigger('keydown', { key: 'Backspace' });
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(wrapper.findAll('.chip')[1]!.classes()).toContain('chip--pending-removal');
    expect(wrapper.get('[role="status"]').text()).toContain('b@y.de');

    await input.trigger('keydown', { key: 'Backspace' });
    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toEqual(['a@x.de']);
    wrapper.unmount();
  });

  it('forgets the selection when another key comes in between', async () => {
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: ['a@x.de'] } });
    const input = wrapper.get('input');

    await input.trigger('keydown', { key: 'Backspace' });
    await input.trigger('keydown', { key: 'ArrowLeft' });
    await input.trigger('keydown', { key: 'Backspace' });
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    wrapper.unmount();
  });

  it.each(['metaKey', 'ctrlKey'] as const)('turns the typed address into a chip, then submits on %s+Enter', async (modifier) => {
    const wrapper = mount(RecipientChips, { props: { label: 'To', modelValue: [] } });
    const input = wrapper.get('input');
    await input.setValue('c@z.de');

    await input.trigger('keydown', { key: 'Enter', [modifier]: true });

    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toEqual(['c@z.de']);
    expect(wrapper.emitted('submit')).toHaveLength(1);
    wrapper.unmount();
  });
});


it('keeps distinct accessible names and input IDs when chips hide the placeholder', () => {
  setActivePinia(createPinia());
  const wrapper = mount({
    components: { RecipientChips },
    template: `<RecipientChips label="To" :model-value="['a@example.com']" placeholder="Name or email" />
      <RecipientChips label="Bcc" :model-value="['b@example.com']" />`,
  });
  const [first, second] = wrapper.findAll('input');
  expect(first!.attributes('aria-label')).toBe('To');
  expect(second!.attributes('aria-label')).toBe('Bcc');
  expect(first!.attributes('id')).toBeTruthy();
  expect(first!.attributes('id')).not.toBe(second!.attributes('id'));
  wrapper.unmount();
});

it('activates a suggestion through semantic click without committing the search text on blur', async () => {
  setActivePinia(createPinia());
  vi.useFakeTimers();
  searchMock.mockResolvedValue([alice]);
  const wrapper = mount(RecipientChips, { attachTo: document.body, props: { label: 'To', modelValue: [] } });
  const input = wrapper.get('input');
  await input.setValue('al');
  await vi.advanceTimersByTimeAsync(120);
  await flushPromises();
  const option = wrapper.get('[role="option"]');
  await input.trigger('blur', { relatedTarget: option.element });
  await option.trigger('click');
  expect(wrapper.emitted('update:modelValue')).toEqual([[['Alice <alice@example.com>']]]);
  wrapper.unmount();
  vi.useRealTimers();
});

describe('RecipientChips drag and drop', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  afterEach(() => {
    resetAppDragForTests();
  });

  function drop(type: string, data: Record<string, string>, altKey = false) {
    const event = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(event, 'altKey', { value: altKey });
    Object.defineProperty(event, 'dataTransfer', {
      value: { types: Object.keys(data), getData: (key: string) => data[key] ?? '', dropEffect: 'none' },
    });
    return event;
  }

  function chipPayload(addresses: string[], source: { composer: string; field: string } | null) {
    return { [RECIPIENT_DRAG_TYPE]: JSON.stringify({ kind: 'recipients', addresses, source }) };
  }

  function field(modelValue: string[] = []) {
    return mount(RecipientChips, {
      props: { label: 'Cc', modelValue, field: 'cc', composerId: 'me' },
    });
  }

  it('adds a dropped contact', async () => {
    const wrapper = field(['bo@example.test']);
    const event = drop('drop', chipPayload(['Ana <ana@example.test>'], null));
    wrapper.get('[data-recipient-field]').element.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(true);
    expect(wrapper.emitted('update:modelValue')?.at(-1)?.[0]).toEqual(['bo@example.test', 'Ana <ana@example.test>']);
    expect(wrapper.emitted('moveFrom')).toBeUndefined();
  });

  it('copies with Ctrl, the copy modifier off the Mac', () => {
    const wrapper = field();
    const event = drop('drop', chipPayload(['ana@example.test'], { composer: 'me', field: 'to' }));
    Object.defineProperty(event, 'ctrlKey', { value: true });
    wrapper.get('[data-recipient-field]').element.dispatchEvent(event);
    expect(wrapper.emitted('update:modelValue')).toHaveLength(1);
    expect(wrapper.emitted('moveFrom')).toBeUndefined();
  });

  it('moves a chip from another field of the same composer, and copies with Alt', async () => {
    const wrapper = field();
    wrapper.get('[data-recipient-field]').element.dispatchEvent(
      drop('drop', chipPayload(['ana@example.test'], { composer: 'me', field: 'to' })),
    );
    expect(wrapper.emitted('moveFrom')?.[0]).toEqual(['to', ['ana@example.test']]);

    wrapper.get('[data-recipient-field]').element.dispatchEvent(
      drop('drop', chipPayload(['bo@example.test'], { composer: 'me', field: 'to' }), true),
    );
    expect(wrapper.emitted('moveFrom')).toHaveLength(1);
    expect(wrapper.emitted('update:modelValue')).toHaveLength(2);
  });

  it('lights up while a recipient hovers and offers a move for its own chips', async () => {
    const wrapper = field();
    startAppDrag(
      { dataTransfer: { setData: () => {}, effectAllowed: '' }, target: null } as unknown as DragEvent,
      { kind: 'recipients', addresses: ['ana@example.test'], source: { composer: 'me', field: 'to' } },
    );
    const over = drop('dragover', { [RECIPIENT_DRAG_TYPE]: '' });
    wrapper.get('[data-recipient-field]').element.dispatchEvent(drop('dragenter', { [RECIPIENT_DRAG_TYPE]: '' }));
    wrapper.get('[data-recipient-field]').element.dispatchEvent(over);
    await wrapper.vm.$nextTick();

    expect(wrapper.get('.chips').classes()).toContain('chips--drop-over');
    expect(over.dataTransfer?.dropEffect).toBe('move');
  });

  it('leaves a chip dropped back on its own field where it is', () => {
    const wrapper = field(['ana@example.test']);
    wrapper.get('[data-recipient-field]').element.dispatchEvent(
      drop('drop', chipPayload(['ana@example.test'], { composer: 'me', field: 'cc' })),
    );
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(wrapper.emitted('moveFrom')).toBeUndefined();
  });

  it('lets mail drags through to the composer', () => {
    const wrapper = field();
    const event = drop('drop', { [THREAD_DRAG_TYPE]: JSON.stringify({ kind: 'threads', ids: ['th-1'] }) });
    wrapper.get('[data-recipient-field]').element.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it('drags a chip out with its field', () => {
    const wrapper = field(['ana@example.test']);
    const data = new Map<string, string>();
    const event = new Event('dragstart', { bubbles: true }) as DragEvent;
    Object.defineProperty(event, 'dataTransfer', {
      value: { setData: (key: string, value: string) => data.set(key, value), effectAllowed: '' },
    });
    wrapper.get('.chip').element.dispatchEvent(event);

    expect(JSON.parse(data.get(RECIPIENT_DRAG_TYPE) ?? '{}')).toEqual({
      kind: 'recipients',
      addresses: ['ana@example.test'],
      source: { composer: 'me', field: 'cc' },
    });
    expect(data.get('text/plain')).toBe('ana@example.test');
  });
});
