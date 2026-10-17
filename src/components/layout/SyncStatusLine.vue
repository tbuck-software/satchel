<script setup lang="ts">
/**
 * The one sentence that answers "am I online, and when did mail last arrive?".
 *
 * It stays silent while `ConnectionBanner` is up. A standing backend failure is
 * the same fact in a stronger, actionable form, and a local-first client that
 * says it twice in two tones reads as two problems (STYLEGUIDE.md § Error
 * States). The precedence lives in `connection.syncStatus`, so every mount
 * point of this line tells the same story.
 *
 * `status`, never `alert`: being offline is a state the user usually caused,
 * not a failure that interrupts them.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConnectionStore } from '@/stores/connection';
import { formatListTime } from '@/utils/listTime';

const props = withDefaults(defineProps<{
  /** Drop the resting "last synced" sentence, for places that are not a footer. */
  hideIdle?: boolean;
}>(), { hideIdle: false });

const connection = useConnectionStore();
const { t } = useI18n();

const visible = computed(() => {
  if (connection.syncStatus === 'error') return false;
  return !(props.hideIdle && connection.syncStatus === 'idle');
});

const label = computed(() => {
  if (connection.syncStatus === 'offline') return t('app.sync.status.offline');
  if (connection.syncStatus === 'syncing') return t('app.sync.status.syncing');
  if (!connection.lastSyncAt) return t('app.sync.status.never');
  return t('app.sync.status.lastSynced', { time: formatListTime(connection.lastSyncAt) });
});
</script>

<template>
  <p v-if="visible" class="sync-status" :class="`sync-status--${connection.syncStatus}`" role="status">
    <span class="sync-status__dot" aria-hidden="true" />
    <span class="sync-status__text">{{ label }}</span>
  </p>
</template>

<style scoped>
.sync-status {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  margin: 0;
  min-width: 0;
  padding: var(--space-xs) var(--space-sm);
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
}

.sync-status__dot {
  flex-shrink: 0;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-tertiary);
}

.sync-status__text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sync-status--offline .sync-status__dot {
  background: var(--accent-amber);
}

.sync-status--offline .sync-status__text {
  color: var(--accent-amber-soft);
  font-weight: var(--weight-medium);
}

.sync-status--syncing .sync-status__dot {
  background: var(--accent-primary);
  animation: sync-status-pulse 1.4s var(--ease-in-out) infinite;
}

@keyframes sync-status-pulse {
  50% {
    opacity: 0.3;
  }
}

@media (prefers-reduced-motion: reduce) {
  .sync-status--syncing .sync-status__dot {
    animation: none;
  }
}
</style>
