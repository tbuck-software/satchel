<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useSettingsStore } from '@/stores/settings';
import {
  DEFAULT_KEYBINDINGS,
  KEYBINDING_CONTEXT_LABELS,
  KEYBINDING_DEFINITIONS,
  bindingStrokes,
  formatBinding,
  formatBindingStroke,
  keybindingConflicts,
  keyboardEventStroke,
} from '@/config/keybindings';
import AppButton from '@/components/ui/AppButton.vue';

const { t } = useI18n();
const settings = useSettingsStore();
const filter = ref('');
const recordingAction = ref('');
const pendingStrokes = ref<string[]>([]);

const normalizedFilter = computed(() => filter.value.trim().toLocaleLowerCase());
const visibleBindings = computed(() => KEYBINDING_DEFINITIONS.filter((definition) => {
  if (!normalizedFilter.value) return true;
  return `${definition.label} ${definition.keywords ?? ''} ${settings.keybindings[definition.action] ?? ''}`
    .toLocaleLowerCase()
    .includes(normalizedFilter.value);
}));

const conflicts = computed(() => keybindingConflicts(settings.keybindings));

function conflictTitle(action: string): string | undefined {
  const fixed = conflicts.value.get(action)?.fixed ?? [];
  if (fixed.length === 0) return undefined;
  return t('settings.shortcuts.reservedConflict', { keys: fixed.map((binding) => formatBinding(binding)).join(', ') });
}

const modifiedCount = computed(() => KEYBINDING_DEFINITIONS.filter(
  (definition) => settings.keybindings[definition.action] !== definition.defaultBinding,
).length);

function startRecording(action: string) {
  recordingAction.value = action;
  pendingStrokes.value = [];
}

function stopRecording() {
  recordingAction.value = '';
  pendingStrokes.value = [];
}

function record(event: KeyboardEvent, action: string) {
  event.preventDefault();
  event.stopPropagation();
  if (event.key === 'Escape') {
    stopRecording();
    return;
  }
  if (['Meta', 'Control', 'Alt', 'Shift'].includes(event.key)) return;

  const stroke = keyboardEventStroke(event);
  const definition = KEYBINDING_DEFINITIONS.find((entry) => entry.action === action);
  const isSequence = bindingStrokes(definition?.defaultBinding ?? '').length > 1;
  if (isSequence && pendingStrokes.value.length === 0) {
    pendingStrokes.value = [stroke];
    return;
  }

  settings.setKeybinding(action, [...pendingStrokes.value, stroke].join(' '));
  stopRecording();
}

function displayStrokes(action: string): string[] {
  if (recordingAction.value === action && pendingStrokes.value.length > 0) {
    return [...pendingStrokes.value, '…'];
  }
  return bindingStrokes(settings.keybindings[action] ?? '');
}
</script>

<template>
  <section class="keybindings">
    <header class="keybindings__header">
      <div>
        <span class="eyebrow">{{ $t('settings.shortcuts.eyebrow') }}</span>
        <h2>{{ $t('settings.shortcuts.title') }}</h2>
        <p>{{ $t('settings.shortcuts.description') }}</p>
      </div>
      <div class="header-actions">
        <span class="binding-count">{{ t('settings.shortcuts.commandCount', KEYBINDING_DEFINITIONS.length) }}</span>
        <AppButton variant="secondary" size="sm" :disabled="modifiedCount === 0" @click="settings.resetKeybindings()">
          {{ $t('common.reset') }}<span v-if="modifiedCount"> · {{ modifiedCount }}</span>
        </AppButton>
      </div>
    </header>

    <label class="search-box">
      <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.7"/><path d="m10.7 10.7 3 3"/></svg>
      <input v-model="filter" type="search" :placeholder="$t('settings.shortcuts.searchPlaceholder')" />
      <kbd>/</kbd>
    </label>

    <div class="binding-table" role="table" :aria-label="$t('settings.shortcuts.title')">
      <div class="table-head" role="row">
        <span role="columnheader">{{ $t('settings.shortcuts.colCommand') }}</span>
        <span role="columnheader">{{ $t('settings.shortcuts.colKeys') }}</span>
        <span role="columnheader">{{ $t('settings.shortcuts.colContext') }}</span>
        <span role="columnheader">{{ $t('common.status') }}</span>
      </div>

      <button
        v-for="definition in visibleBindings"
        :key="definition.action"
        type="button"
        class="binding-row"
        :class="{
          'binding-row--recording': recordingAction === definition.action,
          'binding-row--conflict': conflicts.has(definition.action),
        }"
        role="row"
        :aria-label="`${definition.label}: ${formatBinding(settings.keybindings[definition.action] ?? '')}`"
        @click="startRecording(definition.action)"
        @keydown="record($event, definition.action)"
        @blur="stopRecording"
      >
        <span class="command-cell" role="cell">
          <strong>{{ definition.label }}</strong>
          <small>{{ definition.action }}</small>
        </span>
        <span class="key-cell" role="cell">
          <template v-for="(stroke, strokeIndex) in displayStrokes(definition.action)" :key="`${stroke}-${strokeIndex}`">
            <span v-if="strokeIndex" class="then">{{ $t('settings.shortcuts.then') }}</span>
            <span class="key-chord">
              <kbd v-for="part in formatBindingStroke(stroke)" :key="part">{{ part }}</kbd>
            </span>
          </template>
          <span v-if="recordingAction === definition.action" class="recording-dot" aria-hidden="true" />
        </span>
        <span class="context-cell" role="cell">{{ KEYBINDING_CONTEXT_LABELS[definition.context] }}</span>
        <span class="status-cell" role="cell">
          <span
            v-if="conflicts.has(definition.action)"
            class="status status--warning"
            :title="conflictTitle(definition.action)"
          >{{ $t('settings.shortcuts.conflict') }}</span>
          <span
            v-else-if="settings.keybindings[definition.action] !== DEFAULT_KEYBINDINGS[definition.action]"
            class="status status--custom"
          >{{ $t('settings.shortcuts.custom') }}</span>
          <span v-else class="status">{{ $t('common.default') }}</span>
        </span>
      </button>

      <p v-if="visibleBindings.length === 0" class="empty-state">{{ $t('settings.shortcuts.noMatch') }}</p>
    </div>

    <footer class="keybindings__footer">
      <span><i class="recording-dot" /> {{ $t('settings.shortcuts.footerRecord') }}</span>
      <span><kbd>Esc</kbd> {{ $t('common.cancel') }}</span>
    </footer>
  </section>
