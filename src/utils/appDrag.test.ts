// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  MESSAGE_DRAG_TYPE,
  RECIPIENT_DRAG_TYPE,
  THREAD_DRAG_TYPE,
  draggedThreadIds,
  isThreadDrag,
  startAppDrag,
  startThreadDrag,
} from './appDrag';
import { appDragKind, pendingAppDrag, readAppDrag, resetAppDragForTests, startMessageDrag, watchDragEnd } from './appDragTarget';
import { contactRecipient, startContactDrag } from './contactDrag';

/** A DataTransfer as the browser hands it to `dragstart` and `drop`. */
function transfer(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    get types() { return [...data.keys()]; },
    setData: (type: string, value: string) => { data.set(type, value); },
    getData: (type: string) => data.get(type) ?? '',
    effectAllowed: 'uninitialized',
    dropEffect: 'none',
  };
}

function dragEvent(dataTransfer = transfer(), target: EventTarget = new EventTarget()) {
  return { dataTransfer, target } as unknown as DragEvent;
}

/** What `dragover` sees: the types, but no data. */
function duringDragOver(event: DragEvent) {
  const types = Array.from(event.dataTransfer?.types ?? []);
  return dragEvent({ ...transfer(), get types() { return types; }, getData: () => '' } as never);
}

afterEach(() => {
  vi.useRealTimers();
  resetAppDragForTests();
});

/** A drag the browser starts on its own, as from text selected in the Subject. */
function nativeDragStart() {
  const input = document.createElement('input');
  document.body.append(input);
  input.dispatchEvent(new Event('dragstart', { bubbles: true }));
  input.remove();
}

