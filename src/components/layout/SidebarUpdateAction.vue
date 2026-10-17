<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAppUpdateStore } from '@/stores/appUpdate';

const update = useAppUpdateStore();
const { t } = useI18n();

const label = computed(() => {
  if (update.status === 'downloading') return t('app.update.downloading', { progress: update.progress });
  if (update.status === 'ready') return t('app.update.restart');
  if (update.status === 'failed') return t('app.update.failedRetry');
  return t('app.update.available');
});

const title = computed(() => {
  return update.version ? t('app.update.version', { version: update.version }) : label.value;
});
</script>

<template>
  <button
    v-if="update.visible"
    type="button"
    class="update-action"
    :class="`update-action--${update.status}`"
    :style="update.status === 'downloading' ? { '--update-progress': `${update.progress}%` } : undefined"
    :disabled="update.status === 'downloading'"
    :title="title"
    :aria-label="label"
    :aria-busy="update.status === 'downloading'"
    @click="update.activate()"
  >
    <span class="update-action__content">
      <svg
        v-if="update.status === 'ready'"
        width="18"
        height="18"
        viewBox="0 0 18 18"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M14.4 6.3A6 6 0 1 0 15 9" />
        <path d="M14.4 2.8V6.3H10.9" />
      </svg>
      <svg
        v-else
        width="18"
        height="18"
        viewBox="0 0 18 18"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M9 2.5V11" />
        <path d="M5.7 7.8L9 11.1L12.3 7.8" />
        <path d="M3 11.5V14.5H15V11.5" />
      </svg>
      <span aria-live="polite">{{ label }}</span>
    </span>
  </button>
</template>

<style scoped>
.update-action {
  --update-progress: 0%;
  position: relative;
  isolation: isolate;
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 38px;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--accent-primary) 28%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
  color: var(--accent-primary);
  padding: 8px 10px;
  font: inherit;
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out),
    border-color var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.update-action::before {
  position: absolute;
  z-index: -1;
  inset: 0 auto 0 0;
  width: var(--update-progress);
  background: color-mix(in srgb, var(--accent-primary) 22%, transparent);
  content: '';
  transition: width var(--duration-normal) var(--ease-out);
}

.update-action:not(:disabled):hover {
  border-color: color-mix(in srgb, var(--accent-primary) 45%, transparent);
  background: color-mix(in srgb, var(--accent-primary) 20%, transparent);
}

.update-action:not(:disabled):active {
  transform: scale(0.99);
}

.update-action:disabled {
  cursor: progress;
}

.update-action--failed {
  border-color: color-mix(in srgb, var(--accent-red) 32%, transparent);
  background: color-mix(in srgb, var(--accent-red) 10%, transparent);
  color: var(--accent-red);
}

.update-action__content {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.update-action__content svg {
  flex-shrink: 0;
}
</style>
