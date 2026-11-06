<script setup lang="ts">
/**
 * The cleanup assistant: who filled the Inbox over the last 90 days, and one
 * key per way of dealing with them — unsubscribe, archive or delete all, keep
 * only the latest, or a rule for from now on.
 *
 * Every action is the one the reader and the sender menu offer
 * (`useThreadActions().sweepSender`, the sender sweep dialog, the message's
 * unsubscribe), so this view adds a list, not another way to file mail.
 *
 * The view owns its keys. It is not a mail route: the mail keys there act on
 * the active thread, and behind this page that is a thread nobody is looking
 * at. j/k (or the user's own binding), the arrows and Enter move and open;
 * the action keys are listed in the footer.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import IconButton from '@/components/ui/IconButton.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import Avatar from '@/components/common/Avatar.vue';
import { useThreadActions, type SenderSweepAction } from '@/composables/useThreadActions';
import { useSenderSweep } from '@/composables/useSenderSweep';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import { useShortcutHints } from '@/composables/useShortcutHints';
import { pageKeyBlocked } from '@/composables/pageKeys';
import { useAccountsStore } from '@/stores/accounts';
import { useMessagesStore } from '@/stores/messages';
import { useSettingsStore } from '@/stores/settings';
import { useSweepRulesStore } from '@/stores/sweepRules';
import { useUiStore } from '@/stores/ui';
import type { CleanupSender as BackendCleanupSender } from '@/stores/sweepRules';
import { effectiveBindings, matchesKeybinding } from '@/config/keybindings';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { formatListTime } from '@/utils/listTime';
import { describeSweepRule } from '@/utils/sweepRules';

const WINDOW_DAYS = 90;

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const actions = useThreadActions();
const sweep = useSenderSweep();
const toast = useToast();
const confirm = useConfirm();
const { hint } = useShortcutHints();
const accounts = useAccountsStore();
const messages = useMessagesStore();
const settings = useSettingsStore();
const rules = useSweepRulesStore();
const ui = useUiStore();

const senders = ref<BackendCleanupSender[]>([]);
const loading = ref(false);
const failed = ref(false);
const busyKey = ref('');
const unsubscribed = ref(new Set<string>());
const focusIndex = ref(0);
const listRef = ref<HTMLElement | null>(null);

const accountId = computed(() => typeof route.query.account === 'string' ? route.query.account : '');
const scopedAccount = computed(() => accounts.accounts.find((account) => account.id === accountId.value));
const multipleAccounts = computed(() => accounts.accounts.filter((account) => account.kind === 'email').length > 1);

function keyOf(sender: BackendCleanupSender): string {
  return `${sender.account_id}\u0000${sender.sender_email}`;
}

let request = 0;
async function load() {
  const id = ++request;
  loading.value = true;
  failed.value = false;
  try {
    const rows = await rules.listSenders({ days: WINDOW_DAYS, limit: 50, accountId: accountId.value });
    if (id !== request) return;
    senders.value = rows;
    focusIndex.value = Math.min(focusIndex.value, Math.max(0, rows.length - 1));
  } catch {
    if (id === request) failed.value = true;
  } finally {
    if (id === request) loading.value = false;
  }
}

watch(accountId, () => void load());
onMounted(() => {
  void load();
  void rules.load();
});
// A rule saved in the dialog runs once right away; the counts follow.
watch(() => sweep.state.request, (open, was) => {
  if (!open && was) window.setTimeout(() => void load(), 600);
});

function accountLabel(id: string): string {
  return accounts.accounts.find((account) => account.id === id)?.email ?? '';
}

function unreadShare(sender: BackendCleanupSender): number {
  return sender.message_count > 0 ? Math.round((sender.unread_count / sender.message_count) * 100) : 0;
}

function ruleSummary(sender: BackendCleanupSender): string {
  const rule = rules.forSender(sender.account_id, sender.sender_email);
  return rule ? describeSweepRule(rule, t) : '';
}

function focused(): BackendCleanupSender | undefined {
  return senders.value[focusIndex.value];
}

function focusRow(index: number) {
  const count = senders.value.length;
  if (count === 0) return;
  focusIndex.value = Math.min(count - 1, Math.max(0, index));
  void nextTick(() => listRef.value
    ?.querySelectorAll<HTMLElement>('.cleanup-row__main')[focusIndex.value]
    ?.focus());
}

function openDialog(sender: BackendCleanupSender, focus: 'now' | 'rule' = 'now') {
  sweep.open({
    accountId: sender.account_id,
    email: sender.sender_email,
    name: sender.sender_name ?? undefined,
    focus,
  });
}

async function sweepRow(sender: BackendCleanupSender, action: SenderSweepAction, keepLatest: boolean) {
  const key = keyOf(sender);
  if (busyKey.value) return;
  busyKey.value = key;
  try {
    // Archive is one key and has Undo; moving a sender's mail to Trash asks
    // first, with the number it is about to move.
    if (action === 'trash' && !await confirmTrash(sender, keepLatest)) return;
    await actions.sweepSender({ accountId: sender.account_id, email: sender.sender_email, action, keepLatest });
  } finally {
    busyKey.value = '';
  }
  await load();
}

async function unsubscribe(sender: BackendCleanupSender) {
  const messageId = sender.unsubscribe_message_id;
  if (!messageId || busyKey.value) return;
  const key = keyOf(sender);
  busyKey.value = key;
  try {
    const result = await messages.unsubscribe(messageId);
    unsubscribed.value = new Set([...unsubscribed.value, key]);
    toast.success(t('mail.unsubscribe.success', { method: result.method }), {
      action: {
        label: t('sweep.now.archiveAll'),
        handler: () => sweepRow(sender, 'archive', false),
      },
    });
  } catch (error) {
    toast.error(t('mail.unsubscribe.failed', { error: backendErrorLabel(error) }));
  } finally {
    busyKey.value = '';
  }
}

async function confirmTrash(sender: BackendCleanupSender, keepLatest: boolean): Promise<boolean> {
  const summary = await rules.previewSummary({ accountId: sender.account_id, email: sender.sender_email, keepLatest });
  if (summary.count === 0) return true;
  return confirm.ask({
    title: t('cleanup.trashConfirm.title', { count: summary.count, sender: sender.sender_name || sender.sender_email }, summary.count),
    body: t('cleanup.trashConfirm.body'),
    confirmLabel: t('cleanup.actions.trashAll'),
    tone: 'danger',
  });
}

function canUnsubscribe(sender: BackendCleanupSender): boolean {
  return Boolean(sender.unsubscribe_message_id) && !unsubscribed.value.has(keyOf(sender));
}

function showAllAccounts() {
  void router.replace({ name: 'cleanup' });
}

/* ── Keyboard ─────────────────────────────────────────────────────────── */

