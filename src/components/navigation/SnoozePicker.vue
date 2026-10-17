<script setup lang="ts">
import { useDismissable } from '@/composables/useDismissable';
import { MAX_RECENT_SNOOZE_DATES } from '@/config/historyLimits';
import { snoozePersistence } from '@/persistence/snooze';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';

const { t, d } = useI18n();

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  choose: [date: string];
  close: [];
}>();

const selected = ref('');
const recentManual = ref<string[]>(readRecentManualDates());

type SnoozePreset = {
  id: string;
  label: string;
  date: Date;
};

function atTime(base: Date, hour: number): Date {
  const date = new Date(base);
  date.setHours(hour, 0, 0, 0);
  return date;
}

function addDays(base: Date, days: number): Date {
  const date = new Date(base);
  date.setDate(date.getDate() + days);
  return date;
}

function nextMondayAtEight(base: Date): Date {
  const date = new Date(base);
  const day = date.getDay();
  const daysUntilNextMonday = ((8 - day) % 7) || 7;
  date.setDate(date.getDate() + daysUntilNextMonday);
  date.setHours(8, 0, 0, 0);
  return date;
}

function firstDayNextMonthAtEight(base: Date): Date {
  return new Date(base.getFullYear(), base.getMonth() + 1, 1, 8, 0, 0, 0);
}

function toDatetimeLocalValue(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function readRecentManualDates(): string[] {
  return snoozePersistence.recentManualDates.read();
}

function storeRecentManualDate(value: string) {
  const next = [value, ...recentManual.value.filter((item) => item !== value)]
    .slice(0, MAX_RECENT_SNOOZE_DATES);
  recentManual.value = next;
  snoozePersistence.recentManualDates.write(next);
}

function formatDate(value: string | Date): string {
  const date = new Date(value);
  return `${d(date, 'weekday')}, ${d(date, 'dateTime')}`;
}

const presets = computed<SnoozePreset[]>(() => {
  const now = new Date();
  const todayAtTwo = atTime(now, 14);
  const tomorrow = addDays(now, 1);
  return [
    { id: 'today-14', label: t('nav.snooze.today14'), date: todayAtTwo },
    { id: 'tomorrow-8', label: t('nav.snooze.tomorrow8'), date: atTime(tomorrow, 8) },
    { id: 'tomorrow-14', label: t('nav.snooze.tomorrow14'), date: atTime(tomorrow, 14) },
    { id: 'next-week', label: t('nav.snooze.nextWeek'), date: nextMondayAtEight(now) },
    { id: 'next-month', label: t('nav.snooze.nextMonth'), date: firstDayNextMonthAtEight(now) },
  ].filter((preset) => preset.date.getTime() > now.getTime());
});

const minManualValue = computed(() => toDatetimeLocalValue(new Date()));

function chooseDate(date: Date | string) {
  const target = new Date(date);
  if (target.getTime() <= Date.now()) {
    return;
  }
  emit('choose', target.toISOString());
  emit('close');
}

function submit() {
  if (!selected.value) {
    return;
  }
  const manualDate = new Date(selected.value);
  if (manualDate.getTime() <= Date.now()) {
    return;
  }
  const isoDate = manualDate.toISOString();
  storeRecentManualDate(isoDate);
  chooseDate(isoDate);
}

// Back closes this before whatever it was opened from.
useDismissable(() => props.open, () => emit('close'));
</script>

<template>
  <Teleport to="body">
    <div v-if="props.open" class="picker" @click.stop>
      <h4>{{ $t('nav.snooze.title') }}</h4>
      <div class="presets">
        <button
          v-for="preset in presets"
          :key="preset.id"
          type="button"
          class="preset"
          @click="chooseDate(preset.date)"
        >
          <span>{{ preset.label }}</span>
          <small>{{ formatDate(preset.date) }}</small>
        </button>
      </div>
      <div v-if="recentManual.length > 0" class="recent">
        <span class="section-label">{{ $t('nav.snooze.recent') }}</span>
        <button
          v-for="date in recentManual"
          :key="date"
          type="button"
          class="recent-item"
          @click="chooseDate(date)"
        >
          {{ formatDate(date) }}
        </button>
      </div>
      <label class="manual">
        <span class="section-label">{{ $t('nav.snooze.manual') }}</span>
        <input v-model="selected" type="datetime-local" :min="minManualValue" />
      </label>
      <div class="actions">
        <button type="button" @click="emit('close')">{{ $t('common.cancel') }}</button>
        <button type="button" @click="submit">{{ $t('common.apply') }}</button>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.picker {
  position: fixed;
  bottom: calc(20px + var(--satchel-safe-area-bottom));
  right: calc(20px + var(--satchel-safe-area-right));
  border: var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--bg-secondary);
  box-shadow: var(--shadow-dropdown);
  padding: 12px;
  display: grid;
  gap: 10px;
  z-index: 45;
  min-width: 260px;
}

h4 {
  margin: 0;
}

.presets,
.recent,
.manual {
  display: grid;
  gap: 6px;
}

.section-label {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  text-transform: uppercase;
}

.preset,
.recent-item {
  border: var(--border-subtle);
  border-radius: var(--radius-sm);
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  padding: 8px 10px;
  cursor: pointer;
  text-align: left;
}

.preset {
  display: grid;
  gap: 2px;
}

.preset:hover,
.recent-item:hover,
button:hover {
  color: var(--text-primary);
  background: var(--bg-raised);
}

.preset small {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

input {
  background: var(--bg-tertiary);
  border: var(--border-subtle);
  border-radius: var(--radius-sm);
  padding: 6px;
}

.actions {
  display: flex;
  justify-content: end;
  gap: 8px;
}

button {
  border: var(--border-subtle);
  border-radius: var(--radius-sm);
  background: var(--bg-tertiary);
  padding: 6px 10px;
}

/* A popover anchored near the top of a phone screen is a stretch away and
   often lands half off it. Rising from the bottom edge puts it where the
   thumb already is. */
@media (max-width: 1020px) {
  .picker {
    align-items: flex-end;
    justify-content: stretch;
    padding: 0;
  }
}
</style>
