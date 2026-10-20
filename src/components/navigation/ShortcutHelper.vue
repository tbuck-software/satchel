<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useUiStore } from '@/stores/ui';
import { useSettingsStore } from '@/stores/settings';
import { computed } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import {
  formatBinding,
  keybindingEnabled,
  KEYBINDING_CONTEXT_LABELS,
  KEYBINDING_DEFINITIONS,
  type KeybindingContext,
} from '@/config/keybindings';
import ModalShell from '@/components/ui/ModalShell.vue';

const { t } = useI18n();
const ui = useUiStore();
const settings = useSettingsStore();
const router = useRouter();

interface ShortcutGroup {
  title: string;
  items: Array<[string, string]>;
}

const commandPaletteKeyLabel = formatBinding('mod+k');

/** The configured binding, or '' while the single-key switch silences it. */
function rawBinding(action: string): string {
  const configured = settings.keybindings[action] ?? '';
  return keybindingEnabled(action, configured, settings.singleKeyShortcuts) ? configured : '';
}

function binding(action: string): string {
  return formatBinding(rawBinding(action));
}

function either(...labels: string[]): string {
  return labels.filter(Boolean).join(` ${t('shortcuts.keys.or')} `);
}

/** The order the registry's contexts are read in, widest reach first. */
const CONTEXT_ORDER: readonly KeybindingContext[] = [
  'global',
  'navigation',
  'mailbox',
  'mail',
];

/**
 * The rebindable half comes from `KEYBINDING_DEFINITIONS`.
 *
 * It used to be typed out here, and drifted: `previewAttachment` was
 * implemented and bound but missing from the help entirely, while the help
 * offered `Space` as a third way to open a row, which it is not. A hand-kept
 * copy of a registry is a copy that goes wrong.
 */
/** The action's own key and the keys of its aliases, e.g. `d or e` for Done. */
function keysFor(action: string): string {
  const aliases = KEYBINDING_DEFINITIONS
    .filter((definition) => definition.aliasOf === action)
    .map((definition) => binding(definition.action));
  return either(binding(action), ...aliases);
}

const registeredGroups = computed<ShortcutGroup[]>(() =>
  CONTEXT_ORDER.map((context) => ({
    title: KEYBINDING_CONTEXT_LABELS[context],
    items: KEYBINDING_DEFINITIONS
      // An alias is one more key for an action, not a second action.
      .filter((definition) => definition.context === context && !definition.aliasOf)
      .map((definition) => [keysFor(definition.action), definition.label] as [string, string])
      .filter(([key]) => key !== ''),
  })).filter((group) => group.items.length > 0),
);

/**
 * The other half: combinations that are wired to a key, not to an action, and
 * therefore cannot be rebound. They are listed by hand because there is no
 * registry to read them from — but none of them duplicates a rebindable one.
 */
const fixedGroups = computed<ShortcutGroup[]>(() => {
  const next = binding('nextThread');
  const prev = binding('prevThread');
  // Shift extends a bare letter binding; see `matchesRangeNavigation`.
  const shifted = (action: string) => formatBinding(`shift+${rawBinding(action)}`);
  const rangeKeys = next && prev ? `${shifted('nextThread')} / ${shifted('prevThread')}` : '';
  const readerKeys = next && prev ? `${next} / ${prev} ${t('shortcuts.keys.inReader')}` : '';
  const readerThreadKeys = rangeKeys ? `${rangeKeys} ${t('shortcuts.keys.inReader')}` : '';
  const items: Array<[string, string]> = [
    ['↑ / ↓', t('shortcuts.helper.moveThroughMail')],
    [either(`${formatBinding('shift+arrowup')} / ${formatBinding('shift+arrowdown')}`, rangeKeys), t('shortcuts.helper.selectRange')],
    [readerKeys, t('shortcuts.helper.moveThroughMessages')],
    [readerThreadKeys, t('shortcuts.helper.prevNextThread')],
    [formatBinding('enter'), t('shortcuts.actions.openItem.label')],
    [formatBinding('escape'), t('shortcuts.actions.leaveItem.label')],
    [either(formatBinding('backspace'), formatBinding('delete')), t('common.delete')],
    [formatBinding('mod+z'), t('shortcuts.actions.undo.label')],
  ];
  return [
    {
      title: t('shortcuts.groups.more'),
      items: items.filter(([key]) => key !== ''),
    },
    {
      title: t('shortcuts.groups.compose'),
      items: [
        [formatBinding('mod+n'), t('shortcuts.helper.newMail')],
        [formatBinding('mod+enter'), t('ui.send')],
        [formatBinding('mod+shift+t'), t('shortcuts.helper.insertTemplate')],
      ],
    },
    {
      title: t('shortcuts.groups.search'),
      items: [
        [either(formatBinding('mod+k'), formatBinding('mod+f'), '/'), t('shortcuts.actions.openCommandPalette.label')],
        [formatBinding('mod+enter'), t('nav.omni.showAll')],
      ],
    },
  ];
});

