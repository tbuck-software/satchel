// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { registerThreadReaderCommands } from './useThreadReaderCommands';
import { useCommands } from './useCommands';
import { COMMAND_ICONS } from '@/components/navigation/commandIcons';

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
    expect(command('thread:done')?.shortcut).toEqual({ label: 'd', keys: ['d'], aria: 'D' });
    expect(command('thread:reply-all')?.shortcut?.label).toBe('Shift+A');
  });

  it('follows a rebinding instead of a hard-coded key', () => {
    useSettingsStore().setKeybinding('reply', 'mod+r');

    expect(command('thread:reply')?.shortcut).toEqual({ label: 'Ctrl+R', keys: ['Ctrl+R'], aria: 'Control+R' });
  });

  it('hides bare keys while single-key shortcuts are off', () => {
    useSettingsStore().singleKeyShortcuts = false;

    expect(command('thread:done')?.shortcut).toBeNull();
    expect(command('compose:new')?.shortcut?.label).toBe('Ctrl+N');
    expect(command('help:shortcuts')?.shortcut?.label).toBe('?');
  });
});

describe('palette icons', () => {
  /*
   * Every row used to lead with the same '>' chevron, which promised a
   * submenu none of them had. A command wears the glyph of the button it
   * stands in for, so a missing one shows up here rather than as a gap.
   */
  it('gives every command an icon from the palette set', () => {
    for (const entry of useCommands().commands.value) {
      expect(entry.icon, entry.id).toBeDefined();
      expect(COMMAND_ICONS[entry.icon!], entry.id).toBeDefined();
    }
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

describe('views in the palette', () => {
  /** Typing a view's name found nothing for Drafts, Snoozed or Done. */
  it('offers every view with a g key, under its sidebar name and its key', () => {
    const titles = new Map(useCommands().commands.value.map((entry) => [entry.id, entry]));

    expect(titles.get('view:drafts')?.title).toBe('Go to Drafts');
    expect(titles.get('view:done')?.title).toBe('Go to Done');
    expect(titles.get('view:later')?.title).toBe('Go to Later');
    expect(titles.get('view:later')?.shortcut?.keys).toEqual(['g', 'l']);
    // The old names still find the merged view.
    expect(titles.get('view:later')?.keywords).toContain('Snoozed');
    expect(titles.get('view:later')?.keywords).toContain('Waiting for a reply');
  });

  it('offers File… instead of a topic-only entry', () => {
    expect(command('thread:topic')).toBeUndefined();
    expect(command('thread:file')?.title).toBe('File…');
    expect(command('thread:file')?.shortcut?.keys).toEqual(['v']);
  });
});
