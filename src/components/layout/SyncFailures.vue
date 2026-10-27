<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import { useUiStore } from '@/stores/ui';
import { useMailSync } from '@/composables/useMailSync';
import { useToast } from '@/composables/useToast';
import { outboxActionLabel, outboxErrorLabel } from '@/utils/outboxLabels';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const connection = useConnectionStore();
const outbox = useOutboxStore();
const router = useRouter();
const ui = useUiStore();
const toast = useToast();
const busy = ref(false);
const { syncing, syncMail } = useMailSync();
const content = ref<HTMLElement | null>(null);
const failures = computed(() => outbox.items.filter((item) =>
  ['failed', 'delivery_unknown', 'delivered'].includes(item.status),
));
let trigger: HTMLElement | null = null;
watch(() => props.open, async (open) => {
  if (open) {
    trigger = document.activeElement as HTMLElement | null;
    void outbox.load();
    await nextTick();
    content.value?.querySelector<HTMLElement>('button')?.focus();
  } else {
    trigger?.focus({ preventScroll: true });
  }
}, { immediate: true });
onBeforeUnmount(() => trigger?.focus({ preventScroll: true }));
async function retryChanges() {
  busy.value = true;
  try {
    const count = await outbox.retryFailedSyncActions();
    toast.info(t('app.sync.retryQueued', { count }, count));
  } catch (err) {
    toast.error(t('app.sync.retryFailed', { error: backendErrorLabel(err) }));
  } finally {
    busy.value = false;
  }
}
function openOutbox() {
  emit('close');
  ui.closeMobileSidebar();
  void router.push({ name: 'outbox' });
}
</script>

<template>
  <Teleport to="body">
    <BottomSheet :open="open" :label="t('app.sync.failures.title')" desktop-width="420px" @dismiss="emit('close')">
      <div ref="content" class="sync-failures">
        <div class="sync-failures__heading">
          <h2>{{ t('app.sync.failures.title') }}</h2>
          <IconButton :title="t('common.close')" @click="emit('close')">
            <AppIcon><path d="m4 4 8 8m0-8-8 8" /></AppIcon>
          </IconButton>
        </div>
        <p>{{ t('app.sync.failures.explanation') }}</p>
        <ul v-if="failures.length" class="sync-failures__list">
          <li v-for="item in failures" :key="item.id">
            <strong>{{ item.subject || outboxActionLabel(item.action_type) }}</strong>
            <span class="sync-failures__reason">{{ outboxErrorLabel(item.last_error) }}</span>
          </li>
        </ul>
        <p v-else>{{ t(connection.failedOutbox > 0 ? 'app.sync.failures.unavailable' : 'app.sync.failures.resolved') }}</p>
        <div class="sync-failures__actions">
          <AppButton variant="ghost" :disabled="syncing || !connection.online" @click="syncMail">
            {{ t('commands.syncMail') }}
          </AppButton>
          <AppButton v-if="connection.failedRemoteMutations > 0 || outbox.failedChangeCount > 0" variant="soft" :disabled="busy || !connection.online" @click="retryChanges">
            {{ t('common.retry') }}
          </AppButton>
          <AppButton variant="primary" @click="openOutbox">{{ t('nav.folders.outbox') }}</AppButton>
        </div>
      </div>
    </BottomSheet>
  </Teleport>
</template>

<style scoped>
.sync-failures { padding: var(--space-lg); }
.sync-failures__heading { display: flex; justify-content: space-between; align-items: center; gap: var(--space-md); }
h2 { margin: 0; font-size: var(--font-size-lg); font-weight: var(--weight-semibold); }
p { color: var(--text-secondary); margin: var(--space-md) 0; font-size: var(--font-size-sm); }
.sync-failures__list { list-style: none; margin: 0; padding: 0; max-height: 40vh; overflow: auto; }
li { display: grid; gap: var(--space-xs); padding: var(--space-md) 0; border-top: var(--border-default); font-size: var(--font-size-sm); }
strong { font-weight: var(--weight-semibold); overflow-wrap: anywhere; }
.sync-failures__reason { color: var(--accent-red-soft); }
.sync-failures__actions { display: flex; justify-content: flex-end; gap: var(--space-sm); margin-top: var(--space-lg); }
</style>
