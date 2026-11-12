<script setup lang="ts">
import { ref } from 'vue';
import { useFocusTrap } from '@/composables/useFocusTrap';
import { useI18n } from 'vue-i18n';
import { useConnectionStore } from '@/stores/connection';
import { progressPercent, useSyncProgress } from '@/composables/useSyncProgress';
import { formatListTime } from '@/utils/listTime';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ProgressBar from '@/components/ui/ProgressBar.vue';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const content = ref<HTMLElement | null>(null);
// Keep the sheet's non-modal Tab behavior, with shared focus entry and return.
useFocusTrap(content);
const connection = useConnectionStore();
const progress = useSyncProgress();
const { t, n } = useI18n();

type AccountProgress = (typeof connection.syncProgress)[number];
const importing = (account: AccountProgress) => account.backfill_remaining > 0 && account.backfill_total > 0;
const done = (account: AccountProgress) => Math.max(0, account.backfill_total - account.backfill_remaining);
const percent = (account: AccountProgress) => n(progressPercent(done(account), account.backfill_total), 'percent');
const counts = (account: AccountProgress) => t('app.sync.progress.counts', { done: n(done(account)), total: n(account.backfill_total) });

function phase(account: AccountProgress): string {
  if (!connection.online) return t('app.sync.progress.paused');
  if (account.phase === 'complete') return t('app.sync.progress.complete');
  if (account.phase === 'error') return t('app.sync.progress.needsAttention');
  if (!connection.syncing || account.phase === 'waiting') return t('app.sync.progress.waiting');
  if (account.folder && account.folder_index && account.folder_total) {
    return t('app.sync.progress.folderCount', { folder: account.folder, current: account.folder_index, total: account.folder_total });
  }
  return t('app.sync.progress.preparing');
}

/** Counts and phase share one quiet line under the bar. */
const meta = (account: AccountProgress) => [importing(account) ? counts(account) : '', phase(account)].filter(Boolean).join(' · ');
</script>

<template>
  <Teleport to="body">
    <BottomSheet :open="props.open" :label="t('app.sync.progress.title')" desktop-width="380px" @dismiss="emit('close')">
      <div ref="content" class="sync-progress-details">
        <PaneHeader :title="t('app.sync.progress.title')">
          <template #actions>
            <IconButton :title="t('common.close')" @click="emit('close')">
              <AppIcon><path d="m4 4 8 8m0-8-8 8" /></AppIcon>
            </IconButton>
          </template>
        </PaneHeader>
        <div class="sync-progress-details__body">
          <p class="sync-progress-details__explanation">{{ t('app.sync.progress.explanation') }}</p>
          <ul v-if="connection.syncProgress.length > 0" class="sync-progress-details__accounts">
            <li v-for="account in connection.syncProgress" :key="account.account_id" class="sync-progress-account"
              :title="t('app.sync.progress.updated', { time: formatListTime(account.updated_at) })">
              <span class="sync-progress-account__line">
                <strong class="sync-progress-account__name">{{ account.account_name || account.email }}</strong>
                <span v-if="importing(account)" class="sync-progress-account__percent">{{ percent(account) }}</span>
              </span>
              <ProgressBar v-if="importing(account)"
                :value="account.backfill_total - account.backfill_remaining" :max="account.backfill_total"
                :label="account.account_name || account.email" :value-text="counts(account)" />
              <span class="sync-progress-account__meta">{{ meta(account) }}</span>
            </li>
          </ul>
          <p v-else role="status" class="sync-progress-details__empty">{{ connection.syncStatus === 'error' ? t('app.sync.progress.needsAttention') : progress.active.value ? progress.phase.value : t('app.sync.progress.complete') }}</p>
        </div>
      </div>
    </BottomSheet>
  </Teleport>
</template>

<style scoped>
.sync-progress-details { color: var(--text-primary); }
.sync-progress-details__body { display: grid; gap: var(--space-md); padding: 0 var(--space-lg) var(--space-lg); }
.sync-progress-details__explanation, .sync-progress-details__empty { margin: 0; font-size: var(--font-size-xs); line-height: 1.5; color: var(--text-secondary); }
.sync-progress-details__accounts { display: grid; gap: var(--space-md); margin: 0; padding: 0; list-style: none; }
.sync-progress-account { display: grid; gap: var(--space-xs); min-width: 0; }
.sync-progress-account__line { display: flex; align-items: baseline; gap: var(--space-sm); min-width: 0; font-size: var(--font-size-sm); }
.sync-progress-account__name { flex: 1; min-width: 0; overflow: hidden; font-weight: var(--weight-medium); text-overflow: ellipsis; white-space: nowrap; }
.sync-progress-account__percent { flex-shrink: 0; color: var(--text-secondary); font-variant-numeric: tabular-nums; }
.sync-progress-account__meta { font-size: var(--font-size-xs); color: var(--text-tertiary); font-variant-numeric: tabular-nums; }
</style>
