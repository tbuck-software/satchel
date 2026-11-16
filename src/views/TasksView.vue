<script setup lang="ts">
/**
 * Tasks: one list or all of them, grouped by when they are due.
 *
 * Quick add at the top understands a trailing date ("Call Anna tomorrow
 * 15:00") through the reminder parser the snooze picker uses. The view owns
 * its keys, as the cleanup view does: it is not a mail route, so the mail
 * keys would act on a thread nobody is looking at. The keys are the mail
 * keys a person already knows — j/k (or their own binding) and the arrows
 * move, x or Done completes, Enter or l opens the editor, n adds, # deletes —
 * and the footer lists them as bound.
 *
 * Lists: the desktop sidebar shows them (`TasksSidebar`); a phone, which has
 * no sidebar on screen, gets them as a row of pills here.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import FilterPill from '@/components/ui/FilterPill.vue';
import IconButton from '@/components/ui/IconButton.vue';
import KeyCap from '@/components/ui/KeyCap.vue';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import { useConfirm } from '@/composables/useConfirm';
import { usePrompt } from '@/composables/usePrompt';
import { useShortcutHints } from '@/composables/useShortcutHints';
import { pageKeyBlocked } from '@/composables/pageKeys';
import { useTaskEditor } from '@/composables/useTaskEditor';
import { useToast } from '@/composables/useToast';
import { effectiveBindings, matchesKeybinding } from '@/config/keybindings';
import { useSettingsStore } from '@/stores/settings';
import { ALL_LISTS, useTasksStore, type Task } from '@/stores/tasks';
import { useUiStore } from '@/stores/ui';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { dueState, formatDue, groupTasks, parseQuickAdd, priorityLevel, type TaskGroup } from '@/utils/tasks';
import { useTaskActions } from '@/components/tasks/taskActions';
import { followTaskSyncs } from '@/composables/taskEvents';
import { listLabel, listName } from '@/components/tasks/taskLabels';

const { t, locale } = useI18n();
const router = useRouter();
const tasks = useTasksStore();
const editor = useTaskEditor();
const actions = useTaskActions();
const toast = useToast();
const prompt = usePrompt();
const confirm = useConfirm();
const settings = useSettingsStore();
const ui = useUiStore();
const { hint } = useShortcutHints();

const quickAdd = ref('');
const quickAddField = ref<HTMLInputElement | null>(null);
const adding = ref(false);
const showCompleted = ref(false);
const focusIndex = ref(0);
const listRef = ref<HTMLElement | null>(null);
const now = ref(new Date());
let clock: ReturnType<typeof setInterval> | null = null;

const activeList = computed(() =>
  tasks.activeListId === ALL_LISTS ? null : tasks.list(tasks.activeListId) ?? null);

const scoped = computed(() =>
  activeList.value ? tasks.tasks.filter((task) => task.list_id === activeList.value!.id) : tasks.tasks);

const groups = computed(() => groupTasks(scoped.value, now.value));
const completedCount = computed(() => groups.value.find((group) => group.group === 'completed')?.tasks.length ?? 0);
const visibleGroups = computed(() =>
  groups.value.filter((group) => group.group !== 'completed' || showCompleted.value));
/** The rows in screen order, for the keyboard. */
const rows = computed(() => visibleGroups.value.flatMap((group) => group.tasks));
const openCount = computed(() => scoped.value.filter((task) => !task.completed_at).length);

const preview = computed(() => {
  const parsed = parseQuickAdd(quickAdd.value, now.value);
  return parsed.dueAt ? { ...parsed, label: dueLabel(parsed.dueAt) } : null;
});

const GROUP_LABELS: Record<TaskGroup, string> = {
  overdue: 'tasks.groups.overdue',
  today: 'tasks.groups.today',
  upcoming: 'tasks.groups.upcoming',
  someday: 'tasks.groups.someday',
  completed: 'tasks.groups.completed',
};

function dueLabel(due: string): string {
  return formatDue(due, locale.value, {
    today: t('tasks.due.today'),
    tomorrow: t('tasks.due.tomorrow'),
    yesterday: t('tasks.due.yesterday'),
  }, now.value);
}

function priorityLabel(task: Task): string | null {
  const level = priorityLevel(task.priority);
  return level === 'none' ? null : t(`tasks.priority.${level}`);
}

function isOverdue(task: Task): boolean {
  return !task.completed_at && dueState(task, now.value) === 'overdue';
}

