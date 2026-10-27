<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useMailSync } from '@/composables/useMailSync';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import { formatListTime } from '@/utils/listTime';

const SyncFailures = defineAsyncComponent(() => import('./SyncFailures.vue'));
const failuresOpen = ref(false);
const { t } = useI18n();
const connection = useConnectionStore();
const { syncing, syncMail } = useMailSync();

const outbox = useOutboxStore();
const failureCount = computed(() => outbox.attentionCount);
const statusLabel = computed(() => {
  // Standing failures already have an actionable explanation in the banner.
  if (connection.syncStatus === 'error') return '';
  if (!connection.online) return t('app.sync.status.offline');
  if (syncing.value) return t('app.sync.status.syncing');
  if (!connection.lastSyncAt) return t('app.sync.status.never');
  return t('app.sync.status.lastSynced', { time: formatListTime(connection.lastSyncAt) });
});
function activate(event: MouseEvent) {
  if (failureCount.value > 0) {
    (event.currentTarget as HTMLElement).focus();
    failuresOpen.value = true;
  } else {
    void syncMail();
  }
}
const title = computed(() => failureCount.value > 0 ? t('app.sync.failures.label', { count: failureCount.value }, failureCount.value) : [t('commands.syncMail'), statusLabel.value].filter(Boolean).join(' · '));
</script>

<template>
  <span class="sync-control">
  <IconButton
    class="sync-button"
    size="sm"
    :title="title"
    :aria-label="failureCount > 0 ? title : t('commands.syncMail')"
    :aria-haspopup="failureCount > 0 ? 'dialog' : undefined"
    :aria-expanded="failureCount > 0 ? failuresOpen : undefined"
    :aria-busy="syncing"
    :tone="failureCount > 0 ? 'warning' : !connection.online ? 'warning' : syncing ? 'accent' : 'default'"
    :disabled="failureCount === 0 && (syncing || !connection.online)"
    @mousedown.stop
    @click="activate"
  >
    <AppIcon :class="{ 'animate-spin': syncing }">
      <path d="M13.5 6A5.7 5.7 0 0 0 3.7 3.7L1.5 6m0-4v4h4M2.5 10a5.7 5.7 0 0 0 9.8 2.3l2.2-2.3m0 4v-4h-4" />
    </AppIcon>
    <span v-if="failureCount > 0" class="sync-button__failure" aria-hidden="true" />
    <span class="sr-only" role="status">{{ statusLabel }}</span>
  </IconButton>
  <SyncFailures v-if="failureCount > 0 || failuresOpen" :open="failuresOpen" @close="failuresOpen = false" />
  </span>
</template>

<style scoped>
.sync-control { display: inline-flex; }
.sync-button { position: relative; }
.sync-button__failure { position: absolute; top: var(--space-xs); right: var(--space-xs); width: var(--space-sm); height: var(--space-sm); border-radius: var(--radius-full); background: var(--accent-red); box-shadow: 0 0 0 2px var(--bg-base); }
.sync-button[aria-busy='true']:disabled {
  opacity: 1;
}
</style>
