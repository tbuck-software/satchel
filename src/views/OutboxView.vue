<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useUiStore } from '@/stores/ui';
import { useComposerStore } from '@/stores/composer';
import { useOutboxStore } from '@/stores/outbox';
import { useToast } from '@/composables/useToast';
import ListDetailLayout from '@/components/layout/ListDetailLayout.vue';
import {
  OUTBOX_STATUS_ORDER,
  outboxActionCanBeCancelled,
  outboxActionCanBeDismissed,
  outboxActionCanBeReopenedAsDraft,
  outboxActionCanBeRetried,
  outboxActionLabel,
  outboxAttemptsLabel,
  outboxErrorLabel,
  outboxIsOverdue,
  outboxRetryLabel,
  outboxStatusLabel,
  outboxStatusTone,
  outboxTimeLabel,
} from '@/utils/outboxLabels';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import IconButton from '@/components/ui/IconButton.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import SkeletonRow from '@/components/ui/SkeletonRow.vue';
import TimeAgo from '@/components/common/TimeAgo.vue';
import type { BackendOutboxItem } from '@/types/outbox';

const { locale, t } = useI18n();
const ui = useUiStore();
const outbox = useOutboxStore();
const composer = useComposerStore();
const toast = useToast();

/** Ids with a running retry/cancel — keeps double clicks from firing twice. */
const busy = ref(new Set<string>());

const selectedId = ref('');

const selectedItem = computed(() =>
  outbox.items.find((item) => item.id === selectedId.value),
);

onMounted(() => {
  // `load()` reports its own failure into `outbox.error` and the connection
  // banner, so there is no rejection left to handle here.
  void outbox.load();
});

// Keep the detail pane on something that still exists.
watch(
  () => outbox.items,
  (items) => {
    if (!items.some((item) => item.id === selectedId.value)) {
      selectedId.value = ui.isSinglePane ? '' : items[0]?.id ?? '';
    }
  },
  { immediate: true },
);

function closeDetail() {
  selectedId.value = '';
}

onBeforeUnmount(ui.registerBackNavigation({
  isOpen: () => ui.isSinglePane && Boolean(selectedId.value),
  dismiss: closeDetail,
}));

const failedCount = computed(
  () => outbox.items.filter((item) => item.status === 'failed' || isDeliveryWarning(item)).length,
);

const subtitle = computed(() => {
  // "All emails have been sent" is a lie once the list itself failed.
  if (outbox.error && outbox.items.length === 0) return t('outbox.loadFailed');
  if (outbox.items.length === 0) return t('outbox.subtitle.empty');
  if (failedCount.value > 0) return t('outbox.subtitle.failed', failedCount.value);
  return t('outbox.subtitle.sending');
});

/** Grouped by status so the failures sit on top instead of mixed into the queue. */
const groups = computed(() => {
  const known = new Set<string>(OUTBOX_STATUS_ORDER);
  const buckets = new Map<string, BackendOutboxItem[]>();
  for (const item of outbox.items) {
    const key = known.has(item.status) ? item.status : 'pending';
    const bucket = buckets.get(key);
    if (bucket) bucket.push(item);
    else buckets.set(key, [item]);
  }
  return OUTBOX_STATUS_ORDER.filter((status) => buckets.has(status)).map((status) => ({
    status,
    items: buckets.get(status) ?? [],
    label: status === 'scheduled' && (buckets.get(status) ?? []).some(isOverdue)
      ? t('outbox.overdue')
      : outboxStatusLabel(status),
  }));
});

/** The timestamp that says what happens next, not just what happened. */
function relevantTime(item: BackendOutboxItem): string | null {
  if (item.status === 'scheduled') return item.scheduled_for ?? item.created_at;
  if (item.status === 'failed') return item.next_retry_at ?? item.updated_at;
  if (item.status === 'sending' || isDeliveryWarning(item)) return item.updated_at;
  return item.created_at;
}

function isDeliveryWarning(item: BackendOutboxItem): boolean {
  return item.status === 'delivered' || item.status === 'delivery_unknown';
}

function exactTime(value?: string | null): string {
  if (!value) return '';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString(locale.value);
}

function isOverdue(item: BackendOutboxItem): boolean {
  return outboxIsOverdue(item.status, relevantTime(item));
}

/** Subject for mails; for background actions the action itself is the headline. */
function headline(item: BackendOutboxItem): string {
  if (item.subject) return item.subject;
  if (item.action_type === 'send_message') return t('compose.noSubject');
  return outboxActionLabel(item.action_type);
}