/** A read-only list (no write privilege, or refused a write) takes no edits. */
function editable(task: Task): boolean {
  return !tasks.list(task.list_id)?.read_only;
}

/** A list the server dropped still lets its leftovers be removed here. */
function deletable(task: Task): boolean {
  const list = tasks.list(task.list_id);
  return !list?.read_only || Boolean(list?.gone);
}

function listOf(task: Task): string {
  const list = tasks.list(task.list_id);
  return list ? listLabel(list) : '';
}

async function refresh() {
  try {
    await tasks.load();
  } catch {
    // CollectionLoadState shows the failure with a retry.
  }
}

async function syncNow(force = true) {
  try {
    const errors = force ? await tasks.sync(true) : await tasks.syncIfStale();
    if (errors.length > 0) toast.error(backendErrorLabel(errors[0]));
  } catch (error) {
    toast.error(backendErrorLabel(error));
  }
}

onMounted(async () => {
  clock = setInterval(() => { now.value = new Date(); }, 60_000);
  followTaskSyncs();
  await refresh();
  // Remote lists catch up in the background, at most every few minutes on
  // open (discovery is a PROPFIND per calendar); Sync tasks forces it.
  if (tasks.lists.some((list) => list.provider !== 'local')) void syncNow(false);
});

onBeforeUnmount(() => {
  if (clock) clearInterval(clock);
});

async function submitQuickAdd() {
  const parsed = parseQuickAdd(quickAdd.value, new Date());
  if (!parsed.title || adding.value) return;
  adding.value = true;
  try {
    await tasks.create({
      title: parsed.title,
      notes: null,
      due_at: parsed.dueAt,
      reminder_at: null,
      priority: 0,
      completed_at: null,
    });
    quickAdd.value = '';
  } catch (error) {
    toast.error(backendErrorLabel(error));
  } finally {
    adding.value = false;
  }
}

function selectList(id: string) {
  tasks.activeListId = id;
  focusIndex.value = 0;
}

async function newList() {
  const name = await prompt.ask({
    title: t('tasks.lists.newTitle'),
    label: t('tasks.lists.name'),
    confirmLabel: t('tasks.lists.create'),
  });
  if (!name?.trim()) return;
  try {
    await tasks.createList(name.trim());
  } catch (error) {
    toast.error(backendErrorLabel(error));
  }
}

async function renameList() {
  const list = activeList.value;
  if (!list) return;
  const name = await prompt.ask({
    title: t('tasks.lists.renameTitle'),
    label: t('tasks.lists.name'),
    initialValue: listName(list),
    confirmLabel: t('common.save'),
  });
  if (!name?.trim()) return;
  try {
    await tasks.renameList(list.id, name.trim());
  } catch (error) {
    toast.error(backendErrorLabel(error));
  }
}

async function deleteList() {
  const list = activeList.value;
  if (!list) return;
  const count = tasks.tasks.filter((task) => task.list_id === list.id).length;
  const confirmed = await confirm.ask({
    title: t('tasks.lists.deleteTitle', { list: listName(list) }),
    body: count > 0 ? t('tasks.lists.deleteBody', { count }, count) : undefined,
    confirmLabel: t('common.delete'),
    tone: 'danger',
  });
  if (!confirmed) return;
  try {
    await tasks.deleteList(list.id);
  } catch (error) {
    toast.error(backendErrorLabel(error));
  }
}

function openMail(task: Task) {
  if (task.thread_id) void router.push({ name: 'thread', params: { id: task.thread_id } });
}

/* ── Keyboard ─────────────────────────────────────────────────────────── */

watch(rows, (next) => {
  if (focusIndex.value >= next.length) focusIndex.value = Math.max(0, next.length - 1);
});

function focused(): Task | undefined {
  return rows.value[focusIndex.value];
}

function focusRow(index: number) {
  if (rows.value.length === 0) return;
  focusIndex.value = Math.min(Math.max(index, 0), rows.value.length - 1);
  void nextTick(() => {
    listRef.value
      ?.querySelectorAll<HTMLElement>('.task-row__main')
      [focusIndex.value]?.focus({ preventScroll: false });
  });
}

function bound(event: KeyboardEvent, action: string): boolean {
  return effectiveBindings(action, settings).some((binding) => matchesKeybinding(event, binding));
}