function bound(event: KeyboardEvent, action: string): boolean {
  return effectiveBindings(action, settings)
    .some((binding) => matchesKeybinding(event, binding));
}

function onKeydown(event: KeyboardEvent) {
  // Includes an armed `g` sequence: `g d`/`g e` navigate, they do not archive.
  if (pageKeyBlocked(event)) return;
  const sender = focused();
  let handled = true;
  if (event.key === 'ArrowDown' || bound(event, 'nextThread')) focusRow(focusIndex.value + 1);
  else if (event.key === 'ArrowUp' || bound(event, 'prevThread')) focusRow(focusIndex.value - 1);
  else if (!sender) handled = false;
  else if (bound(event, 'openItem') || (event.key === 'Enter' && !isButton(event.target))) openDialog(sender);
  else if (bound(event, 'markDone') || bound(event, 'archive')) void sweepRow(sender, 'archive', false);
  else if (bound(event, 'trash')) void sweepRow(sender, 'trash', false);
  else if (bound(event, 'cleanupKeepLatest')) void sweepRow(sender, 'archive', true);
  else if (bound(event, 'cleanupUnsubscribe')) void unsubscribe(sender);
  else if (bound(event, 'cleanupRule')) openDialog(sender, 'rule');
  else handled = false;
  if (handled) {
    event.preventDefault();
    event.stopPropagation();
  }
}

