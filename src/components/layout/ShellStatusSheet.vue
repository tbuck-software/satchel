<script setup lang="ts">
/**
 * Everything the phone's status chip stands for, in full: the expired
 * sign-ins, the backend's report with its fix, the queue items to review,
 * and when mail last arrived. The notices are the same components the
 * desktop sidebar shows, as cards, so both say the same thing.
 */
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AccountReauthBanner from '@/components/settings/AccountReauthBanner.vue';
import ConnectionBanner from './ConnectionBanner.vue';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import { useMailSync } from '@/composables/useMailSync';
import { formatListTime } from '@/utils/listTime';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: []; review: [] }>();
const { t } = useI18n();
const accounts = useAccountsStore();
const connection = useConnectionStore();
const outbox = useOutboxStore();
const { syncing, syncMail } = useMailSync();
const content = ref<HTMLElement | null>(null);

const reauthNeeded = computed(() => Boolean(accounts.credentialLock) || accounts.accountsWithAuthError.length > 0);
const failureCount = computed(() => outbox.attentionCount);
const lastSynced = computed(() => {
  if (!connection.online) return t('app.sync.status.offline');
  if (syncing.value) return t('app.sync.status.syncing');
  if (!connection.lastSyncAt) return t('app.sync.status.never');
  return t('app.sync.status.lastSynced', { time: formatListTime(connection.lastSyncAt) });
});

let trigger: HTMLElement | null = null;
watch(() => props.open, async (open) => {
  if (open) {
    trigger = document.activeElement as HTMLElement | null;
    await nextTick();
    content.value?.querySelector<HTMLElement>('button')?.focus();
  } else {
    trigger?.focus({ preventScroll: true });
  }
}, { immediate: true });
</script>

<template>
  <Teleport to="body">
    <BottomSheet :open="open" :label="t('app.status.title')" desktop-width="420px" @dismiss="emit('close')">
      <div ref="content" class="status-sheet">
        <div class="status-sheet__heading">
          <h2>{{ t('app.status.title') }}</h2>
          <IconButton :title="t('common.close')" @click="emit('close')">
            <AppIcon><path d="m4 4 8 8m0-8-8 8" /></AppIcon>
          </IconButton>
        </div>
        <div class="status-sheet__notices">
          <AccountReauthBanner v-if="reauthNeeded" variant="card" />
          <ConnectionBanner variant="card" :host="false" />
          <AppBanner
            v-if="failureCount > 0"
            variant="card"
            :title="t('app.sync.failures.label', { count: failureCount }, failureCount)"
          >
            <template #actions>
              <AppButton size="sm" variant="soft" aria-haspopup="dialog" @click="emit('review')">
                {{ t('app.sync.failures.review') }}
              </AppButton>
            </template>
          </AppBanner>
        </div>
        <div class="status-sheet__footer">
          <span class="status-sheet__last" role="status">{{ lastSynced }}</span>
          <AppButton size="sm" variant="ghost" :disabled="syncing || !connection.online" @click="void syncMail()">
            {{ t('app.status.syncNow') }}
          </AppButton>
        </div>
      </div>
    </BottomSheet>
  </Teleport>
</template>

<style scoped>
.status-sheet { padding: var(--space-lg); }
.status-sheet__heading { display: flex; justify-content: space-between; align-items: center; gap: var(--space-md); margin-bottom: var(--space-md); }
h2 { margin: 0; font-size: var(--font-size-lg); font-weight: var(--weight-semibold); }
.status-sheet__notices { display: flex; flex-direction: column; gap: var(--space-sm); max-height: 60vh; overflow: auto; }
.status-sheet__footer { display: flex; align-items: center; justify-content: space-between; gap: var(--space-md); margin-top: var(--space-md); }
.status-sheet__last { min-width: 0; color: var(--text-tertiary); font-size: var(--font-size-xs); }
</style>