function onKeydown(event: KeyboardEvent) {
  // Includes an armed `g` sequence: its second key navigates, it is no task key.
  if (pageKeyBlocked(event)) return;
  const task = focused();
  let handled = true;
  if (event.key === 'ArrowDown' || bound(event, 'nextThread')) focusRow(focusIndex.value + 1);
  else if (event.key === 'ArrowUp' || bound(event, 'prevThread')) focusRow(focusIndex.value - 1);
  else if (bound(event, 'compose')) quickAddField.value?.focus();
  else if (!task) handled = false;
  else if (bound(event, 'toggleSelection') || bound(event, 'markDone') || bound(event, 'archive')) {
    if (editable(task)) void actions.toggleCompleted(task);
  } else if (bound(event, 'openItem') || (event.key === 'Enter' && !isButton(event.target))) editor.openTask(task.id);
  else if (bound(event, 'trash') || event.key === 'Delete' || event.key === 'Backspace') {
    if (deletable(task)) void actions.remove(task);
  } else handled = false;
  if (handled) {
    event.preventDefault();
    event.stopPropagation();
  }
}

/** Enter on a focused button presses it; only the row itself opens the editor. */
function isButton(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return Boolean(el?.closest?.('button')) && !el?.classList.contains('task-row__main');
}

// Capture: the global handler must not see a key this page has answered.
onMounted(() => window.addEventListener('keydown', onKeydown, true));
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown, true));

/** The keys as the user has them bound; a switched-off key leaves its row out. */
const legend = computed(() => [
  { keys: [...(hint('nextThread')?.keys ?? []), ...(hint('prevThread')?.keys ?? [])], label: t('tasks.keys.move') },
  { keys: hint('toggleSelection')?.keys ?? hint('markDone')?.keys ?? [], label: t('tasks.keys.complete') },
  { keys: hint('openItem')?.keys ?? [], label: t('tasks.keys.edit') },
  { keys: hint('compose')?.keys ?? [], label: t('tasks.keys.add') },
  { keys: hint('trash')?.keys ?? [], label: t('tasks.keys.delete') },
].filter((item) => item.keys.length > 0));
</script>

