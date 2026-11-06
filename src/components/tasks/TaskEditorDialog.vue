<script setup lang="ts">
/**
 * New task / edit task. Mounted once in `App.vue`; `useTaskEditor()` opens
 * it. A task made from a thread carries a link back to it, shown here as
 * "Open mail".
 *
 * Due is a date, optionally with a time: a date-only due date is what
 * CalDAV clients write as `DUE;VALUE=DATE`, and it keeps meaning "that day"
 * wherever the user is.
 */
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import { useTaskEditor } from '@/composables/useTaskEditor';
import { useToast } from '@/composables/useToast';
import { useTasksStore, fieldsOf, type Task } from '@/stores/tasks';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { backendErrorKey } from '@/utils/backendErrors';
import { splitSubjectPrefixes } from '@/utils/composerText';
import { isDateOnly, localDateKey, priorityLevel, PRIORITY_VALUES, type TaskPriorityLevel } from '@/utils/tasks';
import { listLabel } from './taskLabels';
import { confirmSeriesEnd, dueReceipt, useTaskActions } from './taskActions';

const { t } = useI18n();
const router = useRouter();
const editor = useTaskEditor();
const tasks = useTasksStore();
const toast = useToast();
const actions = useTaskActions();

const request = computed(() => editor.state.request);
const existing = ref<Task | null>(null);
const isEdit = computed(() => existing.value !== null);

const title = ref('');
const notes = ref('');
const listId = ref<string | null>(null);
const dueDate = ref('');
const dueTime = ref('');
const remind = ref(false);
const reminder = ref('');
const priority = ref<TaskPriorityLevel>('none');
const completed = ref(false);
const saving = ref(false);
const error = ref('');
const titleField = ref<HTMLInputElement | null>(null);

const threadId = computed(() => existing.value?.thread_id ?? request.value?.threadId ?? null);
const writableLists = computed(() => tasks.lists.filter((list) => !list.read_only));
const currentList = computed(() => tasks.lists.find((list) => list.id === listId.value) ?? null);
/** A read-only list (or one the server dropped) shows its tasks without edits. */
const locked = computed(() => isEdit.value && Boolean(currentList.value?.read_only));

const priorityOptions = computed(() => [
  { value: 'none', label: t('tasks.priority.none'), disabled: locked.value },
  { value: 'high', label: t('tasks.priority.high'), disabled: locked.value },
  { value: 'medium', label: t('tasks.priority.medium'), disabled: locked.value },
  { value: 'low', label: t('tasks.priority.low'), disabled: locked.value },
]);

function pad(value: number): string {
  return String(value).padStart(2, '0');
}

