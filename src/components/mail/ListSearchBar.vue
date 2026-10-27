<script setup lang="ts">
/*
 * The search field above a thread list. Typing narrows the list in place;
 * the results themselves load lazily in `ListSearchResults`. Esc empties the
 * field and gives the list back as it was.
 */
import { onBeforeUnmount, ref } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import SearchInput from '@/components/ui/SearchInput.vue';
import { registerListSearchField } from '@/composables/listSearchFocus';
import { moveListKeyboardFocus } from '@/composables/useListKeyboardNavigation';
import { useListSearchStore } from '@/stores/listSearch';
import { useUiStore } from '@/stores/ui';

const props = defineProps<{
  /** The list's name, for the placeholder: "Search Inbox…". */
  mailboxLabel: string;
}>();

const store = useListSearchStore();
const ui = useUiStore();
const input = ref<InstanceType<typeof SearchInput> | null>(null);
const focused = ref(false);

function focus() {
  input.value?.focus();
  input.value?.select();
}

onBeforeUnmount(registerListSearchField(focus));

function onEscape(event: KeyboardEvent) {
  // Handled here so the global Escape does not also leave the page.
  event.preventDefault();
  if (store.active) {
    store.clear();
    return;
  }
  (event.target as HTMLElement).blur();
}

/** Down or Enter hands the keyboard to the results, like a combobox. */
function enterResults(event: KeyboardEvent) {
  if (!store.active) return;
  event.preventDefault();
  if (moveListKeyboardFocus(1)) (event.target as HTMLElement).blur();
}

function clear() {
  store.clear();
  input.value?.focus();
}
</script>

<template>
  <div class="list-search-bar">
    <SearchInput
      ref="input"
      :model-value="store.query"
      :label="$t('inbox.listSearch.label')"
      :placeholder="$t('inbox.listSearch.placeholder', { mailbox: props.mailboxLabel })"
      aria-keyshortcuts="/"
      @update:model-value="store.setQuery"
      @focus="focused = true"
      @blur="focused = false"
      @keydown.esc="onEscape"
      @keydown.down="enterResults"
      @keydown.enter="enterResults"
    >
      <template #trailing>
        <IconButton v-if="store.active" size="sm" :title="$t('inbox.listSearch.clear')" @click="clear">
          <AppIcon><path d="m5 5 6 6m0-6-6 6" /></AppIcon>
        </IconButton>
        <KeyCap v-else-if="!focused && !ui.isMobile" class="list-search-bar__hint" aria-hidden="true">/</KeyCap>
      </template>
    </SearchInput>
    <slot />
  </div>
</template>

<style scoped>
.list-search-bar {
  display: flex;
  align-items: center;
  gap: var(--space-1-5);
  min-width: 0;
  padding: 0 var(--space-lg) var(--space-2-5);
  background: var(--bg-list);
}

.list-search-bar__hint {
  margin-inline-end: var(--space-xs);
}
</style>
