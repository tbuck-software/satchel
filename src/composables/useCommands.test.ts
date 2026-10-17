// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { registerThreadReaderCommands } from './useThreadReaderCommands';
import { useCommands } from './useCommands';

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn() }),
  useRoute: () => ({ name: 'inbox', params: {}, query: {} }),
}));

function command(id: string) {
  return useCommands().commands.value.find((entry) => entry.id === id);
}

beforeEach(() => {
  window.localStorage.clear();
  setActivePinia(createPinia());
  const threads = useThreadsStore();
  vi.spyOn(threads, 'activeThreadId', 'get').mockReturnValue('thread-1');
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('palette shortcut hints', () => {
  /** `D` reads as Shift+D in registry notation; the binding is a plain `d`. */
  it('shows the binding as it is pressed', () => {
    expect(command('thread:done')?.shortcut).toEqual({ label: 'd', aria: 'D' });
    expect(command('thread:reply-all')?.shortcut?.label).toBe('Shift+A');
  });

  it('follows a rebinding instead of a hard-coded key', () => {
    useSettingsStore().setKeybinding('reply', 'mod+r');

    expect(command('thread:reply')?.shortcut).toEqual({ label: 'Ctrl+R', aria: 'Control+R' });
  });

  it('hides bare keys while single-key shortcuts are off', () => {
    useSettingsStore().singleKeyShortcuts = false;

    expect(command('thread:done')?.shortcut).toBeNull();
    expect(command('compose:new')?.shortcut?.label).toBe('Ctrl+N');
    expect(command('help:shortcuts')?.shortcut?.label).toBe('?');
  });
});

describe('reader actions in the palette', () => {
  it('offers the More menu actions for the thread in the reader', () => {
    const reader = {
      threadId: 'thread-1',
      moveTargets: [{ id: 'folder-1', label: 'Receipts' }],
      isMuted: false,
      move: vi.fn(),
      toggleMuted: vi.fn(),
      markSpam: vi.fn(),
      blockSender: vi.fn(),
      print: vi.fn(),
      exportThread: vi.fn(),
    };
    const unregister = registerThreadReaderCommands(reader);

    for (const id of ['thread:mute', 'thread:spam', 'thread:block-address', 'thread:print-conversation', 'thread:export-eml']) {
      expect(command(id), id).toBeTruthy();
    }
    const move = command('thread:move:folder-1');
    expect(move?.title).toBe('Move to Receipts');
    expect(move?.searchOnly).toBe(true);

    void command('thread:print-conversation')?.run();
    void move?.run();
    expect(reader.print).toHaveBeenCalledWith('conversation');
    expect(reader.move).toHaveBeenCalledWith('folder-1');
    unregister();
    expect(command('thread:print-conversation')).toBeUndefined();
  });
});