function localDateTime(date: Date): string {
  return `${localDateKey(date)}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function fill(task: Task | null) {
  const fields = task ? fieldsOf(task) : null;
  title.value = fields?.title
    ?? request.value?.title
    ?? splitSubjectPrefixes(request.value?.subject ?? '').base.trim();
  notes.value = fields?.notes ?? '';
  listId.value = task?.list_id ?? request.value?.listId ?? tasks.targetListId;
  const due = fields?.due_at ?? null;
  if (!due) {
    dueDate.value = '';
    dueTime.value = '';
  } else if (isDateOnly(due)) {
    dueDate.value = due;
    dueTime.value = '';
  } else {
    const local = new Date(due);
    dueDate.value = localDateKey(local);
    dueTime.value = `${pad(local.getHours())}:${pad(local.getMinutes())}`;
  }
  remind.value = Boolean(fields?.reminder_at);
  reminder.value = fields?.reminder_at ? localDateTime(new Date(fields.reminder_at)) : '';
  priority.value = priorityLevel(fields?.priority ?? 0);
  completed.value = Boolean(fields?.completed_at);
  error.value = '';
}

async function open() {
  const current = request.value;
  if (!current) return;
  if (tasks.lists.length === 0 || (current.taskId && !tasks.find(current.taskId))) {
    try {
      await tasks.load();
    } catch {
      // The form still opens; saving reports what went wrong.
    }
  }
  existing.value = current.taskId ? tasks.find(current.taskId) ?? null : null;
  if (current.taskId && !existing.value) {
    toast.error(t('backend.errors.taskNotFound'));
    editor.close();
    return;
  }
  fill(existing.value);
  await nextTick();
  titleField.value?.focus();
  titleField.value?.select();
}

onMounted(() => void open());
watch(request, (next, previous) => {
  if (next && next !== previous) void open();
});

// Turning the reminder on suggests a time: the due time, 9:00 on the due
// day, or tomorrow at 9:00.
watch(remind, (on) => {
  if (!on || reminder.value) return;
  if (dueDate.value) {
    reminder.value = `${dueDate.value}T${dueTime.value || '09:00'}`;
    return;
  }
  const tomorrow = new Date();
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  reminder.value = localDateTime(tomorrow);
});

function dueValue(): string | null {
  if (!dueDate.value) return null;
  if (!dueTime.value) return dueDate.value;
  const instant = new Date(`${dueDate.value}T${dueTime.value}`);
  return Number.isNaN(instant.getTime()) ? dueDate.value : instant.toISOString();
}

function reminderValue(): string | null {
  if (!remind.value || !reminder.value) return null;
  const instant = new Date(reminder.value);
  return Number.isNaN(instant.getTime()) ? null : instant.toISOString();
}

function close() {
  editor.close();
}

useDismissableOnTop(() => request.value !== null, close);

async function submit() {
  if (saving.value || locked.value) return;
  if (!title.value.trim()) {
    error.value = t('backend.errors.taskTitleRequired');
    titleField.value?.focus();
    return;
  }
  saving.value = true;
  error.value = '';
  const original = existing.value;
  const completing = completed.value && !original?.completed_at;
  if (original && completing && !(await confirmSeriesEnd(original))) {
    saving.value = false;
    return;
  }
  // Another client's priority (2, 3, 7, …) stays as it is unless the level
  // was actually changed here; the control only knows three steps.
  const keepsPriority = original && priorityLevel(original.priority) === priority.value;
  const fields = {
    title: title.value,
    notes: notes.value || null,
    due_at: dueValue(),
    reminder_at: reminderValue(),
    priority: keepsPriority ? original.priority : PRIORITY_VALUES[priority.value],
    completed_at: completed.value ? (original?.completed_at ?? new Date().toISOString()) : null,
  };
  try {
    if (original) {
      const saved = await tasks.update(original, fields);
      if (completing && !saved.completed_at && saved.due_at) {
        toast.info(t('tasks.toast.nextOccurrence', { title: saved.title, due: dueReceipt(saved.due_at) }));
      }
    } else {
      await tasks.create(fields, { listId: listId.value, threadId: threadId.value });
      toast.success(threadId.value ? t('tasks.toast.createdFromMail') : t('tasks.toast.created'));
    }
    close();
  } catch (failure) {
    error.value = backendErrorLabel(failure);
    // A sync changed the task meanwhile: take its new revision, keep what
    // was typed, and let Save try again against it.
    if (original && backendErrorKey(failure) === 'backend.errors.taskChanged') {
      try {
        await tasks.load();
        existing.value = tasks.find(original.id) ?? null;
        if (existing.value) error.value = t('tasks.editor.changedRetry');
      } catch {
        // The first message stands.
      }
    }
  } finally {
    saving.value = false;
  }
}

async function remove() {
  const task = existing.value;
  if (!task || saving.value) return;
  saving.value = true;
  try {
    if (await actions.remove(task)) close();
  } finally {
    saving.value = false;
  }
}

function openMail() {
  const id = threadId.value;
  if (!id) return;
  close();
  void router.push({ name: 'thread', params: { id } });
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    void submit();
  }
}
</script>

<template>
  <Teleport to="body">
    <ModalShell
      v-if="request"
      :label="isEdit ? $t('tasks.editor.editTitle') : $t('tasks.editor.newTitle')"
      width="480px"
      height="auto"
      @dismiss="close"
    >
      <form class="task-editor" @submit.prevent="submit" @keydown="onKeydown">
        <header class="task-editor__head">
          <AppButton class="task-editor__bar-action" variant="ghost" size="sm" @click="close">{{ $t('common.cancel') }}</AppButton>
          <h2 class="task-editor__title">{{ isEdit ? $t('tasks.editor.editTitle') : $t('tasks.editor.newTitle') }}</h2>
          <AppButton class="task-editor__bar-action" variant="primary" size="sm" type="submit" :disabled="saving || locked">
            {{ $t('common.save') }}
          </AppButton>
        </header>

        <label class="field">
          <span>{{ $t('tasks.editor.title') }}</span>
          <input
            ref="titleField"
            v-model="title"
            :disabled="saving || locked"
            :placeholder="$t('tasks.editor.titlePlaceholder')"
            data-testid="task-title"
          />
        </label>

        <div v-if="threadId" class="task-editor__mail">
          <span>{{ $t('tasks.editor.fromMail') }}</span>
          <AppButton variant="link" size="sm" @click="openMail">{{ $t('tasks.openMail') }}</AppButton>
        </div>

        <label class="field">
          <span>{{ $t('tasks.editor.list') }}</span>
          <select v-model="listId" :disabled="saving || isEdit" data-testid="task-list">
            <option v-for="list in (isEdit && currentList ? [currentList] : writableLists)" :key="list.id" :value="list.id">
              {{ listLabel(list) }}
            </option>
          </select>
        </label>

        <div class="field-row">
          <label class="field">
            <span>{{ $t('tasks.editor.dueDate') }}</span>
            <input v-model="dueDate" type="date" :disabled="saving || locked" data-testid="task-due-date" />
          </label>
          <label class="field">
            <span>{{ $t('tasks.editor.dueTime') }}</span>
            <input v-model="dueTime" type="time" step="60" :disabled="saving || locked || !dueDate" />
          </label>
        </div>

        <ToggleSwitch v-model="remind" :disabled="saving || locked">{{ $t('tasks.editor.remind') }}</ToggleSwitch>
        <label v-if="remind" class="field">
          <span>{{ $t('tasks.editor.reminderAt') }}</span>
          <input v-model="reminder" type="datetime-local" step="60" :disabled="saving || locked" />
        </label>

        <div class="field">
          <span>{{ $t('tasks.editor.priority') }}</span>
          <SegmentedControl v-model="priority" :options="priorityOptions" :aria-label="$t('tasks.editor.priority')" />
        </div>

        <label class="field">
          <span>{{ $t('tasks.editor.notes') }}</span>
          <textarea v-model="notes" rows="4" :disabled="saving || locked" :placeholder="$t('tasks.editor.notesPlaceholder')" />
        </label>

        <ToggleSwitch v-if="isEdit" v-model="completed" :disabled="saving || locked">{{ $t('tasks.editor.completed') }}</ToggleSwitch>

        <p v-if="locked" class="task-editor__hint" role="note">{{ $t('tasks.editor.readOnlyHint') }}</p>
        <p v-else-if="currentList && currentList.provider !== 'local'" class="task-editor__hint">
          {{ $t('tasks.editor.syncedHint') }}
        </p>
        <p v-if="error" class="task-editor__error" role="alert">{{ error }}</p>

        <footer class="task-editor__actions">
          <AppButton v-if="isEdit" variant="danger" :disabled="saving || (locked && !currentList?.gone)" @click="remove">{{ $t('common.delete') }}</AppButton>
          <span class="task-editor__spacer" />
          <AppButton variant="ghost" @click="close">{{ $t('common.cancel') }}</AppButton>
          <AppButton variant="primary" type="submit" :disabled="saving || locked" data-testid="task-save">
            {{ saving ? $t('common.saving') : $t('common.save') }}
          </AppButton>
        </footer>
      </form>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.task-editor {
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-lg) var(--space-xl);
}

.task-editor__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-sm);
}

.task-editor__title {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  font-weight: var(--weight-semibold);
}

.task-editor__bar-action {
  display: none;
}

.task-editor__mail {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.field {
  display: grid;
  gap: var(--space-2xs);
  min-width: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.field-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-sm);
}

.field input,
.field select,
.field textarea {
  min-width: 0;
  border: var(--border-default);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  padding: var(--space-sm) var(--space-2-5);
  color: var(--text-primary);
  font-family: inherit;
  font-size: var(--font-size-sm);
}

.field input:focus-visible,
.field select:focus-visible,
.field textarea:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.field textarea {
  resize: vertical;
}

.task-editor__hint {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.task-editor__error {
  margin: 0;
  color: var(--accent-red);
  font-size: var(--font-size-sm);
}

.task-editor__actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.task-editor__spacer {
  flex: 1;
}

/* Phone: the sheet is the screen. Cancel and Save sit in its bar, clear of
   the keyboard, as in the event editor; inputs get touch height. */
@media (max-width: 620px) {
  .task-editor {
    padding: var(--space-md);
  }

  .task-editor__bar-action {
    display: inline-flex;
  }

  .task-editor__actions .task-editor__spacer ~ * {
    display: none;
  }

  .field input,
  .field select,
  .field textarea {
    min-height: 44px;
    font-size: var(--font-size-md);
  }

  .field-row {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
