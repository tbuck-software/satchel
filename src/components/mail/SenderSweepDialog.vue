<script setup lang="ts">
/**
 * "Clean up mail from this sender": the one-shot sweeps (archive or delete
 * all, or all but the newest) and the recurring rule (from now on, archive or
 * delete mail older than N days, or move it to a folder).
 *
 * Mounted once in `App.vue`; `useSenderSweep().open()` fills it. One-shot
 * sweeps run through `useThreadActions().sweepSender`, so they leave with the
 * same optimistic update and Undo as a selection. The rule is saved through
 * the sweep rules store; the backend runs it once right away and then on its
 * schedule.
 */
import { computed, nextTick, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import { useSenderSweep } from '@/composables/useSenderSweep';
import { useThreadActions, type SenderSweepAction } from '@/composables/useThreadActions';
import { usePrompt } from '@/composables/usePrompt';
import { useConfirm } from '@/composables/useConfirm';
import { useToast } from '@/composables/useToast';
import { useMailboxesStore } from '@/stores/mailboxes';
import { useSweepRulesStore, type SweepRule, type SweepSummary } from '@/stores/sweepRules';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

const { t } = useI18n();
const sweep = useSenderSweep();
const actions = useThreadActions();
const prompt = usePrompt();
const confirm = useConfirm();
const toast = useToast();
const mailboxes = useMailboxesStore();
const rules = useSweepRulesStore();

const request = computed(() => sweep.state.request);
/** What a one-shot sweep of everything would take, and what it leaves. */
const nowSummary = ref<SweepSummary | null>(null);
const inboxCount = computed(() => nowSummary.value?.count ?? null);
const busy = ref(false);

type RuleAction = 'archive' | 'trash' | 'move';
const PRESET_DAYS = [3, 10, 30] as const;
const ruleAction = ref<RuleAction>('archive');
const ruleDays = ref<number>(10);
const ruleKeepLatest = ref(false);
const ruleFolder = ref('');
const ruleMatches = ref<number | null>(null);
const ruleSection = ref<HTMLElement | null>(null);

const existingRule = computed(() =>
  request.value ? rules.forSender(request.value.accountId, request.value.email) : undefined);

const senderLabel = computed(() => request.value?.name?.trim() || request.value?.email || '');

/** Folders of the sender's account that mail can be filed into. */
const folders = computed(() => {
  const accountId = request.value?.accountId;
  return mailboxes.mailboxes
    .filter((mailbox) => mailbox.account_id === accountId)
    .filter((mailbox) => !['inbox', 'trash', 'spam', 'drafts', 'sent', 'outbox'].includes(mailbox.mailbox_type))
    .map((mailbox) => ({ id: mailbox.id, label: mailbox.name }))
    .sort((left, right) => left.label.localeCompare(right.label));
});

const actionOptions = computed(() => [
  { value: 'archive', label: t('sweep.rule.archive') },
  { value: 'trash', label: t('sweep.rule.trash') },
  { value: 'move', label: t('sweep.rule.move'), disabled: folders.value.length === 0 },
]);

const dayOptions = computed(() => [
  ...PRESET_DAYS.map((days) => ({ value: String(days), label: t('sweep.rule.days', { count: days }, days) })),
  {
    value: 'custom',
    label: (PRESET_DAYS as readonly number[]).includes(ruleDays.value)
      ? t('sweep.rule.customDays')
      : t('sweep.rule.days', { count: ruleDays.value }, ruleDays.value),
  },
]);
const dayValue = computed(() =>
  (PRESET_DAYS as readonly number[]).includes(ruleDays.value) ? String(ruleDays.value) : 'custom');

watch(
  request,
  (next) => {
    nowSummary.value = null;
    ruleMatches.value = null;
    if (!next) return;
    const cached = rules.forSender(next.accountId, next.email);
    applyRule(cached);
    // The list may not be loaded yet; a rule that turns up fills the form.
    void rules.load().then(() => {
      const loaded = rules.forSender(next.accountId, next.email);
      if (request.value === next && loaded && loaded !== cached) applyRule(loaded);
    });
    void countInbox(next.accountId, next.email);
    if (next.focus === 'rule') {
      void nextTick(() => ruleSection.value?.querySelector<HTMLElement>('button:not(:disabled)')?.focus());
    }
  },
  { immediate: true },
);

watch([request, ruleAction, ruleDays, ruleKeepLatest], () => void countRuleMatches());

function applyRule(rule: SweepRule | undefined) {
  ruleAction.value = (rule?.action as RuleAction | undefined) ?? 'archive';
  ruleDays.value = rule?.older_than_days || 10;
  ruleKeepLatest.value = rule?.keep_latest ?? false;
  ruleFolder.value = rule?.target_folder_id ?? '';
}

async function countInbox(accountId: string, email: string) {
  try {
    const summary = await rules.previewSummary({ accountId, email });
    if (request.value?.email === email) nowSummary.value = summary;
  } catch {
    nowSummary.value = null;
  }
}

/** The sender's threads a sweep does not touch, in one sentence each. */
const skippedNotes = computed(() => {
  const summary = nowSummary.value;
  if (!summary) return [];
  return [
    summary.skipped_shared > 0
      ? t('sweep.skipped.shared', { count: summary.skipped_shared }, summary.skipped_shared) : '',
    summary.skipped_starred > 0
      ? t('sweep.skipped.starred', { count: summary.skipped_starred }, summary.skipped_starred) : '',
  ].filter(Boolean);
});

/** A move rule whose folder disappeared in a sync waits for a new one. */
const ruleNeedsFolder = computed(() =>
  existingRule.value?.action === 'move' && !existingRule.value.target_folder_id);

/** How much the rule would file if it ran now — it does, once, on save. */
async function countRuleMatches() {
  const current = request.value;
  if (!current) return;
  try {
    const summary = await rules.previewSummary({
      accountId: current.accountId,
      email: current.email,
      keepLatest: ruleKeepLatest.value,
      olderThanDays: ruleAction.value === 'move' ? null : ruleDays.value,
    });
    if (request.value === current) ruleMatches.value = summary.count;
  } catch {
    ruleMatches.value = null;
  }
}

function close() {
  if (!busy.value) sweep.close();
}

useDismissableOnTop(() => request.value !== null, close);

async function runNow(action: SenderSweepAction, keepLatest: boolean) {
  const current = request.value;
  if (!current || busy.value) return;
  busy.value = true;
  // The dialog leaves first: the sweep's receipt is a toast over the list.
  sweep.close();
  try {
    await actions.sweepSender({ accountId: current.accountId, email: current.email, action, keepLatest });
  } finally {
    busy.value = false;
  }
}

const nowEntries = computed(() => {
  const all = inboxCount.value;
  const rest = all === null ? null : Math.max(0, all - 1);
  return [
    { key: '1', label: t('sweep.now.archiveAll'), count: all, action: 'archive' as const, keepLatest: false },
    { key: '2', label: t('sweep.now.trashAll'), count: all, action: 'trash' as const, keepLatest: false, danger: true },
    { key: '3', label: t('sweep.now.keepLatestArchive'), count: rest, action: 'archive' as const, keepLatest: true },
    { key: '4', label: t('sweep.now.keepLatestTrash'), count: rest, action: 'trash' as const, keepLatest: true, danger: true },
  ];
});

async function chooseDays(value: string) {
  if (value !== 'custom') {
    ruleDays.value = Number(value);
    return;
  }
  const answer = await prompt.ask({
    title: t('sweep.rule.customTitle'),
    label: t('sweep.rule.customLabel'),
    initialValue: String(ruleDays.value),
    confirmLabel: t('common.save'),
  });
  const days = Number.parseInt(answer ?? '', 10);
  if (Number.isFinite(days) && days >= 1 && days <= 3650) ruleDays.value = days;
  else if (answer !== null) toast.error(t('sweep.rule.customInvalid'));
}

const canSaveRule = computed(() => ruleAction.value !== 'move' || Boolean(ruleFolder.value));

/**
 * A trash rule runs once on save and has no Undo: it says how much it moves
 * right away and asks.
 */
async function confirmTrashRule(email: string): Promise<boolean> {
  if (ruleAction.value !== 'trash') return true;
  // Asked again here: the preview may be a toggle behind, or not back yet.
  const current = request.value;
  if (!current) return false;
  let count: number;
  try {
    count = (await rules.previewSummary({
      accountId: current.accountId,
      email: current.email,
      keepLatest: ruleKeepLatest.value,
      olderThanDays: ruleDays.value,
    })).count;
  } catch (error) {
    toast.error(t('sweep.rule.saveFailed', { error: backendErrorLabel(error) }));
    return false;
  }
  return confirm.ask({
    title: t('sweep.rule.trashConfirmTitle', { count }, count),
    body: t('sweep.rule.trashConfirmBody', { sender: email, count: ruleDays.value }, ruleDays.value),
    confirmLabel: t('sweep.rule.save'),
    tone: 'danger',
  });
}

async function saveRule() {
  const current = request.value;
  if (!current || !canSaveRule.value || busy.value) return;
  busy.value = true;
  if (!await confirmTrashRule(current.email)) {
    busy.value = false;
    return;
  }
  try {
    await rules.save({
      account_id: current.accountId,
      sender_email: current.email,
      action: ruleAction.value,
      older_than_days: ruleAction.value === 'move' ? null : ruleDays.value,
      keep_latest: ruleKeepLatest.value,
      target_folder_id: ruleAction.value === 'move' ? ruleFolder.value : null,
    });
    toast.success(t('sweep.rule.saved', { sender: current.email }));
    busy.value = false;
    sweep.close();
  } catch (error) {
    toast.error(t('sweep.rule.saveFailed', { error: backendErrorLabel(error) }));
    busy.value = false;
  }
}

async function removeRule() {
  const rule = existingRule.value;
  if (!rule || busy.value) return;
  busy.value = true;
  try {
    await rules.remove(rule.id);
    toast.success(t('sweep.rule.removed', { sender: rule.sender_email }));
  } catch (error) {
    toast.error(t('sweep.rule.removeFailed', { error: backendErrorLabel(error) }));
  } finally {
    busy.value = false;
  }
}

/** 1–4 run the one-shot sweeps; the dialog's own fields keep their keys. */
function onKeydown(event: KeyboardEvent) {
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const target = event.target as HTMLElement | null;
  if (target?.closest('input, textarea, [contenteditable="true"]')) return;
  const entry = nowEntries.value.find((item) => item.key === event.key);
  if (!entry || !entry.count) return;
  event.preventDefault();
  event.stopPropagation();
  void runNow(entry.action, entry.keepLatest);
}
</script>

<template>
  <Teleport to="body">
    <ModalShell
      v-if="request"
      :label="$t('sweep.title', { sender: senderLabel })"
      width="480px"
      height="auto"
      @dismiss="close"
    >
      <div class="sweep" @keydown="onKeydown">
        <header class="sweep__head">
          <h2 class="sweep__title">{{ $t('sweep.title', { sender: senderLabel }) }}</h2>
          <p class="sweep__subtitle">
            <span class="sweep__email">{{ $t('sweep.from', { email: request.email }) }}</span>
            <span v-if="inboxCount !== null">{{ $t('sweep.inboxCount', { count: inboxCount }, inboxCount) }}</span>
          </p>
          <p v-for="note in skippedNotes" :key="note" class="sweep__hint">{{ note }}</p>
        </header>

        <section class="sweep__section" :aria-label="$t('sweep.now.heading')">
          <h3 class="sweep__heading">{{ $t('sweep.now.heading') }}</h3>
          <div class="sweep__menu">
            <MenuItem
              v-for="entry in nowEntries"
              :key="entry.key"
              :label="entry.label"
              :tone="entry.danger ? 'danger' : 'default'"
              :disabled="busy || !entry.count"
              :aria-keyshortcuts="entry.key"
              @select="runNow(entry.action, entry.keepLatest)"
            >
              <template #icon><KeyCap aria-hidden="true">{{ entry.key }}</KeyCap></template>
              <template v-if="entry.count !== null" #trailing>
                <span class="sweep__count">{{ entry.count }}</span>
              </template>
            </MenuItem>
          </div>
        </section>

        <section ref="ruleSection" class="sweep__section" :aria-label="$t('sweep.rule.heading')">
          <h3 class="sweep__heading">{{ $t('sweep.rule.heading') }}</h3>
          <p class="sweep__hint">{{ $t('sweep.rule.hint') }}</p>
          <SegmentedControl
            :options="actionOptions"
            :model-value="ruleAction"
            :aria-label="$t('sweep.rule.actionLabel')"
            @update:model-value="ruleAction = $event as RuleAction"
          />
          <template v-if="ruleAction !== 'move'">
            <span class="sweep__label">{{ $t('sweep.rule.olderThan') }}</span>
            <SegmentedControl
              :options="dayOptions"
              :model-value="dayValue"
              :aria-label="$t('sweep.rule.olderThan')"
              @update:model-value="chooseDays"
            />
          </template>
          <p v-if="ruleAction === 'move' && ruleNeedsFolder && !ruleFolder" class="sweep__hint sweep__hint--warn">
            {{ $t('sweep.rule.needsFolder') }}
          </p>
          <div v-if="ruleAction === 'move'" class="sweep__folders" role="radiogroup" :aria-label="$t('sweep.rule.folder')">
            <MenuItem
              v-for="folder in folders"
              :key="folder.id"
              :label="folder.label"
              selection="radio"
              :checked="ruleFolder === folder.id"
              @select="ruleFolder = folder.id"
            />
          </div>
          <label class="sweep__toggle">
            <span>{{ $t('sweep.rule.keepLatest') }}</span>
            <ToggleSwitch v-model="ruleKeepLatest" :label="$t('sweep.rule.keepLatest')" />
          </label>
          <p v-if="ruleMatches !== null" class="sweep__hint">
            {{ $t('sweep.rule.matchesNow', { count: ruleMatches }, ruleMatches) }}
          </p>
          <footer class="sweep__actions">
            <AppButton v-if="existingRule" variant="ghost" :disabled="busy" @click="removeRule">
              {{ $t('sweep.rule.remove') }}
            </AppButton>
            <AppButton variant="ghost" :disabled="busy" @click="close">{{ $t('common.cancel') }}</AppButton>
            <AppButton variant="primary" :disabled="busy || !canSaveRule" @click="saveRule">
              {{ existingRule ? $t('sweep.rule.update') : $t('sweep.rule.save') }}
            </AppButton>
          </footer>
        </section>
      </div>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.sweep {
  display: grid;
  gap: var(--space-lg);
  padding: var(--space-xl);
  min-width: 0;
}

.sweep__head {
  display: grid;
  gap: var(--space-2xs);
  min-width: 0;
}

.sweep__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  overflow-wrap: anywhere;
}

.sweep__subtitle {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs) var(--space-sm);
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.sweep__email {
  overflow-wrap: anywhere;
}

.sweep__section {
  display: grid;
  gap: var(--space-sm);
  min-width: 0;
}

.sweep__heading {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

.sweep__menu,
.sweep__folders {
  display: grid;
  gap: var(--space-2xs);
}

.sweep__folders {
  max-height: 220px;
  overflow-y: auto;
}

.sweep__count {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.sweep__hint {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.sweep__hint--warn {
  color: var(--accent-amber);
}

.sweep__label {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.sweep__toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  font-size: var(--font-size-sm);
}

.sweep__actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
