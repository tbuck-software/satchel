<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useMailSync } from '@/composables/useMailSync';
import { useConnectionStore } from '@/stores/connection';
import { formatListTime } from '@/utils/listTime';

const { t } = useI18n();
const connection = useConnectionStore();
const { syncing, syncMail } = useMailSync();

const statusLabel = computed(() => {
  // Standing failures already have an actionable explanation in the banner.
  if (connection.syncStatus === 'error') return '';
  if (!connection.online) return t('app.sync.status.offline');
  if (syncing.value) return t('app.sync.status.syncing');
  if (!connection.lastSyncAt) return t('app.sync.status.never');
  return t('app.sync.status.lastSynced', { time: formatListTime(connection.lastSyncAt) });
});
const title = computed(() => [t('commands.syncMail'), statusLabel.value].filter(Boolean).join(' · '));
</script>

<template>
  <IconButton
    class="sync-button"
    size="sm"
    :title="title"
    :aria-label="t('commands.syncMail')"
    :aria-busy="syncing"
    :tone="!connection.online ? 'warning' : syncing ? 'accent' : 'default'"
    :disabled="syncing || !connection.online"
    @mousedown.stop
    @click="syncMail"
  >
    <AppIcon :class="{ 'animate-spin': syncing }">
      <path d="M13.5 6A5.7 5.7 0 0 0 3.7 3.7L1.5 6m0-4v4h4M2.5 10a5.7 5.7 0 0 0 9.8 2.3l2.2-2.3m0 4v-4h-4" />
    </AppIcon>
    <span class="sr-only" role="status">{{ statusLabel }}</span>
  </IconButton>
</template>

<style scoped>
.sync-button[aria-busy='true']:disabled {
  opacity: 1;
}
</style>