/** Enter on a focused button presses it; only the row itself opens the dialog. */
function isButton(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return Boolean(el?.closest?.('button')) && !el?.classList.contains('cleanup-row__main');
}

// Capture: the global handler must not see a key this page has answered.
onMounted(() => window.addEventListener('keydown', onKeydown, true));
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown, true));

/** The keys as the user has them bound; a switched-off key leaves its row out. */
const legend = computed(() => [
  { keys: [...(hint('nextThread')?.keys ?? []), ...(hint('prevThread')?.keys ?? [])], label: t('cleanup.keys.move') },
  { keys: hint('archive')?.keys ?? hint('markDone')?.keys ?? [], label: t('sweep.now.archiveAll') },
  { keys: hint('trash')?.keys ?? [], label: t('cleanup.actions.trashAll') },
  { keys: hint('cleanupKeepLatest')?.keys ?? [], label: t('cleanup.actions.keepLatest') },
  { keys: hint('cleanupUnsubscribe')?.keys ?? [], label: t('mail.unsubscribe.action') },
  { keys: hint('cleanupRule')?.keys ?? [], label: t('cleanup.actions.rule') },
].filter((item) => item.keys.length > 0));
</script>

<template>
  <section class="cleanup-view">
    <PaneHeader
      :title="$t('cleanup.title')"
      :subtitle="scopedAccount ? $t('cleanup.subtitleAccount', { account: scopedAccount.email, days: WINDOW_DAYS }) : $t('cleanup.subtitle', { days: WINDOW_DAYS })"
      :count="senders.length || undefined"
    >
      <template #leading>
        <IconButton :title="$t('newSenders.backToInbox')" @click="router.push({ name: 'inbox' })">
          <AppIcon><path d="M10 3.5 5.5 8l4.5 4.5" /></AppIcon>
        </IconButton>
      </template>
      <template v-if="scopedAccount && multipleAccounts" #actions>
        <AppButton variant="ghost" size="sm" @click="showAllAccounts">{{ $t('cleanup.allAccounts') }}</AppButton>
      </template>
    </PaneHeader>

    <div class="cleanup-view__body">
      <CollectionLoadState
        :loading="loading"
        :failed="failed"
        :has-content="senders.length > 0"
        :error-label="$t('cleanup.loadFailed')"
        @retry="load"
      />
      <EmptyState
        v-if="!loading && !failed && senders.length === 0"
        :title="$t('cleanup.empty.title')"
        :description="$t('cleanup.empty.description', { days: WINDOW_DAYS })"
      />

      <ol v-if="senders.length" ref="listRef" class="cleanup-list" :aria-label="$t('cleanup.title')">
        <li
          v-for="(sender, index) in senders"
          :key="keyOf(sender)"
          class="cleanup-row"
          :class="{ 'cleanup-row--focused': index === focusIndex }"
          :aria-busy="busyKey === keyOf(sender) || undefined"
        >
          <ListRowButton class="cleanup-row__main" @focus="focusIndex = index" @click="focusIndex = index; openDialog(sender)">
            <Avatar :name="sender.sender_name || sender.sender_email" :email="sender.sender_email" :size="36" />
            <span class="cleanup-row__copy">
              <strong class="cleanup-row__name">{{ sender.sender_name || sender.sender_email }}</strong>
              <span class="cleanup-row__email">
                {{ sender.sender_email }}<template v-if="multipleAccounts && !scopedAccount"> · {{ accountLabel(sender.account_id) }}</template>
              </span>
              <span class="cleanup-row__meta">
                <span>{{ $t('cleanup.messages', { count: sender.message_count }, sender.message_count) }}</span>
                <span>{{ $t('cleanup.threads', { count: sender.thread_count }, sender.thread_count) }}</span>
                <span>{{ $t('cleanup.unreadShare', { percent: unreadShare(sender) }) }}</span>
                <span>{{ $t('cleanup.newest', { date: formatListTime(sender.newest_date) }) }}</span>
                <span v-if="sender.unsubscribe_message_id" class="cleanup-row__tag">{{ $t('cleanup.canUnsubscribe') }}</span>
                <span v-if="ruleSummary(sender)" class="cleanup-row__tag cleanup-row__tag--rule">{{ ruleSummary(sender) }}</span>
              </span>
            </span>
          </ListRowButton>
          <div class="cleanup-row__actions">
            <AppButton
              v-if="sender.unsubscribe_message_id"
              size="sm"
              variant="secondary"
              :disabled="Boolean(busyKey) || !canUnsubscribe(sender)"
              @click="unsubscribe(sender)"
            >
              {{ canUnsubscribe(sender) ? $t('mail.unsubscribe.action') : $t('cleanup.unsubscribed') }}
            </AppButton>
            <AppButton size="sm" variant="secondary" :disabled="Boolean(busyKey)" @click="sweepRow(sender, 'archive', false)">
              {{ $t('cleanup.actions.archiveAll') }}
            </AppButton>
            <AppButton size="sm" variant="secondary" :disabled="Boolean(busyKey)" @click="sweepRow(sender, 'trash', false)">
              {{ $t('cleanup.actions.trashAll') }}
            </AppButton>
            <AppButton size="sm" variant="ghost" :disabled="Boolean(busyKey) || sender.thread_count < 2" @click="sweepRow(sender, 'archive', true)">
              {{ $t('cleanup.actions.keepLatest') }}
            </AppButton>
            <AppButton size="sm" variant="ghost" :disabled="Boolean(busyKey)" @click="openDialog(sender, 'rule')">
              {{ $t('cleanup.actions.rule') }}
            </AppButton>
          </div>
        </li>
      </ol>

      <footer v-if="senders.length && !ui.isMobile" class="cleanup-legend" :aria-label="$t('cleanup.keys.title')">
        <span v-for="item in legend" :key="item.label" class="cleanup-legend__item">
          <KeyCap v-for="(key, index) in item.keys" :key="index">{{ key }}</KeyCap>
          <span>{{ item.label }}</span>
        </span>
      </footer>
    </div>
  </section>