</template>

<style scoped>
.keybindings {
  display: grid;
  gap: 20px;
  width: min(100%, 900px);
}

.keybindings__header {
  display: flex;
  align-items: end;
  justify-content: space-between;
  gap: 28px;
}

.eyebrow {
  color: var(--accent-primary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: .14em;
  text-transform: uppercase;
}

h2 {
  margin: 5px 0 0;
  color: var(--text-primary);
  font-size: var(--font-size-2xl);
  letter-spacing: -.025em;
}

.keybindings__header p {
  margin: 5px 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

.binding-count {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.search-box {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  min-height: 42px;
  padding: 0 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-raised) 72%, transparent);
  box-shadow: inset 0 1px 0 color-mix(in srgb, white 4%, transparent);
}

.search-box:focus-within {
  border-color: color-mix(in srgb, var(--accent-primary) 65%, var(--border-color));
  box-shadow: var(--focus-ring);
}

.search-box svg { width: 16px; fill: none; stroke: var(--text-tertiary); stroke-width: 1.5; }
.search-box input { min-width: 0; border: 0; outline: 0; background: transparent; color: var(--text-primary); font: inherit; }
.search-box input::placeholder { color: var(--text-tertiary); }

kbd {
  display: inline-grid;
  min-width: 24px;
  height: 24px;
  place-items: center;
  padding: 0 6px;
  border: 1px solid color-mix(in srgb, var(--border-color) 80%, transparent);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--bg-overlay) 88%, var(--bg-base));
  color: var(--text-secondary);
  box-shadow: inset 0 -1px 0 var(--border-color);
  font-family: var(--font-mono);
  font-size: var(--font-size-2xs);
  line-height: 1;
}

.binding-table {
  overflow: hidden;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-surface) 90%, var(--bg-base));
}

.table-head,
.binding-row {
  display: grid;
  grid-template-columns: minmax(220px, 1.55fr) minmax(160px, 1fr) minmax(130px, .85fr) 86px;
  align-items: center;
  gap: 16px;
}

.table-head {
  min-height: 38px;
  padding: 0 16px;
  background: color-mix(in srgb, var(--bg-raised) 66%, transparent);
  color: var(--text-tertiary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: .1em;
  text-transform: uppercase;
}

.binding-row {
  width: 100%;
  min-height: 58px;
  padding: 8px 16px;
  border: 0;
  border-top: 1px solid var(--border-color-muted);
  background: transparent;
  color: var(--text-secondary);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out), box-shadow var(--duration-fast) var(--ease-out);
}

.binding-row:hover { background: var(--bg-hover); }
.binding-row:focus-visible,
.binding-row--recording {
  position: relative;
  z-index: 1;
  background: color-mix(in srgb, var(--accent-primary) 10%, var(--bg-base));
  box-shadow: inset 3px 0 0 var(--accent-primary), inset 0 0 0 1px color-mix(in srgb, var(--accent-primary) 34%, transparent);
}

.binding-row--conflict { box-shadow: inset 3px 0 0 var(--accent-orange); }

.command-cell { display: grid; gap: 3px; min-width: 0; }
.command-cell strong { overflow: hidden; color: var(--text-primary); font-size: var(--font-size-sm); font-weight: var(--weight-semibold); text-overflow: ellipsis; white-space: nowrap; }
.command-cell small { color: var(--text-tertiary); font-family: var(--font-mono); font-size: var(--font-size-3xs); }

.key-cell { display: flex; align-items: center; gap: 6px; min-width: 0; }
.key-chord { display: inline-flex; gap: 3px; }
.then { color: var(--text-tertiary); font-size: var(--font-size-3xs); }
.recording-dot { display: inline-block; width: 6px; height: 6px; border-radius: var(--radius-full); background: var(--accent-primary); box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent-primary) 14%, transparent); }

.context-cell { color: var(--text-tertiary); font-size: var(--font-size-xs); }
.status-cell { justify-self: start; }
.status { color: var(--text-tertiary); font-size: var(--font-size-3xs); font-weight: var(--weight-strong); }
.status--custom { color: var(--accent-primary); }
.status--warning { color: var(--accent-orange); }

.empty-state { margin: 0; padding: 42px 16px; color: var(--text-tertiary); text-align: center; }

.keybindings__footer {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 12px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.keybindings__footer span { display: inline-flex; align-items: center; gap: 8px; }

@media (max-width: 800px) {
  .table-head,
  .binding-row { grid-template-columns: minmax(180px, 1.4fr) minmax(130px, 1fr) 82px; }
  .table-head > :nth-child(3), .context-cell { display: none; }
  .keybindings__header { align-items: start; }
}

@media (max-width: 560px) {
  .keybindings__header { display: grid; }
  .table-head,
  .binding-row { grid-template-columns: minmax(0, 1fr) auto; }
  .table-head > :nth-child(3), .table-head > :nth-child(4), .context-cell, .status-cell { display: none; }
  .key-cell { justify-content: end; }
}
</style>
