<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, nextTick, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useContactsStore } from '@/stores/contacts';
import {
  hasOpenRecipientToken,
  isValidRecipient,
  recipientChipLabel,
  splitRecipientList,
} from '@/utils/recipients';
import type { BackendContact } from '@/types/contacts';

const props = defineProps<{
  modelValue: string[];
  placeholder?: string;
}>();

const emit = defineEmits<{
  (e: 'update:modelValue', value: string[]): void;
  /** Cmd/Ctrl+Enter, after the address being typed has become a chip. */
  (e: 'submit'): void;
}>();

const { t } = useI18n();
const draft = ref('');
const contacts = useContactsStore();
const inputRef = ref<HTMLInputElement | null>(null);
const suggestions = ref<BackendContact[]>([]);
const suggestionOpen = ref(false);
const activeSuggestionIndex = ref(-1);
const listboxId = `recipient-suggestions-${useId()}`;
/**
 * The chip a first Backspace in the empty field selected. The second one
 * removes it; one stray keypress used to take a recipient away unseen.
 */
const pendingRemoval = ref<number | null>(null);
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let searchRevision = 0;

const chips = computed(() => props.modelValue);

watch(chips, () => {
  pendingRemoval.value = null;
});

/** Appends tokens that are not chips yet, preserving their order. */
function addTokens(tokens: string[]) {
  const seen = new Set(chips.value);
  const merged = [...chips.value];
  for (const token of tokens) {
    if (seen.has(token)) continue;
    merged.push(token);
    seen.add(token);
  }
  if (merged.length !== chips.value.length) emit('update:modelValue', merged);
}

function commitDraft() {
  addTokens(splitRecipientList(draft.value));
  draft.value = '';
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Backspace') pendingRemoval.value = null;
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
    event.preventDefault();
    if (draft.value.trim()) commitDraft();
    suggestionOpen.value = false;
    activeSuggestionIndex.value = -1;
    emit('submit');
    return;
  }
  if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && suggestions.value.length > 0) {
    event.preventDefault();
    suggestionOpen.value = true;
    moveActiveSuggestion(event.key === 'ArrowDown' ? 1 : -1);
    return;
  }
  if (suggestionOpen.value && event.key === 'Escape') {
    event.preventDefault();
    suggestionOpen.value = false;
    activeSuggestionIndex.value = -1;
    return;
  }
  if (
    suggestionOpen.value &&
    (event.key === 'Enter' || event.key === 'Tab') &&
    activeSuggestionIndex.value >= 0
  ) {
    event.preventDefault();
    const active = suggestions.value[activeSuggestionIndex.value];
    if (active) pickSuggestion(active);
    return;
  }
  if (event.key === ',' || event.key === ';' || event.key === 'Enter' || event.key === 'Tab') {
    // A separator inside a quoted name or an unclosed `<` belongs to the token.
    if ((event.key === ',' || event.key === ';') && hasOpenRecipientToken(draft.value)) return;
    if (draft.value.trim()) {
      event.preventDefault();
      commitDraft();
    }
  } else if (event.key === 'Backspace' && draft.value === '' && chips.value.length > 0) {
    const last = chips.value.length - 1;
    if (pendingRemoval.value === last) remove(last);
    else pendingRemoval.value = last;
  }
}

function onBlur() {
  commitDraft();
  pendingRemoval.value = null;
}

const pendingRemovalAnnouncement = computed(() => {
  const index = pendingRemoval.value;
  const value = index === null ? undefined : chips.value[index];
  return value === undefined ? '' : t('compose.recipients.backspaceAgain', { address: value });
});

function suggestionId(index: number) {
  return `${listboxId}-${index}`;
}

function moveActiveSuggestion(direction: 1 | -1) {
  const count = suggestions.value.length;
  if (count === 0) return;
  const current = activeSuggestionIndex.value;
  activeSuggestionIndex.value = current < 0
    ? (direction === 1 ? 0 : count - 1)
    : (current + direction + count) % count;
  void nextTick(() => {
    document.getElementById(suggestionId(activeSuggestionIndex.value))?.scrollIntoView({ block: 'nearest' });
  });
}

function remove(index: number) {
  const next = [...chips.value];
  next.splice(index, 1);
  emit('update:modelValue', next);
}

function pickSuggestion(contact: BackendContact) {
  const value = contact.display_name
    ? `${contact.display_name} <${contact.email}>`
    : contact.email;
  if (!chips.value.includes(value)) {
    emit('update:modelValue', [...chips.value, value]);
  }
  draft.value = '';
  suggestions.value = [];
  suggestionOpen.value = false;
  activeSuggestionIndex.value = -1;
}

function onPaste(event: ClipboardEvent) {
  const text = event.clipboardData?.getData('text');
  if (!text || !/[,;\n]/.test(text)) return;
  event.preventDefault();
  addTokens(splitRecipientList(text));
}

