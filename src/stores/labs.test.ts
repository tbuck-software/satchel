import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  available: true,
  backendListLabsFlags: vi.fn(),
  backendSetLabsFlag: vi.fn(),
}));
vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: () => backend.available,
  backendListLabsFlags: backend.backendListLabsFlags,
  backendSetLabsFlag: backend.backendSetLabsFlag,
}));

import { useLabsStore } from './labs';

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  backend.available = true;
});

describe('labs store', () => {
  it('reads every flag as off until the backend says otherwise', async () => {
    const labs = useLabsStore();
    expect(labs.isEnabled('chats')).toBe(false);
    backend.backendListLabsFlags.mockResolvedValue([
      { id: 'chats', enabled: true },
      { id: 'from-a-newer-build', enabled: true },
    ]);
    await labs.load();
    await labs.ready;
    expect(labs.isEnabled('chats')).toBe(true);
    expect(Object.keys(labs.enabled)).toEqual(['chats']);
  });

  it('settles ready without a backend and keeps the defaults', async () => {
    backend.available = false;
    const labs = useLabsStore();
    await labs.load();
    await labs.ready;
    expect(backend.backendListLabsFlags).not.toHaveBeenCalled();
    expect(labs.isEnabled('chats')).toBe(false);
  });

  it('switches at once and puts the old state back when the backend refuses', async () => {
    const labs = useLabsStore();
    let refuse: (error: Error) => void = () => {};
    backend.backendSetLabsFlag.mockReturnValue(new Promise((_, reject) => { refuse = reject; }));
    const pending = labs.setEnabled('chats', true);
    expect(labs.isEnabled('chats')).toBe(true);
    expect(labs.saving).toBe('chats');
    refuse(new Error('nope'));
    await expect(pending).rejects.toThrow('nope');
    expect(labs.isEnabled('chats')).toBe(false);
    expect(labs.saving).toBeNull();
  });
});
