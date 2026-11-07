// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { installFileDropGuard } from './fileDropGuard';

function drag(type: 'dragover' | 'drop', types: string[]) {
  const event = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
  Object.defineProperty(event, 'dataTransfer', { value: { types, dropEffect: 'copy' } });
  return event;
}

let dispose: (() => void) | null = null;
afterEach(() => {
  dispose?.();
  dispose = null;
  document.body.replaceChildren();
});

describe('installFileDropGuard', () => {
  it('answers an unclaimed file drag with "not allowed" and keeps the file from opening', () => {
    dispose = installFileDropGuard();
    const over = drag('dragover', ['Files']);
    document.body.dispatchEvent(over);
    expect(over.defaultPrevented).toBe(true);
    expect(over.dataTransfer?.dropEffect).toBe('none');

    const drop = drag('drop', ['Files']);
    document.body.dispatchEvent(drop);
    expect(drop.defaultPrevented).toBe(true);
  });

  it('leaves a file drag a drop zone accepted alone', () => {
    dispose = installFileDropGuard();
    const zone = document.createElement('div');
    zone.addEventListener('dragover', (event) => event.preventDefault());
    document.body.append(zone);
    const over = drag('dragover', ['Files']);
    zone.dispatchEvent(over);
    expect(over.dataTransfer?.dropEffect).toBe('copy');
  });

  it('leaves drags without files to their targets', () => {
    dispose = installFileDropGuard();
    const over = drag('dragover', ['text/plain']);
    document.body.dispatchEvent(over);
    expect(over.defaultPrevented).toBe(false);
  });
});