const groups = computed<ShortcutGroup[]>(() => [
  ...registeredGroups.value,
  ...fixedGroups.value,
]);

function editBindings() {
  ui.toggleShortcuts();
  void router.push({ name: 'settings', query: { section: 'shortcuts' } });
}
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <ModalShell
        v-if="ui.shortcutsOpen"
        :label="$t('shortcuts.title')"
        width="680px"
        height="auto"
        @dismiss="ui.toggleShortcuts()"
      >
        <section class="helper">
          <header>
            <div>
              <h3>{{ $t('shortcuts.title') }}</h3>
              <p>{{ $t('shortcuts.subtitle') }}</p>
            </div>
            <IconButton class="close-btn" :title="$t('common.close')" :aria-label="$t('shortcuts.close')" @click="ui.toggleShortcuts()">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                <path d="M4 4l8 8M12 4l-8 8"/>
              </svg>
            </IconButton>
          </header>

          <div class="groups">
            <section v-for="group in groups" :key="group.title">
              <h4>{{ group.title }}</h4>
              <ul>
                <li v-for="([key, description], index) in group.items" :key="index">
                  <kbd>{{ key }}</kbd>
                  <span>{{ description }}</span>
                </li>
              </ul>
            </section>
          </div>

          <footer>
            <span><kbd>{{ commandPaletteKeyLabel }}</kbd> {{ $t('shortcuts.footerHint') }}</span>
            <AppButton @click="editBindings" variant="ghost" size="sm">{{ $t('shortcuts.editBindings') }}</AppButton>
          </footer>
        </section>
      </ModalShell>
    </Transition>
  </Teleport>
</template>

<style scoped>
.helper {
  min-height: 0;
  overflow: auto;
  padding: var(--space-xl);
}

.groups {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 18px;
}

/*
 * `align-content: start`: a section shares its grid row with its neighbour,
 * and with the default the taller column's surplus went to this section's
 * heading track, pushing a short list hundreds of pixels below its title.
 */
.groups section {
  display: grid;
  align-content: start;
  gap: 4px;
}

.groups h4 {
  margin: 0 0 4px;
  font-size: var(--font-size-xs);
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-tertiary);
  font-weight: var(--weight-semibold);
}

header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 16px;
}

header p {
  margin: 4px 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

h3 {
  margin: 0;
  font-size: var(--font-size-xl);
  font-weight: var(--weight-bold);
}

.close-btn { width: 28px; }

ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 2px;
}

li {
  display: grid;
  grid-template-columns: 120px 1fr;
  gap: 16px;
  align-items: center;
  padding: 6px 0;
  border-bottom: 1px solid var(--border-color-muted);
}

footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-top: 18px;
  padding-top: 14px;
  border-top: 1px solid var(--border-color-muted);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

footer button {
  flex-shrink: 0;
  border: 1px solid color-mix(in srgb, var(--accent-primary) 35%, var(--border-color));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-primary) 10%, transparent);
  color: var(--accent-primary);
  padding: 7px 10px;
  font: inherit;
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
  cursor: pointer;
}

footer button:hover { background: color-mix(in srgb, var(--accent-primary) 17%, transparent); }

footer span {
  display: flex;
  align-items: center;
  gap: 8px;
}

li:last-child {
  border-bottom: 0;
}

kbd {
  font-family: var(--font-mono);
  font-size: var(--font-size-sm);
  background: var(--bg-raised);
  border-radius: var(--radius-xs);
  padding: 3px 8px;
  color: var(--accent-primary);
  /* "Ctrl+Shift+T" and "Backspace or Del" are wider than the column. */
  white-space: normal;
  overflow-wrap: anywhere;
  max-width: 100%;
  width: fit-content;
}

li span {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

@media (max-width: 720px) {
  .groups {
    grid-template-columns: minmax(0, 1fr);
  }

  li {
    grid-template-columns: 112px minmax(0, 1fr);
  }
}
</style>