function recipientLine(item: BackendOutboxItem): string {
  if (item.recipients.length === 0) return '';
  if (item.recipients.length <= 2) return t('outbox.recipients', { recipients: item.recipients.join(', ') });
  return t('outbox.recipientsMore', {
    recipients: item.recipients.slice(0, 2).join(', '),
    count: item.recipients.length - 2,
  });
}

/** Small tertiary facts after the recipients — never a repeat of the headline. */
function metaParts(item: BackendOutboxItem): string[] {
  const parts: string[] = [];
  if (item.subject) parts.push(outboxActionLabel(item.action_type));
  if (item.retry_count > 0) parts.push(outboxAttemptsLabel(item.retry_count));
  return parts;
}

async function retry(item: BackendOutboxItem) {
  if (busy.value.has(item.id)) return;
  busy.value = new Set(busy.value).add(item.id);
  try {
    await outbox.retry(item.id);
    toast.success(t('outbox.retried'));
  } catch (err) {
    toast.error(t('outbox.retryFailed', { error: String(err) }));
  } finally {
    const next = new Set(busy.value);
    next.delete(item.id);
    busy.value = next;
  }
}

function canRetry(item: BackendOutboxItem): boolean {
  return outboxActionCanBeRetried(item.action_type, item.status, item.submission_started_at);
}

/**
 * The rescue path for a mail that cannot be resent: hand its text back to the
 * composer. Attachments stay behind — the queue carries their names, not their
 * bytes — so say so instead of losing them quietly.
 */
function openAsDraft(item: BackendOutboxItem) {
  composer.open({
    mode: 'new',
    to: [...item.recipients],
    cc: [...(item.cc ?? [])],
    bcc: [...(item.bcc ?? [])],
    subject: item.subject ?? '',
    body: item.body ?? '',
    bodyHtml: item.body_html ?? null,
  });
  const dropped = item.attachments?.length ?? 0;
  if (dropped > 0) toast.info(t('outbox.reopen.attachmentsDropped', dropped));
}

async function cancel(item: BackendOutboxItem) {
  if (busy.value.has(item.id)) return;
  busy.value = new Set(busy.value).add(item.id);
  try {
    await outbox.cancel(item.id);
    toast.info(t(isDeliveryWarning(item) ? 'outbox.dismissed' : 'outbox.cancelled'));
  } catch (err) {
    toast.error(t('outbox.cancelFailed', { error: String(err) }));
  } finally {
    const next = new Set(busy.value);
    next.delete(item.id);
    busy.value = next;
  }
}
</script>

