// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import { KEYBINDING_DEFINITIONS } from '@/config/keybindings';
import { useUiStore } from '@/stores/ui';
import { useSettingsStore } from '@/stores/settings';
import ShortcutHelper from './ShortcutHelper.vue';

vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

const helpers: VueWrapper[] = [];

function open() {
  const wrapper = mount(ShortcutHelper, {
    attachTo: document.body,
    global: { stubs: { Teleport: true, Transition: false } },
  });
  helpers.push(wrapper);
  return wrapper;
}

beforeEach(() => {
  setActivePinia(createPinia());
  useUiStore().shortcutsOpen = true;
});

afterEach(() => {
  while (helpers.length > 0) helpers.pop()?.unmount();
});

describe('shortcut helper', () => {
  /**
   * The groups were typed out by hand and drifted: `previewAttachment` was
   * bound and implemented but missing here, while `Space` was offered as a
   * third way to open a row, which it never was.
   */
  it('lists every action the registry knows', () => {
    const text = open().get('.groups').text();

    for (const definition of KEYBINDING_DEFINITIONS) {
      expect(text, definition.action).toContain(definition.label);
    }
    expect(text).toContain('Preview the focused attachment');
  });

  it('does not offer Space as a way to open a row', () => {
    const rows = open()
      .findAll('.groups li')
      .filter((row) => row.text().includes('Open the focused item'));

    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) expect(row.get('kbd').text()).not.toContain('Space');
  });

  /** A rebound key has to show up as the key the user actually presses. */
  it('reads the configured binding, not the default', () => {
    const settings = useSettingsStore();
    settings.keybindings = { ...settings.keybindings, archive: 'y' };

    const archiveRow = open()
      .findAll('.groups li')
      .find((row) => row.text().includes('Archive'));

    expect(archiveRow?.get('kbd').text()).toBe('y');
  });

  /** With the switch off a bare key does nothing, so the help must not offer it. */
  it('leaves out bare keys while single-key shortcuts are off', () => {
    const settings = useSettingsStore();
    settings.singleKeyShortcuts = false;
    settings.keybindings = { ...settings.keybindings, markDone: 'mod+d' };

    const rows = open().findAll('.groups li');
    const keysFor = (label: string) => rows
      .filter((row) => row.text().includes(label))
      .map((row) => row.get('kbd').text());

    expect(keysFor('Archive')).toEqual([]);
    expect(keysFor('Mark as done')).toEqual(['Ctrl+D']);
    expect(keysFor('Show keyboard help')).toEqual(['?']);
  });

  /** The help used to say "Cmd+…" on every platform. */
  it('names the modifier of the platform it runs on', () => {
    const text = open().get('.groups').text();

    expect(navigator.platform).not.toMatch(/mac/i);
    expect(text).toContain('Ctrl+K');
    expect(text).not.toContain('Cmd+');
  });
});
