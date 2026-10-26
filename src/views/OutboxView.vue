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
  type OutboxTone,
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
import Avatar from '@/components/common/Avatar.vue';
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

/** Failures and delivery warnings wait for a decision; the rest runs on its own. */
function needsAttention(item: BackendOutboxItem): boolean {
  return item.status === 'failed' || isDeliveryWarning(item);
}

/** The status colour, with a scheduled send that missed its time turned amber. */
function markerTone(item: BackendOutboxItem): OutboxTone {
  return isOverdue(item) ? 'warning' : outboxStatusTone(item.status);
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

/** The row's first line, where an inbox row names the sender. */
function who(item: BackendOutboxItem): string {
  return recipientLine(item) || outboxActionLabel(item.action_type);
}

function recipientLine(item: BackendOutboxItem): string {
  if (item.recipients.length === 0) return '';
  if (item.recipients.length <= 2) return t('outbox.recipients', { recipients: item.recipients.join(', ') });
  return t('outbox.recipientsMore', {
    recipients: item.recipients.slice(0, 2).join(', '),
    count: item.recipients.length - 2,
  });
}

/**
 * Small tertiary facts under the subject — never a repeat of the first line
 * or the headline. Sending is what a mail with recipients does, so only
 * another action is named.
 */
function metaParts(item: BackendOutboxItem): string[] {
  const parts: string[] = [];
  if (item.subject && item.recipients.length > 0 && item.action_type !== 'send_message') {
    parts.push(outboxActionLabel(item.action_type));
  }
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
              :class="{
                'row--attention': needsAttention(item),
                'row--busy': busy.has(item.id),
                'row--active': item.id === selectedId,
              }"
              :aria-current="item.id === selectedId ? 'true' : undefined"
              @click="selectedId = item.id"
            >
              <!-- The same lead as an inbox row: who it goes to, with the queue
                   state pinned to the avatar the way unread is. -->
              <span class="row__lead">
                <Avatar
                  v-if="item.recipients.length > 0"
                  :name="item.recipients[0] ?? ''"
                  :email="item.recipients[0]"
                  :size="36"
                />
                <span v-else class="row__action-icon" aria-hidden="true">
                  <AppIcon>
                    <path d="M2.5 5.5h7.8M8.3 3.5l2 2-2 2M13.5 10.5H5.7M7.7 8.5l-2 2 2 2" />
                  </AppIcon>
                </span>
                <span
                  v-if="item.status !== 'pending'"
                  class="row__marker"
                  :class="`row__marker--${markerTone(item)}`"
                  aria-hidden="true"
                >
                  <svg v-if="needsAttention(item)" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
                    <path d="M8 3.5v5" />
                    <path d="M8 12.2h.01" />
                  </svg>
                  <svg v-else-if="item.status === 'sending'" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3 8h9M8.5 4.5 12 8l-3.5 3.5" />
                  </svg>
                  <svg v-else viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M8 3.5V8l3 2" />
                  </svg>
                </span>
              </span>

              <div class="row__body">
                <div class="row__top" :class="{ 'row__top--quick': !needsAttention(item) }">
                  <span class="row__who">{{ who(item) }}</span>
                  <span v-if="isOverdue(item)" class="row__time row__time--overdue">{{ $t('outbox.overdue') }}</span>
                  <TimeAgo
                    v-else-if="relevantTime(item)"
                    :date="relevantTime(item) as string"
                    class="row__time"
                    :title="`${outboxTimeLabel(item.status, relevantTime(item))}: ${exactTime(relevantTime(item))}`"
                  />
                  <!-- Queued work is calm: its actions wait under the pointer,
                       as the inbox row's do. -->
                  <span v-if="!needsAttention(item)" class="row__quick">
                    <IconButton
                      v-if="canRetry(item)"
                      size="sm"
                      :title="outboxRetryLabel(item.action_type, item.status)"
                      :disabled="busy.has(item.id)"
                      @click.stop="retry(item)"
                    >
                      <AppIcon size="sm">
                        <path d="M13.5 8a5.5 5.5 0 1 1-1.7-3.9" />
                        <path d="M13.5 2v3.2h-3.2" />
                      </AppIcon>
                    </IconButton>
                    <IconButton
                      v-if="outboxActionCanBeCancelled(item.action_type, item.status)"
                      size="sm"
                      tone="danger"
                      :title="$t('common.discard')"
                      :disabled="busy.has(item.id)"
                      @click.stop="cancel(item)"
                    >
                      <AppIcon size="sm">
                        <path d="M2.5 4h11M5.5 4V2.8A1.3 1.3 0 0 1 6.8 1.5h2.4a1.3 1.3 0 0 1 1.3 1.3V4" />
                        <path d="M12.6 4v9a1.5 1.5 0 0 1-1.5 1.5h-6.2A1.5 1.5 0 0 1 3.4 13V4" />
                      </AppIcon>
                    </IconButton>
                  </span>
                </div>

                <span v-if="headline(item) !== who(item)" class="row__subject">{{ headline(item) }}</span>

                <p v-if="item.last_error" class="row__reason">
                  {{ outboxErrorLabel(item.last_error) }}
                </p>
                <p v-else-if="metaParts(item).length > 0" class="row__meta">
                  {{ metaParts(item).join(' · ') }}
                </p>

                <!-- What needs a decision says so in words, right where the
                     reason is, instead of behind a hover. -->
                <div v-if="needsAttention(item)" class="row__fix">
                  <AppButton
                    v-if="canRetry(item)"
                    size="sm"
                    variant="soft"
                    :disabled="busy.has(item.id)"
                    @click.stop="retry(item)"
                  >
                    {{ outboxRetryLabel(item.action_type, item.status) }}
                  </AppButton>
                  <AppButton
                    v-if="outboxActionCanBeCancelled(item.action_type, item.status) || outboxActionCanBeDismissed(item.action_type, item.status)"
                    size="sm"
                    variant="ghost"
                    :disabled="busy.has(item.id)"
                    @click.stop="cancel(item)"
                  >
                    {{ $t(isDeliveryWarning(item) ? 'outbox.dismiss' : 'common.discard') }}
                  </AppButton>
                  <span v-if="item.retry_count > 0" class="row__attempts">
                    {{ outboxAttemptsLabel(item.retry_count) }}
                  </span>
                </div>
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
            <p class="detail__status" :class="`detail__status--${markerTone(selectedItem)}`">
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
  gap: var(--space-2xs);
  list-style: none;
  margin: 0;
  padding: 0;
}