<template>
  <ListDetailLayout :has-detail="!!selectedItem">
    <template #list>
    <section class="outbox">
    <PaneHeader :title="$t('outbox.title')" :subtitle="subtitle" :count="outbox.items.length">
      <template #actions>
        <IconButton
          :title="$t('common.refresh')"
          :disabled="outbox.loading"
          @click="outbox.load()"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            :class="{ spinning: outbox.loading }"
          >
            <path d="M13.5 8a5.5 5.5 0 1 1-1.7-3.9" />
            <path d="M13.5 2v3.2h-3.2" />
          </svg>
        </IconButton>
      </template>
    </PaneHeader>

    <div class="outbox__body">
      <SkeletonRow v-if="outbox.loading && outbox.items.length === 0" :count="3" height="76px" />

      <!-- A failed list leaves the queue empty. Without this branch the empty
           state would claim every mail went out. -->
      <ErrorState
        v-else-if="outbox.error && outbox.items.length === 0"
        :title="$t('outbox.loadFailed')"
        :detail="backendErrorLabel(outbox.error)"
        :busy="outbox.loading"
        @retry="outbox.load()"
      />

      <EmptyState
        v-else-if="outbox.items.length === 0"
        :title="$t('outbox.empty.title')"
        :description="$t('outbox.empty.description')"
      >
        <template #icon><AppIcon size="lg"><path d="M8 10V2m-3 3 3-3 3 3M3 9v4h10V9" /></AppIcon></template>
      </EmptyState>

      <div v-else class="groups">
        <section v-for="group in groups" :key="group.status" class="group">
          <SectionHeading :label="group.label" :count="group.items.length" />

          <ul class="list">
            <li
              v-for="item in group.items"
              :key="item.id"
              class="row"
              :class="[`row--${outboxStatusTone(item.status)}`, { 'row--busy': busy.has(item.id), 'row--active': item.id === selectedId }]"
              @click="selectedId = item.id"
            >
              <span class="row__marker" aria-hidden="true">
                <svg
                  v-if="item.status === 'failed' || isDeliveryWarning(item)"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                >
                  <circle cx="8" cy="8" r="5.6" />
                  <path d="M8 5.1v3.4" />
                  <path d="M8 10.7h.01" />
                </svg>
                <svg
                  v-else-if="item.status === 'sending'"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d="M14 2 2 6.6l4.8 2.1L9 13.6 14 2Z" />
                  <path d="m6.8 8.7 3-3" />
                </svg>
                <svg
                  v-else-if="item.status === 'scheduled'"
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <circle cx="8" cy="8" r="5.6" />
                  <path d="M8 4.8V8l2.3 1.4" />
                </svg>
                <svg
                  v-else
                  viewBox="0 0 16 16"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                >
                  <circle cx="8" cy="8" r="5.6" />
                  <path d="M5.4 8h.01M8 8h.01M10.6 8h.01" />
                </svg>
              </span>

              <div class="row__body">
                <div class="row__headline">
                  <span class="row__subject">{{ headline(item) }}</span>
                  <span v-if="isOverdue(item)" class="row__time">{{ $t('outbox.overdue') }}</span>
                  <TimeAgo
                    v-else-if="relevantTime(item)"
                    :date="relevantTime(item) as string"
                    class="row__time"
                    :title="`${outboxTimeLabel(item.status, relevantTime(item))}: ${exactTime(relevantTime(item))}`"
                  />
                </div>

                <p
                  v-if="recipientLine(item) || metaParts(item).length > 0"
                  class="row__meta"
                >
                  <span v-if="recipientLine(item)" class="row__to">{{ recipientLine(item) }}</span>
                  <template v-for="(part, index) in metaParts(item)" :key="part">
                    <span v-if="index > 0 || recipientLine(item)" class="row__dot">·</span>
                    <span>{{ part }}</span>
                  </template>
                </p>

                <p v-if="item.last_error" class="row__reason">
                  {{ outboxErrorLabel(item.last_error) }}
                </p>
              </div>

              <div class="row__actions">
                <AppButton
                  v-if="item.status === 'failed' && canRetry(item)"
                  size="sm"
                  variant="soft"
                  :disabled="busy.has(item.id)"
                  @click.stop="retry(item)"
                >
                  {{ outboxRetryLabel(item.action_type, item.status) }}
                </AppButton>
                <IconButton
                  v-else-if="canRetry(item)"
                  size="sm"
                  :title="outboxRetryLabel(item.action_type, item.status)"
                  :disabled="busy.has(item.id)"
                  @click.stop="retry(item)"
                >
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 16 16"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <path d="M13.5 8a5.5 5.5 0 1 1-1.7-3.9" />
                    <path d="M13.5 2v3.2h-3.2" />
                  </svg>
                </IconButton>
                <IconButton
                  v-if="outboxActionCanBeCancelled(item.action_type, item.status) || outboxActionCanBeDismissed(item.action_type, item.status)"
                  size="sm"
                  :tone="isDeliveryWarning(item) ? 'default' : 'danger'"
                  :title="$t(isDeliveryWarning(item) ? 'outbox.dismiss' : 'common.discard')"
                  :disabled="busy.has(item.id)"
                  @click.stop="cancel(item)"
                >
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 16 16"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  >
                    <template v-if="isDeliveryWarning(item)">
                      <path d="M4 4l8 8M12 4 4 12" />
                    </template>
                    <template v-else>
                      <path d="M2.5 4h11M5.5 4V2.8A1.3 1.3 0 0 1 6.8 1.5h2.4a1.3 1.3 0 0 1 1.3 1.3V4" />
                      <path d="M12.6 4v9a1.5 1.5 0 0 1-1.5 1.5h-6.2A1.5 1.5 0 0 1 3.4 13V4" />
                    </template>
                  </svg>
                </IconButton>
              </div>
            </li>
          </ul>
        </section>
      </div>
    </div>
    </section>
    </template>

    <template #detail>
      <article v-if="selectedItem" class="detail">
        <header class="detail__header">
          <IconButton v-if="ui.isSinglePane" :title="$t('common.back')" @click="closeDetail">
            <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="m10.5 3-5 5 5 5" />
            </svg>
          </IconButton>
          <div class="detail__copy">
            <p class="detail__status" :class="`detail__status--${outboxStatusTone(selectedItem.status)}`">
              {{ isOverdue(selectedItem) ? $t('outbox.overdue') : outboxStatusLabel(selectedItem.status) }}
            </p>
            <h2 class="detail__subject">{{ headline(selectedItem) }}</h2>
            <p v-if="selectedItem.recipients.length > 0" class="detail__line">
              {{ $t('common.to') }} {{ selectedItem.recipients.join(', ') }}
            </p>
          </div>
          <div class="detail__actions">
            <AppButton
              v-if="canRetry(selectedItem)"
              size="sm"
              :variant="selectedItem.status === 'failed' ? 'primary' : 'soft'"
              :disabled="busy.has(selectedItem.id)"
              @click="retry(selectedItem)"
            >
              {{ outboxRetryLabel(selectedItem.action_type, selectedItem.status) }}
            </AppButton>
            <AppButton
              v-if="outboxActionCanBeReopenedAsDraft(selectedItem.action_type, selectedItem.status)"
              size="sm"
              variant="soft"
              data-test="outbox-open-as-draft"
              @click="openAsDraft(selectedItem)"
            >
              {{ $t('outbox.openAsDraft') }}
            </AppButton>
            <AppButton
              v-if="outboxActionCanBeCancelled(selectedItem.action_type, selectedItem.status) || outboxActionCanBeDismissed(selectedItem.action_type, selectedItem.status)"
              size="sm"
              variant="ghost"
              :disabled="busy.has(selectedItem.id)"
              @click="cancel(selectedItem)"
            >
              {{ $t(isDeliveryWarning(selectedItem) ? 'outbox.dismiss' : 'common.discard') }}
            </AppButton>
          </div>
        </header>

        <p v-if="selectedItem.action_type === 'remote_mutation'" class="move-discard-hint">
          {{ t('outbox.moveDiscardHint') }}
        </p>

        <dl class="detail__facts">
          <div>
            <dt>{{ $t('outbox.action.label') }}</dt>
            <dd>{{ outboxActionLabel(selectedItem.action_type) }}</dd>
          </div>
          <div>
            <dt>{{ isDeliveryWarning(selectedItem) ? outboxTimeLabel(selectedItem.status, relevantTime(selectedItem)) : $t('outbox.time.queued') }}</dt>
            <dd>{{ exactTime(isDeliveryWarning(selectedItem) ? relevantTime(selectedItem) : selectedItem.created_at) }}</dd>
          </div>
          <div v-if="selectedItem.scheduled_for">
            <dt>{{ $t('outbox.time.scheduledFor') }}</dt>
            <dd>{{ exactTime(selectedItem.scheduled_for) }}</dd>
          </div>
          <div v-if="selectedItem.next_retry_at">
            <dt>{{ $t('outbox.time.nextRetry') }}</dt>
            <dd>{{ exactTime(selectedItem.next_retry_at) }}</dd>
          </div>
          <div v-if="selectedItem.retry_count > 0">
            <dt>{{ $t('outbox.attemptCount') }}</dt>
            <dd>{{ selectedItem.retry_count }}</dd>
          </div>
        </dl>

        <div v-if="selectedItem.last_error" class="detail__error">
          <h3>{{ $t('outbox.error.heading') }}</h3>
          <p>{{ outboxErrorLabel(selectedItem.last_error) }}</p>
          <details v-if="outboxErrorLabel(selectedItem.last_error) !== selectedItem.last_error">
            <summary>{{ $t('outbox.error.technical') }}</summary>
            <pre>{{ selectedItem.last_error }}</pre>
          </details>
        </div>
      </article>
      <div v-else class="detail-placeholder">
        <EmptyState
          :title="$t('outbox.detail.emptyTitle')"
          :description="$t('outbox.detail.emptyDescription')"
        />
      </div>
    </template>
  </ListDetailLayout>