<template>
  <section class="tasks-view">
    <PaneHeader
      :title="activeList ? listName(activeList) : $t('tasks.title')"
      :subtitle="activeList?.account_id ? listLabel(activeList) : undefined"
      :count="openCount || undefined"
    >
      <template #actions>
        <IconButton
          v-if="activeList && activeList.provider === 'local'"
          :title="$t('tasks.lists.rename')"
          @click="renameList"
        >
          <AppIcon><path d="M10.5 3.5l2 2L6 12H4v-2l6.5-6.5Z" /></AppIcon>
        </IconButton>
        <IconButton
          v-if="activeList && (activeList.provider === 'local' || activeList.gone)"
          :title="$t('tasks.lists.delete')"
          @click="deleteList"
        >
          <AppIcon><path d="M2.5 4h11M6 2.5h4M4 4l.6 9a1 1 0 001 1h4.8a1 1 0 001-1L12 4" /></AppIcon>
        </IconButton>
        <IconButton
          v-if="tasks.lists.some((list) => list.provider !== 'local')"
          :title="$t('tasks.sync')"
          :disabled="tasks.syncing"
          @click="syncNow()"
        >
          <AppIcon><path d="M13 8a5 5 0 0 1-8.6 3.5M3 8a5 5 0 0 1 8.6-3.5M11.5 2v2.5H14M4.5 14v-2.5H2" /></AppIcon>
        </IconButton>
        <AppButton variant="primary" size="sm" @click="editor.openNew({ listId: tasks.targetListId ?? undefined })">
          {{ $t('tasks.new') }}
        </AppButton>
      </template>
    </PaneHeader>

    <div class="tasks-view__body">
      <nav v-if="ui.isSinglePane" class="tasks-view__lists" :aria-label="$t('tasks.lists.title')">
        <FilterPill
          :label="$t('tasks.lists.all')"
          :pressed="tasks.activeListId === ALL_LISTS"
          @click="selectList(ALL_LISTS)"
        />
        <FilterPill
          v-for="list in tasks.lists"
          :key="list.id"
          :label="listName(list)"
          :dot="list.color"
          :count="list.open_count || undefined"
          :pressed="tasks.activeListId === list.id"
          @click="selectList(list.id)"
        />
        <FilterPill :label="$t('tasks.lists.new')" @click="newList" />
      </nav>

      <p v-if="activeList?.gone" class="tasks-view__notice" role="note">{{ $t('tasks.lists.goneHint') }}</p>
      <p v-else-if="activeList?.read_only" class="tasks-view__notice" role="note">{{ $t('tasks.lists.readOnlyHint') }}</p>

      <form class="quick-add" @submit.prevent="submitQuickAdd">
        <input
          ref="quickAddField"
          v-model="quickAdd"
          class="quick-add__input"
          :placeholder="$t('tasks.quickAdd.placeholder')"
          :aria-label="$t('tasks.quickAdd.label')"
          :disabled="adding || !tasks.targetListId"
          enterkeyhint="done"
          data-testid="task-quick-add"
          @keydown.escape="quickAdd = ''; ($event.target as HTMLInputElement).blur()"
        />
        <p v-if="preview" class="quick-add__preview" aria-live="polite">
          {{ $t('tasks.quickAdd.preview', { title: preview.title, due: preview.label }) }}
        </p>
      </form>

      <CollectionLoadState
        :loading="tasks.loading"
        :failed="tasks.loadFailed"
        :has-content="tasks.tasks.length > 0"
        :error-label="$t('tasks.loadFailed')"
        @retry="refresh"
      />
      <EmptyState
        v-if="!tasks.loading && !tasks.loadFailed && rows.length === 0 && completedCount === 0"
        :title="$t('tasks.empty.title')"
        :description="$t('tasks.empty.description')"
      />

      <div v-if="rows.length || completedCount" ref="listRef" class="task-groups">
        <section v-for="group in visibleGroups" :key="group.group" class="task-group" :aria-label="$t(GROUP_LABELS[group.group])">
          <SectionHeading
            :class="{ 'task-group__heading--overdue': group.group === 'overdue' }"
            :label="$t(GROUP_LABELS[group.group])"
            :count="group.tasks.length"
          />
          <ul class="task-list">
            <li
              v-for="task in group.tasks"
              :key="task.id"
              class="task-row"
              :class="{
                'task-row--focused': rows[focusIndex]?.id === task.id,
                'task-row--done': Boolean(task.completed_at),
              }"
              :data-testid="`task-row-${task.id}`"
            >
              <IconButton
                class="task-row__check"
                :title="task.completed_at ? $t('tasks.markOpen') : $t('tasks.markDone')"
                :active="Boolean(task.completed_at)"
                :aria-pressed="Boolean(task.completed_at)"
                :disabled="!editable(task)"
                data-testid="task-check"
                @click="actions.toggleCompleted(task)"
              >
                <AppIcon v-if="task.completed_at"><path d="M8 2.5a5.5 5.5 0 1 1 0 11a5.5 5.5 0 1 1 0-11Z" /><path d="M5.5 8.2l1.8 1.8L10.7 6.4" /></AppIcon>
                <AppIcon v-else><path d="M8 2.5a5.5 5.5 0 1 1 0 11a5.5 5.5 0 1 1 0-11Z" /></AppIcon>
              </IconButton>
              <ListRowButton
                class="task-row__main"
                @focus="focusIndex = rows.findIndex((row) => row.id === task.id)"
                @click="editor.openTask(task.id)"
              >
                <span class="task-row__title">{{ task.title }}</span>
                <span class="task-row__meta">
                  <span v-if="task.due_at" class="task-row__due" :class="{ 'task-row__due--overdue': isOverdue(task) }">
                    {{ dueLabel(task.due_at) }}
                  </span>
                  <span v-if="task.recurrence" class="task-row__icon task-row__repeat" :title="$t('tasks.repeats')" data-testid="task-repeats">
                    <AppIcon size="sm"><path d="M3 7a5 5 0 0 1 8.6-3.4L13 5M13 2v3h-3M13 9a5 5 0 0 1-8.6 3.4L3 11M3 14v-3h3" /></AppIcon>
                    <span class="task-row__repeat-label">{{ $t('tasks.repeats') }}</span>
                  </span>
                  <span v-if="priorityLabel(task)" class="task-row__priority">{{ priorityLabel(task) }}</span>
                  <span v-if="task.reminder_at && !task.completed_at" class="task-row__icon" :title="$t('tasks.hasReminder')">
                    <AppIcon size="sm"><path d="M4.5 11V7.5a3.5 3.5 0 0 1 7 0V11l1 1.5h-9Z" /><path d="M6.8 14h2.4" /></AppIcon>
                  </span>
                  <span v-if="!activeList && tasks.lists.length > 1" class="task-row__list">{{ listOf(task) }}</span>
                  <span v-if="task.pending" class="task-row__pending">{{ $t('tasks.pending') }}</span>
                </span>
              </ListRowButton>
              <IconButton
                v-if="task.thread_id"
                class="task-row__mail"
                :title="$t('tasks.openMail')"
                @click="openMail(task)"
              >
                <AppIcon><rect x="2" y="3.5" width="12" height="9" rx="1.5" /><path d="m2.5 4.5 5.5 4 5.5-4" /></AppIcon>
              </IconButton>
            </li>
          </ul>
        </section>
        <AppButton
          v-if="completedCount > 0"
          class="task-groups__toggle"
          variant="ghost"
          size="sm"
          @click="showCompleted = !showCompleted"
        >
          {{ showCompleted ? $t('tasks.hideCompleted') : $t('tasks.showCompleted', { count: completedCount }, completedCount) }}
        </AppButton>
      </div>

      <footer v-if="rows.length && !ui.isMobile" class="tasks-legend" :aria-label="$t('tasks.keys.title')">
        <span v-for="item in legend" :key="item.label" class="tasks-legend__item">
          <KeyCap v-for="(key, index) in item.keys" :key="index">{{ key }}</KeyCap>
          <span>{{ item.label }}</span>
        </span>
      </footer>
    </div>
  </section>
