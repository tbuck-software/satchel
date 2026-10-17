import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useCrudActions } from './useCrudActions';
import { useToast } from './useToast';

interface Entry {
  id: string;
  name: string;
}

const entry: Entry = { id: 'sig-1', name: 'Work signature' };

function toasts() {
  return useToast().state.toasts;
}

/** Reads a toast the assertion expects to be there. */
function toastAt(index: number) {
  const toast = toasts().at(index);
  if (!toast) throw new Error(`no toast at ${index}`);
  return toast;
}

describe('useCrudActions', () => {
  beforeEach(() => {
    useToast().clear();
    vi.clearAllMocks();
  });

  it('offers undo for what the delete returned', async () => {
    const crud = useCrudActions();
    const restore = vi.fn(async () => {});

    const removed = await crud.removeWithUndo({
      id: entry.id,
      name: entry.name,
      remove: async () => entry,
      restore,
    });

    expect(removed).toBe(true);
    expect(toasts()).toHaveLength(1);
    expect(toastAt(0).message).toBe('Work signature deleted.');
    expect(toastAt(0).action?.label).toBe('Undo');

    await toastAt(0).action?.handler();

    expect(restore).toHaveBeenCalledWith(entry);
    expect(toastAt(-1).message).toBe('Work signature restored.');
  });

  it('reports a rejected delete with its cause and offers no undo', async () => {
    const crud = useCrudActions();
    const restore = vi.fn(async () => {});

    const removed = await crud.removeWithUndo({
      id: entry.id,
      name: entry.name,
      remove: async () => {
        throw new Error('database is locked');
      },
      restore,
    });

    expect(removed).toBe(false);
    expect(toasts()).toHaveLength(1);
    expect(toastAt(0).tone).toBe('error');
    expect(toastAt(0).message).toBe('Could not delete Work signature: database is locked');
    expect(toastAt(0).action).toBeUndefined();
    expect(restore).not.toHaveBeenCalled();
  });

  it('reports a rejected undo instead of pretending the entry is back', async () => {
    const crud = useCrudActions();

    await crud.removeWithUndo({
      id: entry.id,
      name: entry.name,
      remove: async () => entry,
      restore: async () => {
        throw new Error('write failed');
      },
    });
    await toastAt(0).action?.handler();

    expect(toastAt(-1).tone).toBe('error');
    expect(toastAt(-1).message).toBe('Could not restore Work signature: write failed');
  });

  it('shows no undo button when there was nothing to snapshot', async () => {
    const crud = useCrudActions();

    await crud.removeWithUndo({
      id: entry.id,
      name: entry.name,
      remove: async () => null,
      restore: vi.fn(async () => {}),
    });

    expect(toastAt(0).tone).toBe('info');
    expect(toastAt(0).action).toBeUndefined();
  });

  it('ignores a second click while the first delete is still running', async () => {
    const crud = useCrudActions();
    let release = () => {};
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    const remove = vi.fn(async () => {
      await pending;
      return entry;
    });
    const removal = { id: entry.id, name: entry.name, remove, restore: async () => {} };

    const first = crud.removeWithUndo(removal);
    const second = await crud.removeWithUndo(removal);
    release();
    await first;

    expect(second).toBe(false);
    expect(remove).toHaveBeenCalledTimes(1);
  });
});