</template>

<style scoped>
.move-discard-hint { margin: var(--space-md) var(--space-xl); font-size: var(--font-size-sm); color: var(--text-secondary); }
.outbox {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  min-width: 0;
}

.outbox__body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 4px var(--space-md) var(--space-xl);
}

.groups {
  display: flex;
  flex-direction: column;
  gap: var(--space-lg);
}

.group {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.list {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  list-style: none;
  margin: 0;
  padding: 0;
}

.row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: start;
  gap: var(--space-md);
  padding: var(--space-md) var(--space-md) var(--space-md) var(--space-sm);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  border: var(--border-muted);
  transition:
    background var(--duration-fast) var(--ease-out),
    border-color var(--duration-fast) var(--ease-out),
    opacity var(--duration-fast) var(--ease-out);
}

.row:hover {
  background: var(--bg-hover);
}

.row {
  cursor: pointer;
}

.row--active {
  border-color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-surface));
}

.row--busy {
  opacity: 0.55;
  pointer-events: none;
}

/* Status marker — a tinted disc, the same language as the workflow chips. */
.row__marker {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  margin-top: 1px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-tertiary);
}

.row__marker svg {
  width: 16px;
  height: 16px;
}

.row--danger {
  border-color: color-mix(in srgb, var(--accent-red) 30%, transparent);
  background: color-mix(in srgb, var(--accent-red) 5%, var(--bg-surface));
}