</template>

<style scoped>
.cleanup-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: var(--bg-base);
  color: var(--text-primary);
}

.cleanup-view__body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-md) var(--space-lg) var(--space-xl);
}

.cleanup-list {
  display: grid;
  gap: var(--space-xs);
  max-width: 820px;
  margin: 0 auto;
  padding: 0;
  list-style: none;
}

.cleanup-row {
  display: grid;
  gap: var(--space-xs);
  padding: var(--space-xs);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  min-width: 0;
}

.cleanup-row--focused {
  border-color: var(--accent-primary);
}

.cleanup-row[aria-busy='true'] {
  opacity: 0.6;
}

.cleanup-row__main {
  display: grid;
  grid-template-columns: 36px minmax(0, 1fr);
  align-items: center;
  gap: var(--space-sm);
  padding: var(--space-xs);
}

.cleanup-row__copy {
  display: grid;
  gap: var(--space-2xs);
  min-width: 0;
}

.cleanup-row__name,
.cleanup-row__email {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cleanup-row__email {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.cleanup-row__meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2xs) var(--space-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.cleanup-row__tag {
  color: var(--accent-primary);
}

.cleanup-row__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs);
  padding: 0 var(--space-xs) var(--space-xs);
}

.cleanup-legend {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--space-xs) var(--space-md);
  max-width: 820px;
  margin: var(--space-lg) auto 0;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.cleanup-legend__item {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2xs);
}

/* Phone: the actions get the row's full width, and touch-sized buttons. */
@media (max-width: 620px) {
  .cleanup-view__body {
    padding: var(--space-sm) var(--space-md) var(--space-xl);
  }

  .cleanup-row__actions > * {
    flex: 1 1 calc(50% - var(--space-xs));
    min-height: 40px;
  }
}
</style>
