<script setup lang="ts">
/**
 * The last step before a mail merge goes out: step through the recipients
 * and read each personalised copy, see which rows are skipped and why, pick
 * the pace, and start. Rendering is the backend's, the same function the job
 * uses, so the preview is what each person receives.
 *
 * Starting is the composer's: it owns the draft that the job replaces.
 */
import { computed, onMounted, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IsolatedEmailDocument from '@/components/mail/IsolatedEmailDocument.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import { useMailMergeStore } from '@/stores/mailMerge';
import type { BackendMergePreview, BackendMergeRowIssue, BackendMergeTemplate } from '@/types/mailMerge';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { mergeDurationMinutes, type MailMergeSource } from '@/utils/mailMerge';

const props = defineProps<{
  source: MailMergeSource;
  template: BackendMergeTemplate;
  accountId: string;
  /** The address the copies leave from. */
  fromAddress: string;
  encrypt: boolean;
  busy: boolean;
  /** Why the last start failed, already translated. */
  error?: string | null;
}>();

const emit = defineEmits<{
  /** `count` is what will go out: the rows the plan lets through. */
  start: [delaySeconds: number, count: number];
  'change-list': [];
  close: [];
}>();

const { t } = useI18n();
const mailMerge = useMailMergeStore();
const delayId = useId();

const preview = ref<BackendMergePreview | null>(null);
const loadError = ref('');
const index = ref(0);
const delay = ref<number | null>(null);
let request = 0;

async function load() {
  const current = ++request;
  try {
    const next = await mailMerge.preview({
      accountId: props.accountId,
      template: props.template,
      list: props.source,
      index: index.value,
    });
    if (current !== request) return;
    preview.value = next;
    loadError.value = '';
    if (delay.value === null && next.default_delay_seconds > 0) delay.value = next.default_delay_seconds;
  } catch (error) {
    if (current !== request) return;
    loadError.value = backendErrorLabel(error);
  }
}

onMounted(load);
watch(index, load);

const rows = computed(() => preview.value?.plan.rows ?? []);
const total = computed(() => props.source.rows.length);
const ready = computed(() => preview.value?.plan.ready ?? 0);
const limit = computed(() => preview.value?.max_recipients ?? 500);
const overLimit = computed(() => ready.value > limit.value);
const skipped = computed(() => rows.value.filter((row) => row.issue));
const currentRow = computed(() => rows.value[index.value] ?? null);
const delaySeconds = computed(() => {
  const value = Math.round(Number(delay.value));
  return Number.isFinite(value) ? value : 0;
});
const delayValid = computed(() => delaySeconds.value >= 1 && delaySeconds.value <= 3600);
const minutes = computed(() => mergeDurationMinutes(ready.value, delaySeconds.value));

const canStart = computed(() => !props.busy && preview.value !== null && ready.value > 0 && !overLimit.value && delayValid.value);

function issueLabel(issue: BackendMergeRowIssue): string {
  switch (issue.kind) {
    case 'invalid_email': return t('mailMerge.review.issue.invalidEmail');
    case 'duplicate': return t('mailMerge.review.issue.duplicate');
    case 'missing_values': return t('mailMerge.review.issue.missing', { fields: issue.fields.join(', ') });
  }
}

function step(by: number) {
  const next = index.value + by;
  if (next >= 0 && next < total.value) index.value = next;
}

function show(rowIndex: number) {
  index.value = rowIndex;
}

function onKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement;
  if (target.closest('input, select, textarea')) return;
  if (event.key === 'ArrowRight' || event.key === 'j') {
    event.preventDefault();
    step(1);
  } else if (event.key === 'ArrowLeft' || event.key === 'k') {
    event.preventDefault();
    step(-1);
  }
}

function start() {
  if (canStart.value) emit('start', delaySeconds.value, ready.value);
}

function close() {
  if (!props.busy) emit('close');
}

useDismissableOnTop(() => true, close);
</script>

