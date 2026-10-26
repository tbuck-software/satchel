<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import { useUiStore } from '@/stores/ui';
import { useSettingsStore } from '@/stores/settings';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import {
  bindingKeycaps,
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

/** The caps of one binding: `['⌘K']`, or `['g', 'i']` for a sequence. */
type KeyChord = string[];

/**
 * One row of the help. `keys` lists alternatives, read "d or e"; an
 * alternative is usually one chord, or a pair such as ↑ / ↓ that only makes
 * sense together.
 */
interface ShortcutRow {
  keys: KeyChord[][];
  description: string;
  /** Where the keys apply, when that is narrower than the group. */
  scope?: string;
}

interface ShortcutGroup {
  title: string;
  items: ShortcutRow[];
}

const commandPaletteKeys = bindingKeycaps('mod+k');

/** The configured binding, or '' while the single-key switch silences it. */
function rawBinding(action: string): string {
  const configured = settings.keybindings[action] ?? '';
  return keybindingEnabled(action, configured, settings.singleKeyShortcuts) ? configured : '';
}

/** Alternatives of one chord each; unbound ones fall away. */
function either(...bindings: string[]): KeyChord[][] {
  return bindings.map((binding) => bindingKeycaps(binding)).filter((chord) => chord.length > 0).map((chord) => [chord]);
}

/** One alternative made of a pair, or nothing while either half is unbound. */
function pair(first: string, second: string): KeyChord[][] {
  return first && second ? [[bindingKeycaps(first), bindingKeycaps(second)]] : [];
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
function keysFor(action: string): KeyChord[][] {
  const aliases = KEYBINDING_DEFINITIONS
    .filter((definition) => definition.aliasOf === action)
    .map((definition) => rawBinding(definition.action));
  return either(rawBinding(action), ...aliases);
}

const registeredGroups = computed<ShortcutGroup[]>(() =>
  CONTEXT_ORDER.map((context) => ({
    title: KEYBINDING_CONTEXT_LABELS[context],
    items: KEYBINDING_DEFINITIONS
      // An alias is one more key for an action, not a second action.
      .filter((definition) => definition.context === context && !definition.aliasOf)
      .map((definition): ShortcutRow => ({ keys: keysFor(definition.action), description: definition.label }))
      .filter((row) => row.keys.length > 0),
  })).filter((group) => group.items.length > 0),
);

/**
 * The other half: combinations that are wired to a key, not to an action, and
 * therefore cannot be rebound. They are listed by hand because there is no
 * registry to read them from — but none of them duplicates a rebindable one.
 */
const fixedGroups = computed<ShortcutGroup[]>(() => {
  const next = rawBinding('nextThread');
  const prev = rawBinding('prevThread');
  // Shift extends a bare letter binding; see `matchesRangeNavigation`.
  const shifted = (binding: string) => (binding ? `shift+${binding}` : '');
  const inReader = t('shortcuts.keys.inReader');
  const items: ShortcutRow[] = [
    { keys: pair('arrowup', 'arrowdown'), description: t('shortcuts.helper.moveThroughMail') },
    {
      keys: [...pair('shift+arrowup', 'shift+arrowdown'), ...pair(shifted(next), shifted(prev))],
      description: t('shortcuts.helper.selectRange'),
    },
    { keys: pair(next, prev), description: t('shortcuts.helper.moveThroughMessages'), scope: inReader },
    { keys: pair(shifted(next), shifted(prev)), description: t('shortcuts.helper.prevNextThread'), scope: inReader },
    { keys: either('enter'), description: t('shortcuts.actions.openItem.label') },
    { keys: either('escape'), description: t('shortcuts.actions.leaveItem.label') },
    { keys: either('backspace', 'delete'), description: t('common.delete') },
    { keys: either('mod+z'), description: t('shortcuts.actions.undo.label') },
  ];
  return [
    {
      title: t('shortcuts.groups.more'),
      items: items.filter((row) => row.keys.length > 0),
    },
    {
      title: t('shortcuts.groups.compose'),
      items: [
        { keys: either('mod+n'), description: t('shortcuts.helper.newMail') },
        { keys: either('mod+enter'), description: t('ui.send') },
        { keys: either('mod+shift+t'), description: t('shortcuts.helper.insertTemplate') },
      ],
    },
    {
      title: t('shortcuts.groups.search'),
      items: [
        { keys: either('mod+k', 'mod+f', '/'), description: t('shortcuts.actions.openCommandPalette.label') },
        { keys: either('mod+enter'), description: t('nav.omni.showAll') },
      ],
    },
  ];
});

const groups = computed<ShortcutGroup[]>(() => [
  ...registeredGroups.value,
  ...fixedGroups.value,
]);

/*
 * The list outgrows a laptop screen. It scrolls between a fixed header and
 * footer, and fades out at whichever edge has more behind it — a list cut
 * flush at a row boundary looks finished.
 */
const scroller = ref<HTMLElement | null>(null);
const moreAbove = ref(false);
const moreBelow = ref(false);
let resizeObserver: ResizeObserver | null = null;

function updateFades() {
  const el = scroller.value;
  if (!el) return;
  moreAbove.value = el.scrollTop > 1;
  moreBelow.value = el.scrollTop + el.clientHeight < el.scrollHeight - 1;
}

watch(scroller, (el) => {
  resizeObserver?.disconnect();
  resizeObserver = null;
  if (!el) return;
  updateFades();
  if (typeof ResizeObserver === 'undefined') return;
  resizeObserver = new ResizeObserver(updateFades);
  resizeObserver.observe(el);
});

onBeforeUnmount(() => resizeObserver?.disconnect());

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

          <div
            ref="scroller"
            class="scroller"
            :class="{ 'scroller--more-above': moreAbove, 'scroller--more-below': moreBelow }"
            @scroll.passive="updateFades"
          >
            <div class="groups">
              <section v-for="group in groups" :key="group.title">
                <h4>{{ group.title }}</h4>
                <ul>
                  <li v-for="(row, index) in group.items" :key="index">
                    <span class="keys">
                      <template v-for="(alternative, altIndex) in row.keys" :key="altIndex">
                        <span v-if="altIndex > 0" class="keys__or">{{ $t('shortcuts.keys.or') }}</span>
                        <span class="keys__alternative">
                          <template v-for="(chord, chordIndex) in alternative" :key="chordIndex">
                            <span v-if="chordIndex > 0" class="keys__pair" aria-hidden="true" />
                            <KeyCap v-for="(cap, capIndex) in chord" :key="capIndex">{{ cap }}</KeyCap>
                          </template>
                        </span>
                      </template>
                    </span>
                    <span class="description">
                      {{ row.description }}
                      <span v-if="row.scope" class="description__scope">{{ row.scope }}</span>
                    </span>
                  </li>
                </ul>
              </section>
            </div>
          </div>

          <footer>
            <span><KeyCap v-for="(cap, index) in commandPaletteKeys" :key="index">{{ cap }}</KeyCap> {{ $t('shortcuts.footerHint') }}</span>
            <AppButton @click="editBindings" variant="ghost" size="sm">{{ $t('shortcuts.editBindings') }}</AppButton>
          </footer>
        </section>
      </ModalShell>
    </Transition>
  </Teleport>
</template>

<style scoped>
.helper {
  display: flex;
  flex-direction: column;
  flex: 1 1 auto;
  min-height: 0;
  padding: var(--space-xl);
}

/* Runs edge to edge, so the scrollbar sits on the panel's rim, not in the text. */
.scroller {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  margin-inline: calc(-1 * var(--space-xl));
  padding-inline: var(--space-xl);
  --fade-top: 0px;
  --fade-bottom: 0px;
  mask-image: linear-gradient(
    to bottom,
    transparent,
    black var(--fade-top),
    black calc(100% - var(--fade-bottom)),
    transparent
  );
}

.scroller--more-above {
  --fade-top: var(--space-xl);
}

.scroller--more-below {
  --fade-bottom: var(--space-2xl);
}

.groups {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 18px var(--space-xl);
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

/*
 * The key column is as wide as the widest keys in the group and no wider —
 * a fixed column left a gutter after `d` and wrapped the descriptions. The
 * cap keeps one long row of alternatives from starving the text; past it
 * the caps wrap instead.
 */
ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: fit-content(60%) minmax(0, 1fr);
  column-gap: var(--space-md);
}

li {
  display: grid;
  grid-column: 1 / -1;
  grid-template-columns: subgrid;
  align-items: center;
  padding: var(--space-1-5) 0;
  border-bottom: 1px solid var(--border-color-muted);
}

li:last-child {
  border-bottom: 0;
}

.keys {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs);
}

.keys__alternative {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2xs);
}

.keys__pair::before {
  content: '/';
  padding: 0 var(--space-2xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
}

.keys__or {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
}

.description {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.description__scope {
  margin-left: var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
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
  color: var(--text-accent);
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
  gap: var(--space-xs);
}

@media (max-width: 720px) {
  .groups {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
