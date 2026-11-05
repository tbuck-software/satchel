import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useUiStore } from '@/stores/ui';
import type { SettingsSection } from '@/components/settings/settingsSections';

const firstString = (value: unknown): string | null => {
  const first = Array.isArray(value) ? value[0] : value;
  return typeof first === 'string' ? first : null;
};

/**
 * Where in Settings the user is, and how to go elsewhere in it.
 *
 * Two pane: Settings is a dialog over the mail, and its place is UI state
 * (`ui.settingsSection`). Single pane: Settings is the `/settings` route, and
 * its place is the query, so Back walks out of a section. Sections, the
 * section list and account setup read the same place either way.
 */
export function useSettingsLocation() {
  const route = useRoute();
  const router = useRouter();
  const ui = useUiStore();

  const inDialog = computed(() => ui.settingsOpen && !ui.isSinglePane);
  const section = computed(() => (inDialog.value ? ui.settingsSection : firstString(route?.query.section)));
  const account = computed(() => (inDialog.value ? ui.settingsAccount : firstString(route?.query.account)));

  /** Show a section. `push` adds a history step on a single pane (overview → section). */
  function show(next: SettingsSection, mode: 'push' | 'replace' = 'replace') {
    if (inDialog.value) {
      ui.settingsSection = next;
      ui.settingsAccount = null;
      return;
    }
    const navigation = { name: 'settings' as const, query: { section: next } };
    void (mode === 'push' ? router.push(navigation) : router.replace(navigation));
  }

  return { inDialog, section, account, show };
}
