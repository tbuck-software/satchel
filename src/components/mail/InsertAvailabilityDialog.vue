<script setup lang="ts">
/**
 * "Insert my availability": free windows from the owner's own calendars,
 * within working hours, as a short list of text the composer inserts. The
 * time zone is stated in the text; nothing links to a booking page.
 */
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import ModalShell from '@/components/ui/ModalShell.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import { useAvailability } from '@/composables/useAvailability';
import { useCalendarUiStore } from '@/stores/calendarUi';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import {
  availabilityHtml,
  availabilityLines,
  availabilityPlainText,
  type AvailabilityLines,
} from '@/utils/availabilityText';
import { minuteToTimeInput } from '@/utils/workingHours';

const DURATIONS = [15, 30, 45, 60, 90] as const;
const RANGES = [3, 5, 10] as const;

const emit = defineEmits<{
  close: [];
  insert: [content: { text: string; html: string }];
}>();

const { t, locale } = useI18n();
const calendarUi = useCalendarUiStore();
const availability = useAvailability();

const duration = ref<string>('30');
const range = ref<string>('5');
const lines = ref<AvailabilityLines | null>(null);
const loading = ref(false);
const error = ref('');

const durationOptions = computed(() => DURATIONS.map((minutes) => ({
  value: String(minutes),
  label: minutes % 60 === 0
    ? t('availability.insert.hoursShort', { count: minutes / 60 })
    : t('availability.insert.minutesShort', { count: minutes }),
})));

const rangeOptions = computed(() => RANGES.map((days) => ({
  value: String(days),
  label: t('availability.insert.workingDays', { count: days }, days),
})));

const hoursText = computed(() => {
  const hours = calendarUi.workingHours;
  const names = hours.weekdays.map((day) =>
    // 2024-01-01 was a Monday; ISO day 1 lands on it.
    new Date(2024, 0, day).toLocaleDateString(locale.value, { weekday: 'short' }));
  const end = hours.endMinute >= 24 * 60 ? '24:00' : minuteToTimeInput(hours.endMinute);
  return `${names.join(', ')} · ${minuteToTimeInput(hours.startMinute)}–${end}`;
});

const preview = computed(() => (lines.value ? availabilityPlainText(lines.value) : ''));

let generation = 0;

async function compute() {
  const current = ++generation;
  loading.value = true;
  error.value = '';
  try {
    const minutes = Number(duration.value);
    // Only the owner's own calendars: nobody else is in the meeting yet.
    const report = await availability.freeSlots(
      { account_id: null, attendees: [] },
      { from: new Date(), working_days: Number(range.value), duration_minutes: minutes },
    );
    if (current !== generation) return;
    lines.value = availabilityLines(report, { t, locale: locale.value, durationMinutes: minutes });
  } catch (failure) {
    if (current !== generation) return;
    lines.value = null;
    error.value = t('availability.insert.failed', { error: backendErrorLabel(failure) });
  } finally {
    if (current === generation) loading.value = false;
  }
}

watch([duration, range], () => void compute(), { immediate: true });

function insert() {
  if (!lines.value || loading.value) return;
  emit('insert', { text: availabilityPlainText(lines.value), html: availabilityHtml(lines.value) });
}

useDismissableOnTop(() => true, () => emit('close'));
</script>

<template>
  <ModalShell :label="$t('availability.insert.title')" width="460px" height="auto" @dismiss="emit('close')">
    <div class="availability" data-testid="insert-availability">
      <header class="availability__head">
        <h3>{{ $t('availability.insert.title') }}</h3>
        <IconButton size="sm" :title="$t('common.close')" @click="emit('close')">
          <AppIcon><path d="m4 4 8 8M12 4l-8 8" /></AppIcon>
        </IconButton>
      </header>

      <div class="availability__field">
        <span class="availability__label">{{ $t('availability.insert.duration') }}</span>
        <SegmentedControl v-model="duration" :options="durationOptions" :aria-label="$t('availability.insert.duration')" />
      </div>
      <div class="availability__field">
        <span class="availability__label">{{ $t('availability.insert.range') }}</span>
        <SegmentedControl v-model="range" :options="rangeOptions" :aria-label="$t('availability.insert.range')" />
      </div>

      <p class="availability__hint">{{ $t('availability.insert.hoursHint', { hours: hoursText }) }}</p>

      <div class="availability__preview" aria-live="polite">
        <p v-if="error" class="availability__error" role="alert">{{ error }}</p>
        <p v-else-if="loading && !lines" class="availability__hint">{{ $t('availability.insert.loading') }}</p>
        <p v-else-if="!lines" class="availability__hint">{{ $t('availability.insert.none', { count: Number(range) }, Number(range)) }}</p>
        <pre v-else class="availability__text" data-testid="insert-availability-preview">{{ preview }}</pre>
      </div>

      <footer class="availability__actions">
        <AppButton variant="ghost" @click="emit('close')">{{ $t('common.cancel') }}</AppButton>
        <AppButton variant="primary" :disabled="!lines || loading" data-testid="insert-availability-submit" @click="insert">
          {{ $t('availability.insert.insert') }}
        </AppButton>
      </footer>
    </div>
  </ModalShell>
</template>

<style scoped>
.availability {
  display: grid;
  gap: var(--space-md);
  padding: var(--space-lg);
}

.availability__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-sm);
}

.availability__head h3 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
}

.availability__field {
  display: grid;
  gap: var(--space-xs);
}

.availability__label {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.availability__hint,
.availability__error {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.availability__error {
  color: var(--accent-red);
}

.availability__preview {
  min-height: 5rem;
  border: var(--border-muted);
  border-radius: var(--radius-sm);
  padding: var(--space-sm);
}

.availability__text {
  margin: 0;
  color: var(--text-primary);
  font-family: inherit;
  font-size: var(--font-size-sm);
  white-space: pre-wrap;
}

.availability__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