.row--danger:hover {
  background: color-mix(in srgb, var(--accent-red) 9%, var(--bg-surface));
}

.row--danger .row__marker {
  background: color-mix(in srgb, var(--accent-red) 16%, transparent);
  color: var(--accent-red);
}

.row--accent .row__marker {
  background: color-mix(in srgb, var(--accent-primary) 16%, transparent);
  color: var(--accent-primary);
}

.row--warning .row__marker {
  background: color-mix(in srgb, var(--accent-amber) 16%, transparent);
  color: var(--accent-amber);
}

.row__body {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.row__headline {
  display: flex;
  align-items: baseline;
  gap: var(--space-sm);
  min-width: 0;
}

.row__subject {
  flex: 1;
  min-width: 0;
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__time {
  flex-shrink: 0;
}

.row__meta {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin: 0;
  min-width: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.row__to {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__meta > span:not(.row__to) {
  flex-shrink: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.row__dot {
  color: var(--text-tertiary);
}

/* The reason a mail is stuck — readable prose, not a red terminal block. */
.row__reason {
  margin: 5px 0 0;
  padding-left: 10px;
  border-left: 2px solid color-mix(in srgb, var(--accent-red) 45%, transparent);
  font-size: var(--font-size-sm);
  line-height: 1.45;
  color: var(--text-secondary);
}

.row__actions {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
  flex-shrink: 0;
  opacity: 0.55;
  transition: opacity var(--duration-fast) var(--ease-out);
}

.row:hover .row__actions,
.row:focus-within .row__actions,
.row--danger .row__actions {
  opacity: 1;
}

.spinning {
  animation: spin 900ms linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@media (prefers-reduced-motion: reduce) {
  .spinning {
    animation: none;
  }
}

/* ── Detail pane ── */
.detail {
  height: 100%;
  display: block;
  overflow-y: auto;
}

.detail__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-md);
  padding: var(--space-lg) var(--space-xl) var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
}

.detail__copy {
  flex: 1;
  min-width: 0;
  display: grid;
  gap: 4px;
}

.detail__status {
  margin: 0;
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-tertiary);
}

.detail__status--danger {
  color: var(--accent-red);
}

.detail__status--accent {
  color: var(--accent-primary);
}

.detail__status--warning {
  color: var(--accent-amber);
}

.detail__subject {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.detail__line {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  overflow-wrap: anywhere;
}

.detail__actions {
  display: flex;
  gap: var(--space-xs);
  flex-shrink: 0;
}

.detail__facts {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: var(--space-md);
  margin: 0;
  padding: var(--space-lg) var(--space-xl) 0;
}

.detail__facts dt {
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-tertiary);
}

.detail__facts dd {
  margin: 2px 0 0;
  font-size: var(--font-size-sm);
  color: var(--text-primary);
}

.detail__error {
  margin: var(--space-lg) var(--space-xl) var(--space-xl);
  padding: var(--space-md);
  border-left: 2px solid color-mix(in srgb, var(--accent-red) 45%, transparent);
  background: color-mix(in srgb, var(--accent-red) 4%, var(--bg-surface));
  border-radius: var(--radius-md);
}

.detail__error h3 {
  margin: 0 0 4px;
  font-size: var(--font-size-sm);
  color: var(--text-primary);
}

.detail__error p {
  margin: 0;
  font-size: var(--font-size-sm);
  line-height: 1.5;
  color: var(--text-secondary);
}

.detail__error details {
  margin-top: var(--space-sm);
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.detail__error pre {
  margin: 6px 0 0;
  padding: var(--space-sm);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  overflow-x: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.detail-placeholder {
  height: 100%;
  display: grid;
  place-items: center;
}
</style>