</template>

<style scoped>
.tasks-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: var(--bg-base);
  color: var(--text-primary);
}

.tasks-view__body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-md) var(--space-lg) var(--space-xl);
}

.tasks-view__body > * {
  max-width: 760px;
  margin-inline: auto;
}

.tasks-view__lists {
  display: flex;
  gap: var(--space-xs);
  overflow-x: auto;
  padding-bottom: var(--space-sm);
  scrollbar-width: none;
}

.tasks-view__notice {
  margin: 0 0 var(--space-sm);
  padding: var(--space-xs) var(--space-md);
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.task-row__repeat {
  gap: var(--space-2xs);
  align-items: center;
}

.quick-add {
  display: grid;
  gap: var(--space-2xs);
  margin-bottom: var(--space-md);
}

.quick-add__input {
  min-width: 0;
  min-height: 40px;
  border: var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  padding: var(--space-sm) var(--space-md);
  color: var(--text-primary);
  font-family: inherit;
  font-size: var(--font-size-sm);
}

.quick-add__input:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.quick-add__preview {
  margin: 0;
  padding-inline: var(--space-md);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.task-groups {
  display: grid;
  gap: var(--space-md);
}

.task-group {
  display: grid;
  gap: var(--space-xs);
}

.task-group__heading--overdue :deep(.section-heading__label) {
  color: var(--accent-red);
}

.task-list {
  display: grid;
  gap: var(--space-2xs);
  margin: 0;
  padding: 0;
  list-style: none;
}

.task-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-2xs);
  min-width: 0;
  padding: var(--space-2xs) var(--space-xs);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}

.task-row--focused {
  border-color: var(--accent-primary);
}

.task-row__main {
  display: grid;
  gap: var(--space-2xs);
  min-width: 0;
  padding: var(--space-xs);
  text-align: start;
}

.task-row__title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}

.task-row--done .task-row__title {
  color: var(--text-tertiary);
  text-decoration: line-through;
}

.task-row__meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2xs) var(--space-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.task-row__meta:empty {
  display: none;
}

.task-row__due--overdue {
  color: var(--accent-red);
  font-weight: var(--weight-medium);
}

.task-row__priority {
  color: var(--accent-orange);
}

.task-row__icon {
  display: inline-flex;
  color: var(--text-tertiary);
}

.task-row__list,
.task-row__pending {
  color: var(--text-tertiary);
}

.task-row__check[aria-pressed='true'] {
  color: var(--accent-green);
}

.task-groups__toggle {
  justify-self: start;
}

.tasks-legend {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--space-xs) var(--space-md);
  margin-top: var(--space-lg);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.tasks-legend__item {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2xs);
}

/* Phone: touch-sized check and row, the full width for the title. */
@media (max-width: 620px) {
  .tasks-view__body {
    padding: var(--space-sm) var(--space-md) var(--space-xl);
  }

  .quick-add__input {
    min-height: 44px;
    font-size: var(--font-size-md);
  }

  .task-row {
    min-height: 52px;
  }

  .task-row__check,
  .task-row__mail {
    min-width: 44px;
    min-height: 44px;
  }

  .task-row__title {
    white-space: normal;
    font-size: var(--font-size-md);
  }
}
</style>