watch(draft, (value) => {
  const revision = ++searchRevision;
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  const term = value.trim();
  if (term.length < 2) {
    suggestions.value = [];
    suggestionOpen.value = false;
    activeSuggestionIndex.value = -1;
    return;
  }
  searchTimer = setTimeout(async () => {
    try {
      const result = await contacts.search(term, 8);
      if (revision !== searchRevision) return;
      suggestions.value = result;
      suggestionOpen.value = suggestions.value.length > 0;
      activeSuggestionIndex.value = suggestions.value.length > 0 ? 0 : -1;
    } catch {
      if (revision !== searchRevision) return;
      suggestions.value = [];
      suggestionOpen.value = false;
      activeSuggestionIndex.value = -1;
    }
  }, 120);
});

// So a composer that opens with no recipients can start in the field the user
// has to fill first. The input is inside the chip row and unreachable from
// outside without this.
defineExpose({ focus: () => inputRef.value?.focus() });
</script>

<template>
  <div class="chips-wrap">
  <div class="chips" @click="inputRef?.focus()">
    <span
      v-for="(value, index) in chips"
      :key="value + index"
      class="chip"
      :class="{ 'chip--invalid': !isValidRecipient(value), 'chip--pending-removal': pendingRemoval === index }"
      :title="isValidRecipient(value) ? value : t('compose.recipients.invalid')"
    >
      {{ recipientChipLabel(value) }}
      <span v-if="!isValidRecipient(value)" class="sr-only">{{ t('compose.recipients.invalidMarker') }}</span>
      <IconButton :title="t('compose.recipients.remove', { address: value })" class="chip-remove" @click.stop="remove(index)">×</IconButton>
    </span>
    <input
      ref="inputRef"
      v-model="draft"
      :placeholder="chips.length === 0 ? props.placeholder ?? '' : ''"
      type="text"
      autocomplete="off"
      spellcheck="false"
      role="combobox"
      aria-autocomplete="list"
      :aria-expanded="suggestionOpen"
      :aria-controls="suggestionOpen ? listboxId : undefined"
      :aria-activedescendant="suggestionOpen && activeSuggestionIndex >= 0 ? suggestionId(activeSuggestionIndex) : undefined"
      @keydown="onKey"
      @blur="onBlur"
      @paste="onPaste"
    />
  </div>
  <span class="sr-only" role="status" aria-live="polite">{{ pendingRemovalAnnouncement }}</span>
  <div v-if="suggestionOpen" :id="listboxId" class="suggestions" role="listbox">
    <CardButton
      v-for="(contact, index) in suggestions"
      :key="contact.id"
      :id="suggestionId(index)"
      role="option"
      :aria-selected="index === activeSuggestionIndex"
      :class="{ active: index === activeSuggestionIndex }"
      @mouseenter="activeSuggestionIndex = index"
      @mousedown.prevent="pickSuggestion(contact)"
    >
      <span>{{ contact.display_name || contact.email }}</span>
      <small>{{ contact.email }}</small>
    </CardButton>
  </div>
  </div>
</template>

<style scoped>
.chips-wrap {
  position: relative;
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  align-items: center;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  padding: 6px 8px;
  min-height: 36px;
  cursor: text;
}

.suggestions {
  position: absolute;
  z-index: 30;
  left: 0;
  right: 0;
  top: calc(100% + 4px);
  display: grid;
  max-height: min(360px, 45vh);
  overflow-y: auto;
  gap: 2px;
  padding: 6px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  box-shadow: var(--shadow-lg);
}

.suggestions button {
  display: grid;
  gap: 2px;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-primary);
  text-align: left;
  padding: 7px 8px;
  cursor: pointer;
}

.suggestions button:hover,
.suggestions button.active {
  background: var(--bg-hover);
}

.suggestions button.active {
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-primary) 48%, transparent) inset;
}

.suggestions small {
  color: var(--text-tertiary);
}

.chips:focus-within {
  border-color: var(--accent-primary);
  box-shadow: var(--focus-ring);
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: color-mix(in srgb, var(--accent-primary) 12%, var(--bg-raised));
  color: var(--text-primary);
  padding: 2px 8px;
  border-radius: var(--radius-full);
  font-size: var(--font-size-sm);
}

.chip--invalid {
  background: color-mix(in srgb, var(--accent-red) 16%, var(--bg-raised));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-red) 55%, transparent) inset;
}

.chip--pending-removal {
  box-shadow: 0 0 0 2px var(--accent-primary) inset;
}

.chip-remove {
  background: transparent;
  border: 0;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-md);
  line-height: 1;
  padding: 0 2px;
}

.chip-remove:hover {
  color: var(--text-primary);
}

input {
  flex: 1 1 100px;
  min-width: 100px;
  border: 0 !important;
  background: transparent !important;
  outline: none;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  padding: 2px 0;
  font-family: inherit;
  box-shadow: none !important;
}
/* A hint, not a value: in the secondary ink "Name or email" read like a
   recipient already entered. */
input::placeholder {
  color: var(--text-tertiary);
  font-weight: var(--weight-regular);
  opacity: 1;
}
</style>