describe('appDrag', () => {
  it('carries threads under the type folders already read', () => {
    const event = dragEvent();
    startThreadDrag(event, ['th-1', 'th-2']);

    expect(JSON.parse(event.dataTransfer?.getData(THREAD_DRAG_TYPE) ?? '')).toEqual({ kind: 'threads', ids: ['th-1', 'th-2'] });
    expect(event.dataTransfer?.effectAllowed).toBe('copyMove');
    expect(appDragKind(event)).toBe('threads');
    expect(draggedThreadIds(event)).toEqual(['th-1', 'th-2']);
  });

  it('tells a message apart from a thread', () => {
    const event = dragEvent();
    startMessageDrag(event, ['msg-1']);

    expect(event.dataTransfer?.types).toEqual([MESSAGE_DRAG_TYPE]);
    expect(appDragKind(event)).toBe('messages');
    expect(readAppDrag(event)).toEqual({ kind: 'messages', ids: ['msg-1'] });
    // A folder moves threads; a single message is not one.
    expect(draggedThreadIds(event)).toEqual([]);
  });

  it('gives contacts as recipients, with the address as text for other apps', () => {
    const event = dragEvent();
    startContactDrag(event, [{ email: 'ana@example.test', name: 'Ana Example' }, { email: '  ' }]);

    expect(event.dataTransfer?.getData('text/plain')).toBe('Ana Example <ana@example.test>');
    expect(event.dataTransfer?.effectAllowed).toBe('copy');
    expect(readAppDrag(event)).toEqual({
      kind: 'recipients',
      addresses: ['Ana Example <ana@example.test>'],
      source: null,
    });
  });

  it('writes a contact the way the suggestion list does', () => {
    expect(contactRecipient({ email: 'bo@example.test', name: 'Bo' })).toBe('Bo <bo@example.test>');
    expect(contactRecipient({ email: 'bo@example.test', name: 'BO@example.test' })).toBe('bo@example.test');
    expect(contactRecipient({ email: 'bo@example.test', name: null })).toBe('bo@example.test');
  });

  it('knows a chip’s origin before the drop, while getData is still empty', () => {
    const start = dragEvent();
    startAppDrag(start, { kind: 'recipients', addresses: ['cy@example.test'], source: { composer: 'c1', field: 'to' } });

    const over = duringDragOver(start);
    expect(appDragKind(over)).toBe('recipients');
    expect(pendingAppDrag(over)).toMatchObject({ source: { composer: 'c1', field: 'to' } });
    expect(start.dataTransfer?.effectAllowed).toBe('copyMove');
  });

  it('resolves a drop through the registry when the webview hid the custom types', () => {
    startMessageDrag(dragEvent(), ['msg-9']);
    const hidden = dragEvent(transfer());

    expect(appDragKind(hidden)).toBe('messages');
    expect(readAppDrag(hidden)).toEqual({ kind: 'messages', ids: ['msg-9'] });
  });

  it('never mistakes files or another application’s drag for its own', () => {
    startMessageDrag(dragEvent(), ['msg-9']);

    expect(appDragKind(dragEvent(transfer({ Files: '' })))).toBeNull();
    expect(appDragKind(dragEvent(transfer({ 'text/html': '<b>x</b>' })))).toBeNull();
  });

  it('reads nothing but threads for a folder', () => {
    expect(draggedThreadIds(dragEvent(transfer({ [THREAD_DRAG_TYPE]: 'th-1,th-2' })))).toEqual([]);
    expect(draggedThreadIds(dragEvent(transfer({ [THREAD_DRAG_TYPE]: '{"kind":"messages","ids":["m"]}' })))).toEqual([]);
    expect(isThreadDrag(dragEvent(transfer({ Files: '' })))).toBe(false);
  });

  it('forgets the drag once its source sees it end', () => {
    const source = document.createElement('div');
    document.body.append(source);
    startMessageDrag(dragEvent(transfer(), source), ['msg-9']);
    source.dispatchEvent(new Event('dragend', { bubbles: true }));
    source.remove();

    expect(appDragKind(dragEvent(transfer()))).toBeNull();
  });

  it('never takes text dragged inside the window for the last drag', () => {
    startAppDrag(dragEvent(), { kind: 'recipients', addresses: ['ana@example.test'], source: null });
    const hidden = dragEvent(transfer({ 'text/plain': 'ana@example.test' }));
    expect(appDragKind(hidden)).toBe('recipients');

    nativeDragStart();

    const text = dragEvent(transfer({ 'text/plain': 'Quarterly report' }));
    expect(appDragKind(text)).toBeNull();
    expect(readAppDrag(text)).toBeNull();
  });

  it('starts over with each drag of its own', () => {
    const start = dragEvent();
    // Capture first, as the browser dispatches it, then the source's handler.
    nativeDragStart();
    startThreadDrag(start, ['th-1']);
    expect(appDragKind(dragEvent(transfer()))).toBe('threads');
  });

  it('forgets the drag after any drop or drag end in the window', async () => {
    startMessageDrag(dragEvent(), ['msg-9']);
    window.dispatchEvent(new Event('drop'));
    // Drop handlers still read it in the same turn.
    expect(appDragKind(dragEvent(transfer()))).toBe('messages');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(appDragKind(dragEvent(transfer()))).toBeNull();

    startMessageDrag(dragEvent(), ['msg-9']);
    window.dispatchEvent(new Event('dragend'));
    expect(appDragKind(dragEvent(transfer()))).toBeNull();
  });

  it('lets a drag whose end never arrived expire', () => {
    vi.useFakeTimers();
    startMessageDrag(dragEvent(), ['msg-9']);
    vi.advanceTimersByTime(60_000);
    expect(appDragKind(dragEvent(transfer()))).toBe('messages');
    vi.advanceTimersByTime(61_000);
    expect(appDragKind(dragEvent(transfer()))).toBeNull();
  });

  it('reads the memory with Satchel types only for the same kind', () => {
    startMessageDrag(dragEvent(), ['msg-9']);
    const recipients = dragEvent(transfer({ [RECIPIENT_DRAG_TYPE]: '' }));
    expect(appDragKind(recipients)).toBe('recipients');
    expect(pendingAppDrag(recipients)).toBeNull();
  });

  it('gives up on a drag whose end never comes at the next pointer move, key or timeout', () => {
    vi.useFakeTimers();
    for (const finish of [
      () => window.dispatchEvent(new Event('pointermove')),
      () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' })),
      () => vi.advanceTimersByTime(60_000),
    ]) {
      startMessageDrag(dragEvent(), ['msg-9']);
      const recover = vi.fn();
      watchDragEnd(recover);
      finish();
      expect(recover).toHaveBeenCalledOnce();
      expect(appDragKind(dragEvent(transfer()))).toBeNull();
      // Once only: the listeners are gone.
      window.dispatchEvent(new Event('pointermove'));
      expect(recover).toHaveBeenCalledOnce();
    }
  });

  it('stops watching when the drag ends properly', () => {
    vi.useFakeTimers();
    const recover = vi.fn();
    const stop = watchDragEnd(recover);
    stop();
    window.dispatchEvent(new Event('pointermove'));
    vi.advanceTimersByTime(60_000);
    expect(recover).not.toHaveBeenCalled();
  });

  it('resolves a drag another window announced, until a drop here has read it', async () => {
    const otherWindow = new BroadcastChannel('satchel:app-drag');
    otherWindow.postMessage({ kind: 'messages', ids: ['msg-elsewhere'] });
    await vi.waitFor(() => expect(readAppDrag(dragEvent(transfer()))).toEqual({ kind: 'messages', ids: ['msg-elsewhere'] }));

    window.dispatchEvent(new Event('drop'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(appDragKind(dragEvent(transfer()))).toBeNull();
    otherWindow.close();
  });

  it('ignores a payload it cannot read', () => {
    const event = dragEvent(transfer({ [RECIPIENT_DRAG_TYPE]: '{"kind":"recipients","addresses":[1,""]}' }));
    expect(readAppDrag(event)).toBeNull();
    const wrongKind = dragEvent(transfer({ [RECIPIENT_DRAG_TYPE]: '{"kind":"threads","ids":["th-1"]}' }));
    expect(readAppDrag(wrongKind)).toBeNull();
    const broken = dragEvent(transfer({ [RECIPIENT_DRAG_TYPE]: 'not json' }));
    expect(readAppDrag(broken)).toBeNull();
  });
});
