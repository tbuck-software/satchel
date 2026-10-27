// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { useSettingsStore } from '@/stores/settings';
import {
  ariaKeyshortcuts,
  bindingKeycaps,
  DEFAULT_KEYBINDINGS,
  effectiveBindings,
  formatBinding,
  isSingleKeyBinding,
  keybindingConflicts,
  keybindingContextActive,
  keybindingEnabled,
  keyboardEventStroke,
  matchesKeybinding,
  shortcutHint,
} from './keybindings';

describe('keybindings', () => {
  beforeEach(() => {
    window.localStorage.clear();
    setActivePinia(createPinia());
  });

  it('normalizes modifier and shifted letter strokes', () => {
    expect(keyboardEventStroke(new KeyboardEvent('keydown', { key: 'A', shiftKey: true }))).toBe('A');
    expect(keyboardEventStroke(new KeyboardEvent('keydown', { key: 'k', metaKey: true }))).toBe('mod+k');
    expect(matchesKeybinding(new KeyboardEvent('keydown', { key: 'A', shiftKey: true }), 'A')).toBe(true);
  });

  it('does not treat a sequence as a direct single-key binding', () => {
    expect(matchesKeybinding(new KeyboardEvent('keydown', { key: 'g' }), 'g i')).toBe(false);
  });

  it('loads, persists and resets custom bindings', async () => {
    window.localStorage.setItem('satchel.keybindings', JSON.stringify({ nextThread: 'n', unknown: 'z' }));
    const settings = useSettingsStore();
    expect(settings.keybindings.nextThread).toBe('n');
    expect(settings.keybindings.openItem).toBe(DEFAULT_KEYBINDINGS.openItem);
    expect(settings.keybindings.unknown).toBeUndefined();

    settings.setKeybinding('nextThread', 'arrowdown');
    await nextTick();
    expect(JSON.parse(window.localStorage.getItem('satchel.keybindings') ?? '{}').nextThread).toBe('arrowdown');

    settings.resetKeybindings();
    expect(settings.keybindings).toEqual(DEFAULT_KEYBINDINGS);
  });
});

describe('keybinding contexts', () => {
  it('keeps global and navigation bindings on every page', () => {
    for (const route of ['inbox', 'settings', 'calendar', 'contacts', 'drafts', 'search']) {
      expect(keybindingContextActive('global', { route, covered: false }), route).toBe(true);
      expect(keybindingContextActive('navigation', { route, covered: false }), route).toBe(true);
    }
  });

  it('limits mailbox and mail bindings to mail lists and the reader', () => {
    for (const context of ['mailbox', 'mail'] as const) {
      for (const route of ['inbox', 'thread', 'search', 'topic', 'new-senders']) {
        expect(keybindingContextActive(context, { route, covered: false }), `${context} on ${route}`).toBe(true);
      }
      for (const route of ['settings', 'calendar', 'contacts', 'drafts', null]) {
        expect(keybindingContextActive(context, { route, covered: false }), `${context} on ${route}`).toBe(false);
      }
    }
  });

  it('turns every context off while something covers the page', () => {
    for (const context of ['global', 'navigation', 'mailbox', 'mail'] as const) {
      expect(keybindingContextActive(context, { route: 'inbox', covered: true })).toBe(false);
    }
  });
});

describe('single-key switch', () => {
  it('only silences bindings a typed letter could trigger', () => {
    expect(isSingleKeyBinding('e')).toBe(true);
    expect(isSingleKeyBinding('A')).toBe(true);
    expect(isSingleKeyBinding('g i')).toBe(true);
    expect(isSingleKeyBinding('mod+e')).toBe(false);
    expect(isSingleKeyBinding('alt+e')).toBe(false);

    expect(keybindingEnabled('archive', 'e', false)).toBe(false);
    expect(keybindingEnabled('archive', 'mod+e', false)).toBe(true);
    expect(keybindingEnabled('help', '?', false)).toBe(true);
    expect(keybindingEnabled('archive', 'e', true)).toBe(true);
  });

  it('falls back to a fixed key, or to nothing, once the bare key is off', () => {
    const off = { keybindings: DEFAULT_KEYBINDINGS, singleKeyShortcuts: false };
    expect(effectiveBindings('compose', off)).toEqual(['mod+n']);
    expect(effectiveBindings('trash', off)).toEqual(['backspace', 'delete']);
    expect(effectiveBindings('archive', off)).toEqual([]);
    expect(shortcutHint('archive', off)).toBeNull();
    expect(shortcutHint('compose', { ...off, singleKeyShortcuts: true }, 'MacIntel')).toEqual({ label: 'n', keys: ['n'], aria: 'N' });
  });
});

