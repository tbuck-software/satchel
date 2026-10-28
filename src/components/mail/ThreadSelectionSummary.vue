<script setup lang="ts">
/*
 * Several conversations selected: the reader shows what the toolbar above will
 * act on, instead of one of them in full under a "Done (3)" that means all
 * three. The actions stay in the toolbar; this only says what they reach.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import Avatar from '@/components/common/Avatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import { useShortcutHints } from '@/composables/useShortcutHints';
import { useThreadsStore } from '@/stores/threads';

const SHOWN = 6;

const threads = useThreadsStore();
const { t } = useI18n();
const { hint } = useShortcutHints();

const selected = computed(() => threads.selectedThreadIds.flatMap((id) => threads.find(id) ?? []));
const shown = computed(() => selected.value.slice(0, SHOWN));
const more = computed(() => Math.max(0, selected.value.length - SHOWN));
const clearKey = computed(() => hint('clearSelection'));

function sender(thread: (typeof selected.value)[number]) {
  const person = thread.participants[0];
  return person?.name || person?.email || '';
}
</script>

<template>
  <section class="selection-summary" :aria-label="t('thread.selection.label', { count: selected.length })">
    <h1 class="selection-summary__title">{{ t('thread.selection.summaryTitle', { count: selected.length }, selected.length) }}</h1>
    <p class="selection-summary__hint">{{ t('thread.selection.summaryHint') }}</p>
    <ul class="selection-summary__list">
      <li v-for="thread in shown" :key="thread.id" class="selection-summary__item">
        <Avatar :name="sender(thread)" :email="thread.participants[0]?.email ?? ''" :size="28" />
        <span class="selection-summary__text">
          <span class="selection-summary__sender">{{ sender(thread) }}</span>
          <span class="selection-summary__subject">{{ thread.normalizedSubject ?? thread.subject }}</span>
        </span>
      </li>
    </ul>
    <p v-if="more" class="selection-summary__more">{{ t('thread.selection.summaryMore', { count: more }) }}</p>
    <AppButton class="selection-summary__clear" variant="ghost" size="sm" @click="threads.clearSelection()">
      {{ t('thread.selection.clear') }}
      <KeyCap v-if="clearKey" aria-hidden="true">{{ clearKey.label }}</KeyCap>
    </AppButton>
  </section>
</template>

<style scoped>
.selection-summary {
  display: grid;
  gap: var(--space-md);
  padding-top: var(--space-xl);
}

.selection-summary__title {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-semibold);
  line-height: 1.2;
}

.selection-summary__hint {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.selection-summary__list {
  display: grid;
  margin: 0;
  padding: 0;
  border: var(--border-muted);
  border-radius: var(--radius-md);
  list-style: none;
  overflow: hidden;
}

.selection-summary__item {
  display: flex;
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-sm) var(--space-md);
  background: var(--bg-surface);
}

.selection-summary__item + .selection-summary__item {
  border-top: var(--border-muted);
}

.selection-summary__text {
  display: grid;
  min-width: 0;
}

.selection-summary__sender {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}

.selection-summary__subject,
.selection-summary__more {
  overflow: hidden;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.selection-summary__more {
  margin: 0;
}

.selection-summary__clear {
  justify-self: start;
}
</style>