/*
 * The inbox row, not a card: avatar, who it goes to and when, subject, then
 * a meta line. Failures announce themselves through the badge, the reason and
 * the buttons, not through a red box around the row.
 */
.row {
  --row-surface: var(--bg-list);
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: start;
  gap: var(--space-md);
  padding: var(--space-2-5) var(--space-md);
  border-radius: var(--radius-md);
  background: var(--row-surface);
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    opacity var(--duration-fast) var(--ease-out);
}

.row:hover {
  --row-surface: var(--bg-hover);
}

/* The open entry is the sheet with the petrol edge, as in the inbox and in
   search — whatever its state. */
.row--active,
.row--active:hover {
  --row-surface: var(--bg-surface);
  box-shadow: var(--selection-edge), var(--shadow-sm);
}

.row--busy {
  opacity: 0.55;
  pointer-events: none;
}

.row__lead {
  position: relative;
  width: 36px;
  height: 36px;
}

.row__action-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  background: var(--bg-raised);
  color: var(--text-secondary);
}

/* The queue state, pinned to the avatar where the inbox pins unread. The
   ring is the row's own surface, so the badge sits cut out of the avatar. */
.row__marker {
  position: absolute;
  right: calc(var(--space-1-5) * -1);
  bottom: calc(var(--space-xs) * -1);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: var(--radius-full);
  box-shadow: 0 0 0 var(--space-2xs) var(--row-surface);
  transition: box-shadow var(--duration-fast) var(--ease-out);
}

.row__marker svg {
  width: 10px;
  height: 10px;
}

.row__marker--danger {
  background: var(--accent-red);
  color: var(--text-on-accent);
}

.row__marker--warning {
  background: var(--accent-amber);
  color: var(--text-on-amber);
}

.row__marker--accent {
  background: var(--accent-primary);
  color: var(--text-on-accent);
}

.row__marker--neutral {
  background: var(--bg-raised);
  color: var(--text-secondary);
}

.row__body {
  display: flex;
  flex-direction: column;
  gap: var(--space-2xs);
  min-width: 0;
}

.row__top {
  position: relative;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: baseline;
  gap: var(--space-sm);
  min-height: var(--space-5);
}

.row__who {
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-medium);
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* When the next step happens is information, not a hint. */
.row__time {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  transition: opacity var(--duration-fast) var(--ease-out);
}

.row__time--overdue {
  color: var(--accent-amber-soft);
  font-weight: var(--weight-medium);
}

.row__quick {
  position: absolute;
  top: 50%;
  right: calc(var(--space-xs) * -1);
  display: flex;
  gap: var(--space-2xs);
  opacity: 0;
  pointer-events: none;
  transform: translateY(-50%);
  transition: opacity var(--duration-fast) var(--ease-out);
}

.row:hover .row__quick,
.row:focus-within .row__quick {
  opacity: 1;
  pointer-events: auto;
}

.row:hover .row__top--quick .row__time,
.row:focus-within .row__top--quick .row__time {
  opacity: 0;
}

.row__subject {
  overflow: hidden;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__meta {
  margin: 0;
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Why it is stuck, in plain words and the error ink — two lines at most;
   the detail pane has the rest. */
.row__reason {
  display: -webkit-box;
  margin: 0;
  overflow: hidden;
  color: var(--accent-red-soft);
  font-size: var(--font-size-xs);
  line-height: 1.45;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.row__fix {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs);
  margin-top: var(--space-1-5);
}

.row__attempts {
  margin-left: var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
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
  color: var(--accent-red-soft);
}

.detail__status--accent {
  color: var(--text-accent);
}

.detail__status--warning {
  color: var(--accent-amber-soft);
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
