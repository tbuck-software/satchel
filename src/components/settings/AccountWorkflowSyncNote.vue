<script setup lang="ts">
/**
 * Shown only for an account whose mail server cannot store Satchel's
 * keywords: Done and Snooze then stay on this device instead of syncing
 * through the server. Every other account says nothing here.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useWorkflowSyncStore } from '@/stores/workflowSync';

const props = defineProps<{ accountId: string }>();

const { t } = useI18n();
const workflowSync = useWorkflowSyncStore();
const localOnly = computed(() => workflowSync.isLocalOnly(props.accountId));
</script>

<template>
  <section v-if="localOnly" class="workflow-sync-note" :aria-label="t('workflowSync.localOnlyTitle')" data-testid="workflow-sync-local-only">
    <span class="workflow-sync-note__title">{{ t('workflowSync.localOnlyTitle') }}</span>
    <p class="workflow-sync-note__body">{{ t('workflowSync.localOnlyBody') }}</p>
  </section>
</template>

<style scoped>
.workflow-sync-note { display: grid; gap: var(--space-xs); flex-basis: 100%; padding-top: var(--space-md); border-top: 1px solid var(--border-color-muted); }
.workflow-sync-note__title { color: var(--text-primary); font-size: var(--font-size-sm); font-weight: var(--weight-semibold); }
.workflow-sync-note__body { margin: 0; color: var(--text-secondary); font-size: var(--font-size-xs); }
</style>