describe('shortcut labels', () => {
  it('uses symbols on Apple systems and words elsewhere', () => {
    expect(formatBinding('mod+k', 'MacIntel')).toBe('⌘K');
    expect(formatBinding('mod+k', 'Win32')).toBe('Ctrl+K');
    expect(formatBinding('mod+shift+t', 'MacIntel')).toBe('⇧⌘T');
    expect(formatBinding('mod+shift+t', 'Linux x86_64')).toBe('Ctrl+Shift+T');
    expect(formatBinding('mod+enter', 'Win32')).toBe('Ctrl+Enter');
  });

  /** Uppercase means Shift in the registry; a bare `d` must not read as `D`. */
  it('keeps bare letters lowercase and spells out Shift', () => {
    expect(formatBinding('d', 'MacIntel')).toBe('d');
    expect(formatBinding('A', 'MacIntel')).toBe('⇧A');
    expect(formatBinding('A', 'Win32')).toBe('Shift+A');
    expect(formatBinding('g i', 'MacIntel')).toBe('g i');
  });

  /** A cap is a whole chord; a sequence is one cap per stroke. */
  it('splits a binding into keycaps, one per stroke', () => {
    expect(bindingKeycaps('mod+shift+t', 'MacIntel')).toEqual(['⇧⌘T']);
    expect(bindingKeycaps('mod+enter', 'Win32')).toEqual(['Ctrl+Enter']);
    expect(bindingKeycaps('alt+d', 'MacIntel')).toEqual(['⌥D']);
    expect(bindingKeycaps('g i', 'MacIntel')).toEqual(['g', 'i']);
    expect(bindingKeycaps('')).toEqual([]);
  });

  it('describes single strokes for aria-keyshortcuts and skips sequences', () => {
    expect(ariaKeyshortcuts('mod+k', 'MacIntel')).toBe('Meta+K');
    expect(ariaKeyshortcuts('mod+k', 'Win32')).toBe('Control+K');
    expect(ariaKeyshortcuts('A')).toBe('Shift+A');
    expect(ariaKeyshortcuts('#')).toBe('#');
    expect(ariaKeyshortcuts('space')).toBe('Space');
    expect(ariaKeyshortcuts('g i')).toBeUndefined();
  });

  it('follows a rebinding', () => {
    const state = { keybindings: { ...DEFAULT_KEYBINDINGS, markDone: 'mod+d' }, singleKeyShortcuts: false };
    expect(shortcutHint('markDone', state, 'MacIntel')).toEqual({ label: '⌘D', keys: ['⌘D'], aria: 'Meta+D' });
  });
});

describe('keybinding conflicts', () => {
  it('reports nothing for the defaults', () => {
    expect([...keybindingConflicts(DEFAULT_KEYBINDINGS).keys()]).toEqual([]);
  });

  it('reports two actions on one key and a key that shadows a sequence', () => {
    const conflicts = keybindingConflicts({ ...DEFAULT_KEYBINDINGS, archive: 'r', markDone: 'g' });
    expect(conflicts.get('archive')?.actions).toEqual(['reply']);
    expect(conflicts.get('markDone')?.actions).toContain('goInbox');
  });

  /** These keys are handled before any rebindable action and would swallow it. */
  it.each(['/', 'arrowdown', 'delete', 'backspace', 'enter', 'mod+k', 'mod+f', 'mod+n', 'mod+z'])(
    'reports a binding to the fixed key %s',
    (binding) => {
      const conflicts = keybindingConflicts({ ...DEFAULT_KEYBINDINGS, archive: binding });
      expect(conflicts.get('archive')?.fixed).toEqual([binding]);
    },
  );

  it('accepts a fixed key that already triggers the same action', () => {
    expect(keybindingConflicts({ ...DEFAULT_KEYBINDINGS, trash: 'delete' }).has('trash')).toBe(false);
  });
});
