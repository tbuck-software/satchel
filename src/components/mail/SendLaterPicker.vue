<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { useDismissable } from '@/composables/useDismissable';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import ModalShell from '@/components/ui/ModalShell.vue';

// `d` is aliased: the date helpers below use `d` as a local Date.
const { d: formatTime, t } = useI18n();

const props = defineProps<{
  value: string | null;
}>();

const emit = defineEmits<{
  (e: 'select', value: string | null): void;
  (e: 'close'): void;
}>();

const customDate = ref(props.value ? props.value.slice(0, 16) : defaultLocal());

function defaultLocal() {
  const d = new Date();
  d.setHours(d.getHours() + 1, 0, 0, 0);
  return localIsoMinute(d);
}

function localIsoMinute(d: Date) {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

const presets = computed(() => {
  const tomorrow9 = nextHour(9, 1);
  const tomorrow15 = nextHour(15, 1);
  const monday9 = nextWeekday(1, 9);
  return [
    { label: t('compose.sendLater.tomorrowAt', { time: formatTime(tomorrow9, 'time') }), value: tomorrow9.toISOString() },
    { label: t('compose.sendLater.tomorrowAt', { time: formatTime(tomorrow15, 'time') }), value: tomorrow15.toISOString() },
    { label: t('compose.sendLater.mondayAt', { time: formatTime(monday9, 'time') }), value: monday9.toISOString() },
  ];
});

function nextHour(hour: number, addDays = 0): Date {
  const d = new Date();
  d.setDate(d.getDate() + addDays);
  d.setHours(hour, 0, 0, 0);
  return d;
}

function nextWeekday(day: number, hour: number): Date {
  const d = new Date();
  const diff = (day + 7 - d.getDay()) % 7 || 7;
  d.setDate(d.getDate() + diff);
  d.setHours(hour, 0, 0, 0);
  return d;
}

function chooseCustom() {
  if (!customDate.value) return;
  const local = new Date(customDate.value);
  if (Number.isNaN(local.getTime())) return;
  emit('select', local.toISOString());
}

// Back closes this before whatever it was opened from.
useDismissable(() => true, () => emit('close'));
</script>

<template>
  <ModalShell
    :label="$t('compose.sendLater.title')"
    width="420px"
    height="auto"
    @dismiss="emit('close')"
  >
    <div class="picker-body">
      <h3>{{ $t('compose.sendLater.title') }}</h3>
      <div class="presets">
        <CardButton
          v-for="preset in presets"
          :key="preset.value"
          @click="emit('select', preset.value)"
        >
          {{ preset.label }}
        </CardButton>
      </div>
      <label class="custom">
        <span>{{ $t('compose.sendLater.custom') }}</span>
        <input v-model="customDate" type="datetime-local" />
      </label>
      <div class="actions">
        <AppButton v-if="props.value" class="link" @click="emit('select', null)" variant="ghost" size="sm">
          {{ $t('compose.sendLater.clear') }}
        </AppButton>
        <AppButton class="ghost" @click="emit('close')" variant="ghost" size="sm">{{ $t('common.cancel') }}</AppButton>
        <AppButton class="primary" @click="chooseCustom" variant="primary" size="sm">{{ $t('compose.sendLater.confirm') }}</AppButton>
      </div>
    </div>
  </ModalShell>
</template>

<style scoped>
.picker-body {
  display: grid;
  gap: var(--space-md);
  align-content: start;
  min-height: 0;
  padding: var(--space-lg);
  overflow: auto;
}

h3 {
  margin: 0;
  color: var(--text-primary);
}

.presets {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.presets button {
  background: var(--bg-raised);
  color: var(--text-primary);
  border: 1px solid var(--border-color);
  padding: 9px 12px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
}
.presets button:hover {
  background: var(--bg-hover);
}

.custom {
  display: grid;
  gap: 4px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.custom input {
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  align-items: center;
}

.link { margin-right:auto; }
</style>
