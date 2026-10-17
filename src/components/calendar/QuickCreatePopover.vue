<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import type { CalendarEventDraft } from '@/utils/calendarGrid';
import AppButton from '@/components/ui/AppButton.vue';
import { useI18n } from 'vue-i18n';

const props = defineProps<{
  draft: CalendarEventDraft;
  saving: boolean;
}>();

const emit = defineEmits<{
  save: [title: string];
  more: [title: string];
}>();

const { d, locale, t } = useI18n();

const title = ref(props.draft.title);
const titleField = ref<HTMLInputElement | null>(null);

const whenLabel = computed(() => {
  const dayStr = props.draft.start.toLocaleDateString(locale.value, {
    weekday: 'short',
    day: 'numeric',
    month: 'long',
  });
  if (props.draft.allDay) return t('calendar.range.allDaySingle', { day: dayStr });
  return t('calendar.range.sameDay', {
    day: dayStr,
    start: d(props.draft.start, 'time'),
    end: d(props.draft.end, 'time'),
  });
});

function submit() {
  if (!title.value.trim() || props.saving) return;
  emit('save', title.value.trim());
}

onMounted(() => titleField.value?.focus());
</script>

<template>
  <form class="quick-create" @submit.prevent="submit">
    <input
      ref="titleField"
      v-model="title"
      class="quick-create__title"
      :placeholder="$t('calendar.quick.titlePlaceholder')"
      :aria-label="$t('calendar.editor.titlePlaceholder')"
      autofocus
    />
    <p class="quick-create__when">{{ whenLabel }}</p>
    <footer class="quick-create__actions">
      <AppButton variant="ghost" size="sm" @click="emit('more', title.trim())">
        {{ $t('calendar.quick.moreOptions') }}
      </AppButton>
      <AppButton
        variant="primary"
        size="sm"
        :disabled="saving || !title.trim()"
        @click="submit"
      >
        {{ saving ? $t('common.saving') : $t('calendar.quick.create') }}
      </AppButton>
    </footer>
  </form>
</template>

<style scoped>
.quick-create {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px;
}

.quick-create__title {
  border: 0;
  border-bottom: var(--border-default);
  border-radius: 0;
  background: transparent;
  padding: 4px 2px 8px;
  color: var(--text-primary);
  font-family: inherit;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-semibold);
}

.quick-create__title:focus-visible {
  border-bottom-color: var(--accent-primary);
}

.quick-create__title::placeholder {
  color: var(--text-tertiary);
  font-weight: var(--weight-medium);
}

.quick-create__when {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.quick-create__actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 4px;
}
</style>
