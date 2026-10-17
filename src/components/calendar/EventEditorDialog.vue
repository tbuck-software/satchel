<script setup lang="ts">
import { useDismissable } from '@/composables/useDismissable';
import { useFocusTrap } from '@/composables/useFocusTrap';
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import {
  addDays,
  startOfDay,
  toDateInput,
  toDateTimeLocalInput,
  type CalendarEventDraft,
} from '@/utils/calendarGrid';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useI18n } from 'vue-i18n';

export interface EventEditorPayload {
  id: string | null;
  title: string;
  location: string | null;
  description: string | null;
  allDay: boolean;
  startsAt: string;
  endsAt: string;
  attendees: string[];
  sendInvites: boolean;
}

const props = defineProps<{
  draft: CalendarEventDraft | null;
  saving: boolean;
}>();

const emit = defineEmits<{
  close: [];
  save: [payload: EventEditorPayload];
}>();

const { t } = useI18n();

const title = ref('');
const location = ref('');
const description = ref('');
const allDay = ref(false);
const startInput = ref('');
const endInput = ref('');
const attendees = ref('');
const sendInvites = ref(false);
const error = ref('');
const titleField = ref<HTMLInputElement | null>(null);
const dialogRef = ref<HTMLElement | null>(null);

const isEdit = computed(() => props.draft?.id != null);

// The title field takes the focus itself, so the trap only keeps Tab off the
// calendar behind the dialog and returns the focus to the day that opened it.
const trap = useFocusTrap(dialogRef, {
  isOpen: () => props.draft !== null,
  moveFocusIn: () => false,
});

watch(
  () => props.draft,
  (draft) => {
    if (!draft) return;
    title.value = draft.title;
    location.value = draft.location;
    description.value = draft.description;
    allDay.value = draft.allDay;
    attendees.value = draft.attendees;
    sendInvites.value = draft.sendInvites;
    syncDateInputs(draft.start, draft.end, draft.allDay);
    error.value = '';
    void nextTick(() => titleField.value?.focus());
  },
  { immediate: true },
);

watch(allDay, (value) => {
  const { start, end } = parseInputs(!value);
  if (start && end) syncDateInputs(start, end, value);
});

function syncDateInputs(start: Date, end: Date, wholeDay: boolean) {
  if (wholeDay) {
    startInput.value = toDateInput(start);
    // The stored end is exclusive; the form shows the inclusive last day.
    const lastDay = end.getTime() > start.getTime() ? addDays(end, -1) : start;
    endInput.value = toDateInput(lastDay.getTime() < start.getTime() ? start : lastDay);
  } else {
    startInput.value = toDateTimeLocalInput(start);
    endInput.value = toDateTimeLocalInput(
      end.getTime() > start.getTime() ? end : new Date(start.getTime() + 60 * 60_000),
    );
  }
}

function parseInputs(wholeDay: boolean): { start: Date | null; end: Date | null } {
  const start = new Date(startInput.value);
  const end = new Date(endInput.value);
  if (!startInput.value || !Number.isFinite(start.getTime())) return { start: null, end: null };
  if (!endInput.value || !Number.isFinite(end.getTime())) return { start, end: null };
  if (wholeDay) {
    return { start: startOfDay(start), end: addDays(startOfDay(end), 1) };
  }
  return { start, end };
}

function submit() {
  if (!title.value.trim()) {
    error.value = t('calendar.editor.errors.titleRequired');
    return;
  }
  const { start, end } = parseInputs(allDay.value);
  if (!start || !end) {
    error.value = t('calendar.editor.errors.invalidDates');
    return;
  }
  if (end.getTime() <= start.getTime()) {
    error.value = t('calendar.editor.errors.endBeforeStart');
    return;
  }
  const attendeeList = attendees.value
    .split(/[,;\n]+/)
    .map((value) => value.trim())
    .filter(Boolean);
  emit('save', {
    id: props.draft?.id ?? null,
    title: title.value.trim(),
    location: location.value.trim() || null,
    description: description.value.trim() || null,
    allDay: allDay.value,
    startsAt: start.toISOString(),
    endsAt: end.toISOString(),
    attendees: attendeeList,
    sendInvites: sendInvites.value && attendeeList.length > 0,
  });
}

function onKeydown(event: KeyboardEvent) {
  if (!props.draft) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    emit('close');
  } else if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    submit();
  }
}

if (typeof window !== 'undefined') window.addEventListener('keydown', onKeydown);
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown));

// Back closes this before whatever it was opened from.
useDismissable(() => props.draft !== null, () => emit('close'));
</script>

