<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';

export interface ComposerIdentityOption {
  key: string;
  label: string;
  accountId: string;
  aliasEmail: string | null;
}

export interface ComposerIdentityGroup {
  account: { id: string; displayName: string };
  options: ComposerIdentityOption[];
}

/**
 * The composer's "From" picker: one flat select over every identity. The
 * desktop shows it above To; the phone folds it under Cc/Bcc, so it renders
 * in either place from here.
 */
defineProps<{
  id: string;
  choices: ComposerIdentityGroup[];
  value: string;
  disabled: boolean;
}>();

const emit = defineEmits<{ change: [key: string] }>();
</script>

<template>
  <div class="sender-field">
    <select
      :id="id"
      class="sender-select"
      :disabled="disabled"
      :title="$t('compose.sender')"
      :aria-label="$t('compose.sender')"
      :value="value"
      @change="emit('change', ($event.target as HTMLSelectElement).value)"
    >
      <template v-if="choices.length > 1">
        <optgroup
          v-for="group in choices"
          :key="group.account.id"
          :label="group.account.displayName"
        >
          <option v-for="option in group.options" :key="option.key" :value="option.key">{{ option.label }}</option>
        </optgroup>
      </template>
      <template v-else>
        <option
          v-for="option in choices[0]?.options ?? []"
          :key="option.key"
          :value="option.key"
        >{{ option.label }}</option>
      </template>
    </select>
    <AppIcon class="sender-chevron" size="sm"><path d="m4 6 4 4 4-4" /></AppIcon>
  </div>
</template>

<style scoped>
.sender-field {
  position: relative;
  min-width: 0;
  justify-self: start;
  max-width: 100%;
}

.sender-select {
  appearance: none;
  width: 100%;
  min-width: 0;
  min-height: 36px;
  padding: var(--space-sm) var(--space-2xl) var(--space-sm) 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-family: inherit;
  text-overflow: ellipsis;
  cursor: pointer;
}

.sender-select option,
.sender-select optgroup {
  background: var(--bg-overlay);
  color: var(--text-primary);
}

.sender-chevron {
  position: absolute;
  right: var(--space-sm);
  top: 50%;
  transform: translateY(-50%);
  color: var(--text-secondary);
  pointer-events: none;
}
</style>
