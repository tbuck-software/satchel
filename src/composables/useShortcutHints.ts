import { useSettingsStore } from '@/stores/settings';
import { shortcutHint, type ShortcutHint } from '@/config/keybindings';

export type { ShortcutHint } from '@/config/keybindings';

/** Shortcut hints for the current bindings and single-key switch. */
export function useShortcutHints() {
  const settings = useSettingsStore();

  function hint(action: string): ShortcutHint | null {
    return shortcutHint(action, {
      keybindings: settings.keybindings,
      singleKeyShortcuts: settings.singleKeyShortcuts,
    });
  }

  return { hint };
}