<template>
  <Teleport to="body">
    <Transition name="event-editor">
      <div
        v-if="draft"
        class="event-editor-backdrop"
        role="presentation"
        @pointerdown.self="emit('close')"
      >
        <section
          ref="dialogRef"
          class="event-editor"
          role="dialog"
          aria-modal="true"
          aria-labelledby="event-editor-title"
          @keydown.tab="trap.onTab"
        >
          <header class="event-editor__header">
            <h2 id="event-editor-title">{{ isEdit ? $t('calendar.editor.editTitle') : $t('calendar.editor.newTitle') }}</h2>
            <IconButton size="sm" :title="$t('common.close')" @click="emit('close')">✕</IconButton>
          </header>

          <div class="event-editor__form">
            <label class="field">
              <span>{{ $t('calendar.event.title') }}</span>
              <input ref="titleField" v-model="title" :placeholder="$t('calendar.editor.titlePlaceholder')" />
            </label>

            <ToggleSwitch v-model="allDay">{{ $t('calendar.event.allDay') }}</ToggleSwitch>

            <div class="field-row">
              <label class="field">
                <span>{{ $t('calendar.event.start') }}</span>
                <input v-model="startInput" :type="allDay ? 'date' : 'datetime-local'" />
              </label>
              <label class="field">
                <span>{{ $t('calendar.event.end') }}</span>
                <input v-model="endInput" :type="allDay ? 'date' : 'datetime-local'" />
              </label>
            </div>

            <label class="field">
              <span>{{ $t('calendar.event.location') }}</span>
              <input v-model="location" :placeholder="$t('calendar.editor.locationPlaceholder')" />
            </label>

            <label class="field">
              <span>{{ $t('common.description') }}</span>
              <textarea v-model="description" rows="3" :placeholder="$t('calendar.editor.descriptionPlaceholder')"></textarea>
            </label>

            <label class="field">
              <span>{{ $t('calendar.event.attendees') }}</span>
              <input v-model="attendees" :placeholder="$t('calendar.editor.attendeesPlaceholder')" />
            </label>

            <ToggleSwitch v-model="sendInvites">{{ $t('calendar.editor.sendInvites') }}</ToggleSwitch>

            <p v-if="error" class="event-editor__error" role="alert">{{ error }}</p>
          </div>

          <footer class="event-editor__actions">
            <AppButton variant="ghost" @click="emit('close')">{{ $t('common.cancel') }}</AppButton>
            <AppButton variant="primary" :disabled="saving" @click="submit">
              {{ saving ? $t('common.saving') : isEdit ? $t('common.save') : $t('calendar.editor.submit') }}
            </AppButton>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.event-editor-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  padding: 24px;
  background: var(--bg-backdrop);
}

.event-editor {
  display: flex;
  flex-direction: column;
  width: min(460px, 100%);
  max-height: calc(100vh - 48px);
  border: var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  padding: 18px 20px;
  box-shadow: var(--shadow-lg);
}

.event-editor__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 14px;
}

.event-editor__header h2 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  letter-spacing: -0.02em;
}

.event-editor__form {
  display: grid;
  gap: 12px;
  min-height: 0;
  overflow-y: auto;
}

.field {
  display: grid;
  gap: 4px;
  min-width: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.field-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.field input,
.field textarea {
  min-width: 0;
  border: var(--border-default);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
  font-size: var(--font-size-sm);
}

.field input:focus-visible,
.field textarea:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.field textarea {
  resize: vertical;
}

.event-editor__error {
  margin: 0;
  color: var(--accent-red);
  font-size: var(--font-size-sm);
}

.event-editor__actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}

.event-editor-enter-active,
.event-editor-leave-active {
  transition: opacity var(--duration-normal) var(--ease-out);
}
.event-editor-enter-active .event-editor,
.event-editor-leave-active .event-editor {
  transition: transform var(--duration-normal) var(--ease-out),
    opacity var(--duration-normal) var(--ease-out);
}
.event-editor-enter-from,
.event-editor-leave-to {
  opacity: 0;
}
.event-editor-enter-from .event-editor,
.event-editor-leave-to .event-editor {
  opacity: 0;
  transform: translateY(8px) scale(0.985);
}

@media (prefers-reduced-motion: reduce) {
  .event-editor-enter-active,
  .event-editor-leave-active,
  .event-editor-enter-active .event-editor,
  .event-editor-leave-active .event-editor {
    transition: none;
  }
}

/* A dialog needs room around it; a phone screen has none to give. Below the
   breakpoint that splits list from detail, it is the screen instead. */
@media (max-width: 1020px) {
  .event-editor-backdrop {
    padding: 0;
  }

  .event-editor {
    width: 100%;
    max-width: none;
    height: 100%;
    max-height: none;
    padding:
      calc(20px + var(--satchel-safe-area-top))
      calc(20px + var(--satchel-safe-area-right))
      calc(20px + var(--satchel-safe-area-bottom))
      calc(20px + var(--satchel-safe-area-left));
    border-radius: 0;
    box-shadow: none;
  }
}
</style>
