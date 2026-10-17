<script setup lang="ts">
/**
 * A list or pane that cannot show its content — the third home for a backend
 * failure next to the connection banner and a toast (STYLEGUIDE.md § Error
 * States).
 *
 * It replaces the empty state, it does not join it: "no mail" and "we could not
 * load your mail" are different sentences, and someone who reads the first one
 * after a failed load assumes the mail is gone.
 */
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';

withDefaults(
  defineProps<{
    title: string;
    /** The translated cause. Raw backend text belongs in a `<details>`. */
    detail?: string;
    busy?: boolean;
  }>(),
  { detail: undefined, busy: false },
);

const emit = defineEmits<{ retry: [] }>();
const { t } = useI18n();
</script>

<template>
  <div class="error-state" role="alert">
    <span class="error-state__icon" aria-hidden="true">
      <AppIcon :grid="24">
        <circle cx="12" cy="12" r="8.6" />
        <path d="M12 7.6v5" />
        <path d="M12 15.8h.01" />
      </AppIcon>
    </span>
    <p class="state state--error">{{ title }}</p>
    <p v-if="detail" class="state state--detail">{{ detail }}</p>
    <AppButton size="sm" variant="soft" :disabled="busy" @click="emit('retry')">
      {{ t('common.retry') }}
    </AppButton>
  </div>
</template>

<style scoped>
.error-state {
  display: flex;
  height: 100%;
  min-height: 200px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-sm);
  padding: var(--space-xl) var(--space-lg);
  text-align: center;
}

.error-state__icon {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  color: var(--accent-red-soft);
}

.error-state__icon svg {
  width: 26px;
  height: 26px;
}

.state {
  margin: 0;
  max-width: 320px;
  font-size: var(--font-size-sm);
}

.state--error {
  color: var(--accent-red-soft);
  font-weight: var(--weight-semibold);
}

.state--detail {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}
</style>
