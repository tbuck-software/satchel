<script setup lang="ts">
/**
 * Mail merge jobs at the top of the Outbox: progress, pause/resume/cancel,
 * and per-recipient results with a retry for the ones that failed. The copy
 * that is on its way sits in the Outbox list below like any other mail.
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import UsageMeter from '@/components/ui/UsageMeter.vue';
import { useMailMergeStore } from '@/stores/mailMerge';
import { useConfirm } from '@/composables/useConfirm';
import { useToast } from '@/composables/useToast';
import type { BackendMergeJob, BackendMergeJobAction, BackendMergeJobRecipient } from '@/types/mailMerge';
import { useMailEvents } from '@/composables/useMailEvents';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { outboxErrorLabel } from '@/utils/outboxLabels';
import { formatTimestamp } from '@/utils/composerText';

const { t } = useI18n();
const store = useMailMergeStore();
const confirm = useConfirm();
const toast = useToast();
const busy = ref<string | null>(null);

onMounted(() => void store.watch());
useMailEvents({ onMailMergeUpdated: () => void store.load() });
onBeforeUnmount(() => store.unwatch());

const jobs = computed(() => store.jobs);

function done(job: BackendMergeJob): number {
  return job.sent + job.failed + job.unknown + job.cancelled;
}

function statusLabel(job: BackendMergeJob): string {
  if (job.status === 'paused') {
    return job.pause_reason === 'failures' ? t('mailMerge.jobs.status.pausedFailures') : t('mailMerge.jobs.status.paused');
  }
  if (job.status === 'cancelled') return t('mailMerge.jobs.status.cancelled');
  if (job.status === 'completed') return t('mailMerge.jobs.status.completed');
  return t('mailMerge.jobs.status.running');
}

function counts(job: BackendMergeJob): string {
  const parts = [t('mailMerge.jobs.sent', { count: job.sent, total: job.total })];
  if (job.failed > 0) parts.push(t('mailMerge.jobs.failed', { count: job.failed }, job.failed));
  if (job.unknown > 0) parts.push(t('mailMerge.jobs.unknown', { count: job.unknown }, job.unknown));
  if (job.cancelled > 0) parts.push(t('mailMerge.jobs.cancelledCount', { count: job.cancelled }, job.cancelled));
  return parts.join(' · ');
}

function recipientStatus(row: BackendMergeJobRecipient): string {
  switch (row.status) {
    case 'sent': return t('mailMerge.jobs.row.sent');
    case 'queued': return t('mailMerge.jobs.row.queued');
    case 'failed': return t('mailMerge.jobs.row.failed');
    case 'unknown': return t('mailMerge.jobs.row.unknown');
    case 'cancelled': return t('mailMerge.jobs.row.cancelled');
    default: return t('mailMerge.jobs.row.pending');
  }
}

function recipientError(row: BackendMergeJobRecipient): string {
  if (!row.error) return '';
  if (row.error.startsWith('backend.errors.')) return backendErrorLabel({ key: row.error });
  return outboxErrorLabel(row.error) || row.error;
}

async function run(job: BackendMergeJob, action: BackendMergeJobAction) {
  if (action === 'cancel') {
    const remaining = job.pending + job.queued;
    const ok = await confirm.ask({
      title: t('mailMerge.jobs.cancelConfirm.title'),
      body: t('mailMerge.jobs.cancelConfirm.body', { count: remaining }, remaining),
      confirmLabel: t('mailMerge.jobs.cancel'),
      tone: 'danger',
    });
    if (!ok) return;
  }
  busy.value = job.id;
  try {
    await store.control(job.id, action);
  } catch (error) {
    toast.error(backendErrorLabel(error));
  } finally {
    busy.value = null;
  }
}

async function toggleDetail(job: BackendMergeJob) {
  if (store.detail?.job.id === job.id) {
    store.closeDetail();
    return;
  }
  try {
    await store.openDetail(job.id);
  } catch (error) {
    toast.error(backendErrorLabel(error));
  }
}
</script>

<template>
  <section v-if="jobs.length > 0" class="merge-jobs" data-testid="mail-merge-jobs">
    <SectionHeading :label="$t('mailMerge.jobs.title')" :count="jobs.length" />
    <AppBanner v-if="store.error" tone="danger" :title="$t('mailMerge.jobs.loadFailed')" :detail="backendErrorLabel(store.error)" />
    <ul class="merge-jobs__list">
      <li v-for="job in jobs" :key="job.id" class="merge-job" :data-status="job.status">
        <div class="merge-job__head">
          <div class="merge-job__titles">
            <span class="merge-job__subject">{{ job.subject }}</span>
            <span class="merge-job__meta">
              {{ statusLabel(job) }} · {{ job.source_label || $t('mailMerge.jobs.noSource') }} · {{ formatTimestamp(job.created_at) }}
            </span>
          </div>
          <div class="merge-job__actions">
            <AppButton v-if="job.status === 'running'" size="sm" variant="ghost" :disabled="busy === job.id" @click="run(job, 'pause')">{{ $t('mailMerge.jobs.pause') }}</AppButton>
            <AppButton v-if="job.status === 'paused'" size="sm" variant="soft" :disabled="busy === job.id" @click="run(job, 'resume')">{{ $t('mailMerge.jobs.resume') }}</AppButton>
            <AppButton v-if="job.failed > 0 && job.status !== 'cancelled'" size="sm" variant="soft" :disabled="busy === job.id" @click="run(job, 'retry_failed')">{{ $t('mailMerge.jobs.retry', { count: job.failed }, job.failed) }}</AppButton>
            <AppButton v-if="job.status === 'running' || job.status === 'paused'" size="sm" variant="ghost" :disabled="busy === job.id" @click="run(job, 'cancel')">{{ $t('mailMerge.jobs.cancel') }}</AppButton>
            <AppButton v-if="(job.status === 'completed' || job.status === 'cancelled') && job.queued === 0" size="sm" variant="ghost" :disabled="busy === job.id" @click="run(job, 'delete')">{{ $t('mailMerge.jobs.remove') }}</AppButton>
          </div>
        </div>
        <UsageMeter
          :value="done(job)"
          :max="Math.max(job.total, 1)"
          :label="$t('mailMerge.jobs.progressLabel', { subject: job.subject })"
          :value-text="counts(job)"
          :tone="job.failed > 0 ? 'warning' : 'neutral'"
        />
        <p class="merge-job__counts" aria-hidden="true">{{ counts(job) }}</p>
        <p v-if="job.status === 'running' && job.next_send_at && job.pending > 0 && job.queued === 0" class="merge-job__next">
          {{ $t('mailMerge.jobs.next', { time: formatTimestamp(job.next_send_at) }) }}
        </p>
        <AppButton size="sm" variant="link" :aria-expanded="store.detail?.job.id === job.id" @click="toggleDetail(job)">
          {{ store.detail?.job.id === job.id ? $t('mailMerge.jobs.hideRecipients') : $t('mailMerge.jobs.showRecipients') }}
        </AppButton>
        <ol v-if="store.detail?.job.id === job.id" class="merge-job__rows" data-testid="mail-merge-job-rows">
          <li v-for="row in store.detail.recipients" :key="row.id" class="merge-job__row" :data-status="row.status">
            <span class="merge-job__email">{{ row.email }}</span>
            <span class="merge-job__row-status">{{ recipientStatus(row) }}</span>
            <span v-if="row.error" class="merge-job__row-error">{{ recipientError(row) }}</span>
          </li>
        </ol>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.merge-jobs {
  display: grid;
  gap: var(--space-sm);
  margin-bottom: var(--space-lg);
  min-width: 0;
}

.merge-jobs__list {
  display: grid;
  gap: var(--space-sm);
  margin: 0;
  padding: 0;
  list-style: none;
}

.merge-job {
  display: grid;
  gap: var(--space-xs);
  padding: var(--space-md);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  min-width: 0;
}

.merge-job__head {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-sm);
  min-width: 0;
}

.merge-job__titles {
  display: grid;
  gap: var(--space-2xs);
  min-width: 0;
}

.merge-job__subject {
  overflow: hidden;
  color: var(--text-primary);
  font-weight: var(--weight-medium);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.merge-job__meta,
.merge-job__counts,
.merge-job__next {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  overflow-wrap: anywhere;
}

.merge-job__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs);
}

.merge-job__rows {
  display: grid;
  gap: var(--space-2xs);
  max-height: 280px;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
  font-size: var(--font-size-sm);
}

.merge-job__row {
  display: flex;
  flex-wrap: wrap;
  gap: 0 var(--space-sm);
  min-width: 0;
}

.merge-job__email {
  min-width: 0;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.merge-job__row-status {
  color: var(--text-secondary);
}

.merge-job__row[data-status='failed'] .merge-job__row-status {
  color: var(--accent-red);
}

.merge-job__row-error {
  flex-basis: 100%;
  color: var(--text-tertiary);
}
</style>