<template>
  <Teleport to="body">
    <!-- On the shell: focus rests on its panel, outside this content. -->
    <ModalShell :label="$t('mailMerge.review.title')" width="720px" height="auto" :dismiss-on-backdrop="false" @dismiss="close" @keydown="onKeydown">
      <div class="merge-review" data-testid="mail-merge-review">
        <header class="merge-review__head">
          <h2 class="merge-review__title">{{ $t('mailMerge.review.title') }}</h2>
          <p class="merge-review__lead" role="status">
            {{ $t('mailMerge.review.summary', { ready, total, source: source.label }, ready) }}
          </p>
        </header>

        <AppBanner v-if="loadError" tone="danger" :title="loadError" />
        <AppBanner v-if="error" tone="danger" :title="error" />
        <AppBanner
          v-if="overLimit"
          tone="danger"
          :title="$t('mailMerge.review.overLimit', { count: ready, limit })"
          :detail="$t('mailMerge.review.overLimitDetail', { limit })"
        />
        <AppBanner
          v-if="preview && preview.plan.unknown_fields.length > 0"
          tone="warning"
          :title="$t('mailMerge.review.unknownFields', { fields: preview.plan.unknown_fields.map((name) => `{{${name}}}`).join(', ') })"
          :detail="$t('mailMerge.review.unknownFieldsDetail', { fields: source.fields.join(', ') })"
        />
        <AppBanner
          v-if="encrypt"
          tone="info"
          :title="$t('mailMerge.review.encrypted')"
        />

        <section class="merge-review__preview" :aria-label="$t('mailMerge.review.previewLabel')">
          <div class="merge-review__stepper">
            <IconButton :title="$t('mailMerge.review.previous')" :aria-label="$t('mailMerge.review.previous')" :disabled="index === 0" @click="step(-1)">
              <AppIcon><path d="m10 3-5 5 5 5" /></AppIcon>
            </IconButton>
            <span class="merge-review__position" aria-live="polite" data-testid="mail-merge-position">
              {{ $t('mailMerge.review.position', { index: index + 1, total }) }}
            </span>
            <IconButton :title="$t('mailMerge.review.next')" :aria-label="$t('mailMerge.review.next')" :disabled="index >= total - 1" @click="step(1)">
              <AppIcon><path d="m6 3 5 5-5 5" /></AppIcon>
            </IconButton>
          </div>
          <dl class="merge-review__meta">
            <dt>{{ $t('common.to') }}</dt>
            <dd data-testid="mail-merge-preview-to">{{ currentRow?.email ?? '' }}</dd>
            <template v-if="preview?.rendered">
              <dt>{{ $t('common.subject') }}</dt>
              <dd data-testid="mail-merge-preview-subject">{{ preview.rendered.subject }}</dd>
            </template>
          </dl>
          <AppBanner
            v-if="currentRow?.issue"
            tone="warning"
            :title="$t('mailMerge.review.rowSkipped')"
            :detail="issueLabel(currentRow.issue)"
          />
          <div v-else-if="preview?.rendered" class="merge-review__body" data-testid="mail-merge-preview-body">
            <IsolatedEmailDocument
              v-if="preview.rendered.body_html"
              :html="preview.rendered.body_html"
              :allow-remote-images="false"
            />
            <pre v-else class="merge-review__text">{{ preview.rendered.body }}</pre>
          </div>
        </section>

        <details v-if="skipped.length > 0" class="merge-review__skipped" data-testid="mail-merge-skipped">
          <summary>{{ $t('mailMerge.review.skipped', { count: skipped.length }, skipped.length) }}</summary>
          <ul>
            <li v-for="row in skipped.slice(0, 200)" :key="row.index">
              <AppButton size="sm" variant="link" @click="show(row.index)">
                {{ row.line ? $t('mailMerge.review.line', { line: row.line }) : (row.email || $t('mailMerge.review.noAddress')) }}
              </AppButton>
              <span class="merge-review__skip-email">{{ row.line ? row.email : '' }}</span>
              <span class="merge-review__skip-reason">{{ row.issue ? issueLabel(row.issue) : '' }}</span>
            </li>
          </ul>
        </details>

        <section class="merge-review__pace">
          <label :for="delayId">{{ $t('mailMerge.review.delay') }}</label>
          <div class="merge-review__pace-field">
            <input
              :id="delayId"
              v-model.number="delay"
              type="number"
              inputmode="numeric"
              min="1"
              max="3600"
              step="1"
              data-testid="mail-merge-delay"
            />
            <span>{{ $t('mailMerge.review.seconds') }}</span>
          </div>
          <p class="merge-review__hint">
            {{ delayValid ? $t('mailMerge.review.duration', { minutes }, minutes) : $t('mailMerge.review.delayInvalid') }}
            {{ $t('mailMerge.review.delayHint') }}
          </p>
        </section>

        <footer class="merge-review__actions">
          <AppButton variant="ghost" :disabled="busy" @click="emit('change-list')">{{ $t('mailMerge.bar.change') }}</AppButton>
          <span class="merge-review__spacer" />
          <AppButton variant="ghost" :disabled="busy" @click="close">{{ $t('common.cancel') }}</AppButton>
          <AppButton variant="primary" :disabled="!canStart" data-testid="mail-merge-start" @click="start">
            {{ $t('mailMerge.review.start', { count: ready, address: fromAddress }, ready) }}
          </AppButton>
        </footer>
      </div>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.merge-review {
  display: grid;
  gap: var(--space-lg);
  padding: var(--space-xl);
  min-width: 0;
}

.merge-review__head {
  display: grid;
  gap: var(--space-2xs);
}

.merge-review__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.merge-review__lead,
.merge-review__hint {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.merge-review__preview {
  display: grid;
  gap: var(--space-sm);
  padding: var(--space-md);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  min-width: 0;
}

.merge-review__stepper {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.merge-review__position {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-variant-numeric: tabular-nums;
}

.merge-review__meta {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: var(--space-2xs) var(--space-md);
  margin: 0;
  font-size: var(--font-size-sm);
}

.merge-review__meta dt {
  color: var(--text-tertiary);
}

.merge-review__meta dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--text-primary);
}

.merge-review__body {
  max-height: 320px;
  overflow: auto;
  min-width: 0;
}

.merge-review__text {
  margin: 0;
  color: var(--text-primary);
  font-family: var(--font-sans);
  font-size: var(--font-size-md);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.merge-review__skipped summary {
  cursor: pointer;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.merge-review__skipped ul {
  display: grid;
  gap: var(--space-2xs);
  max-height: 200px;
  margin: var(--space-sm) 0 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
  font-size: var(--font-size-sm);
}

.merge-review__skipped li {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-xs) var(--space-sm);
  min-width: 0;
}

.merge-review__skip-email {
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.merge-review__skip-reason {
  color: var(--text-tertiary);
}

.merge-review__pace {
  display: grid;
  gap: var(--space-2xs);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.merge-review__pace-field {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.merge-review__pace-field input {
  width: 6rem;
  min-height: var(--touch-target);
  padding: 0 var(--space-sm);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
}

.merge-review__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-sm);
}

.merge-review__spacer {
  flex: 1 1 auto;
}
</style>
