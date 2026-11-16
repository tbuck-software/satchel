<script setup lang="ts">
/**
 * Stands in for To/Cc/Bcc while the composer is in mail merge mode: who the
 * copies go to, a picker that inserts `{{Field}}` where the caret last was
 * (subject or body), and the way out.
 */
import { ref, useId } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import type { MailMergeSource } from '@/utils/mailMerge';

defineProps<{
  source: MailMergeSource;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  insert: [field: string];
  change: [];
  stop: [];
}>();

const pickerId = useId();
const picked = ref('');

function onPick() {
  const field = picked.value;
  picked.value = '';
  if (field) emit('insert', field);
}
</script>

<template>
  <div class="merge-bar" data-testid="mail-merge-bar">
    <AppIcon class="merge-bar__icon" aria-hidden="true"><path d="M2 4.5h8M2 8h8M2 11.5h5" /><path d="m11 9.5 3 2-3 2" /></AppIcon>
    <p class="merge-bar__label">
      <span class="merge-bar__title">{{ $t('mailMerge.bar.title') }}</span>
      <span class="merge-bar__count">{{ $t('mailMerge.bar.recipients', { count: source.rows.length, source: source.label }, source.rows.length) }}</span>
    </p>
    <div class="merge-bar__actions">
      <label class="sr-only" :for="pickerId">{{ $t('mailMerge.bar.insertField') }}</label>
      <select
        :id="pickerId"
        v-model="picked"
        class="merge-bar__picker"
        :disabled="disabled"
        :title="$t('mailMerge.bar.insertFieldHint')"
        data-testid="mail-merge-field-picker"
        @change="onPick"
      >
        <option value="" disabled>{{ $t('mailMerge.bar.insertField') }}</option>
        <option v-for="field in source.fields" :key="field" :value="field">{{ field }}</option>
      </select>
      <AppButton size="sm" variant="ghost" :disabled="disabled" @click="emit('change')">{{ $t('mailMerge.bar.change') }}</AppButton>
      <AppButton size="sm" variant="ghost" :disabled="disabled" @click="emit('stop')">{{ $t('mailMerge.bar.stop') }}</AppButton>
    </div>
  </div>
</template>

<style scoped>
.merge-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs) var(--space-sm);
  padding: var(--space-sm) 0;
  border-bottom: 1px solid var(--border-subtle);
  min-width: 0;
}

.merge-bar__icon {
  flex: none;
  color: var(--text-accent);
}

.merge-bar__label {
  display: flex;
  flex: 1 1 12rem;
  flex-wrap: wrap;
  gap: 0 var(--space-xs);
  margin: 0;
  min-width: 0;
  font-size: var(--font-size-sm);
}

.merge-bar__title {
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

.merge-bar__count {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--text-secondary);
}

.merge-bar__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs);
}

.merge-bar__picker {
  min-height: var(--touch-target);
  max-width: 14rem;
  padding: 0 var(--space-sm);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}
</style>
