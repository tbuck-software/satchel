<script setup lang="ts">
/**
 * Shown only for an account whose mail server cannot store Satchel's
 * keywords: Done and Snooze then stay on this device instead of syncing
 * through the server. Every other account says nothing here.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import AppBanner from '@/components/ui/AppBanner.vue';
import { useWorkflowSyncStore } from '@/stores/workflowSync';

const props = defineProps<{ accountId: string }>();

const { t } = useI18n();
const workflowSync = useWorkflowSyncStore();
const reason = computed(() => workflowSync.reasonFor(props.accountId));
// Static keys, so the catalogue check sees every one.
const body = computed(() => {
  if (reason.value === 'store_refused') return t('workflowSync.storeRefusedBody');
  if (reason.value === 'gmail_imap') return t('workflowSync.gmailImapBody');
  return t('workflowSync.localOnlyBody');
});
</script>

<template>
  <AppBanner
    v-if="reason"
    class="workflow-sync-note"
    tone="info"
    :title="t('workflowSync.localOnlyTitle')"
    :detail="body"
    data-testid="workflow-sync-local-only"
  />
</template>

<style scoped>
/* Full row in the account card's wrapping layout. */
.workflow-sync-note { flex-basis: 100%; }
</style>
