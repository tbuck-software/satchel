// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from 'vitest';
import { RECIPIENT_DRAG_TYPE, THREAD_DRAG_TYPE } from '@/utils/appDrag';
import { resetAppDragForTests } from '@/utils/appDragTarget';
import { useRecipientDrop } from './useRecipientDrop';

function dragEvent(data: Record<string, string>, target: Element) {
  return {
    target,
    altKey: false,
    dataTransfer: {
      types: Object.keys(data),
      getData: (type: string) => data[type] ?? '',
      dropEffect: 'none',
    },
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as DragEvent & { preventDefault: ReturnType<typeof vi.fn>; stopPropagation: ReturnType<typeof vi.fn> };
}

function recipients(addresses: string[], source: { composer: string; field: string } | null = null) {
  return { [RECIPIENT_DRAG_TYPE]: JSON.stringify({ kind: 'recipients', addresses, source }), 'text/plain': addresses.join(', ') };
}

function composer() {
  const root = document.createElement('section');
  const body = document.createElement('div');
  const field = document.createElement('div');
  field.setAttribute('data-recipient-field', '');
  const input = document.createElement('input');
  field.append(input);
  root.append(field, body);
  return { body, input };
}

function setup(disabled = false) {
  const addToTo = vi.fn();
  const onActiveChange = vi.fn();
  const drop = useRecipientDrop({ composerId: 'me', addToTo, disabled: () => disabled, onActiveChange });
  return { addToTo, onActiveChange, drop };
}

afterEach(() => {
  resetAppDragForTests();
});

describe('useRecipientDrop', () => {
  it('adds a contact dropped on the body to To', () => {
    const { body } = composer();
    const { addToTo, onActiveChange, drop } = setup();
    const event = dragEvent(recipients(['Ana <ana@example.test>']), body);

    drop.onDragEnter(event);
    expect(drop.dragActive.value).toBe(true);
    expect(drop.overBody.value).toBe(true);
    expect(onActiveChange).toHaveBeenCalledWith(true);
    drop.onDragOver(event);
    expect(event.dataTransfer?.dropEffect).toBe('copy');
    drop.onDrop(event);

    expect(event.preventDefault).toHaveBeenCalled();
    expect(event.stopPropagation).toHaveBeenCalled();
    expect(addToTo).toHaveBeenCalledWith(['Ana <ana@example.test>']);
    expect(drop.dragActive.value).toBe(false);
  });

  it('leaves a drop on a recipient field to the field', async () => {
    const { input } = composer();
    const { addToTo, onActiveChange, drop } = setup();
    const event = dragEvent(recipients(['ana@example.test']), input);

    drop.onDragEnter(event);
    expect(drop.overBody.value).toBe(false);
    drop.onDrop(event);

    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(event.stopPropagation).not.toHaveBeenCalled();
    expect(addToTo).not.toHaveBeenCalled();
    // The end is reported after the field had its turn.
    expect(onActiveChange).not.toHaveBeenCalledWith(false);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(onActiveChange).toHaveBeenLastCalledWith(false);
  });

  it('does not add this composer’s own chip a second time when it misses the fields', () => {
    const { body } = composer();
    const { addToTo, drop } = setup();
    const event = dragEvent(recipients(['ana@example.test'], { composer: 'me', field: 'cc' }), body);

    drop.onDrop(event);

    expect(addToTo).not.toHaveBeenCalled();
  });

  it('takes a chip from another composer as a contact', () => {
    const { body } = composer();
    const { addToTo, drop } = setup();

    drop.onDrop(dragEvent(recipients(['ana@example.test'], { composer: 'other', field: 'to' }), body));

    expect(addToTo).toHaveBeenCalledWith(['ana@example.test']);
  });

  it('leaves mail drags alone', () => {
    const { body } = composer();
    const { drop } = setup();
    const event = dragEvent({ [THREAD_DRAG_TYPE]: JSON.stringify({ kind: 'threads', ids: ['th-1'] }) }, body);

    drop.onDragEnter(event);
    drop.onDrop(event);

    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(drop.dragActive.value).toBe(false);
  });

  it('lights nothing up and opens nothing while refused', () => {
    const { body } = composer();
    const { onActiveChange, drop } = setup(true);
    const event = dragEvent(recipients(['ana@example.test']), body);

    drop.onDragEnter(event);

    expect(drop.dragActive.value).toBe(false);
    expect(drop.overBody.value).toBe(false);
    expect(onActiveChange).not.toHaveBeenCalled();
    // Still kept out of the editor.
    expect(event.preventDefault).toHaveBeenCalled();
    expect(event.dataTransfer?.dropEffect).toBe('none');
  });

  it('refuses while the message is sending', () => {
    const { body } = composer();
    const { addToTo, drop } = setup(true);
    const event = dragEvent(recipients(['ana@example.test']), body);

    drop.onDragOver(event);
    expect(event.dataTransfer?.dropEffect).toBe('none');
    drop.onDrop(event);
    expect(addToTo).not.toHaveBeenCalled();
  });
});
