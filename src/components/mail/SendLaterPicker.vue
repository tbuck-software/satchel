<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { useDismissable } from '@/composables/useDismissable';
import { computed, ref, useId } from 'vue';
import { useActionClock } from '@/composables/useActionClock';
import ModalShell from '@/components/ui/ModalShell.vue';
import { toLocalMinute } from '@/utils/sendLater';

const props = defineProps<{
  value: string | null;
}>();

const emit = defineEmits<{
  (e: 'select', value: string | null): void;
  (e: 'close'): void;
}>();

const { now, refresh } = useActionClock(() => true);
const errorId = useId();
const customDate = ref(props.value ? toLocalMinute(new Date(props.value)) : defaultLocal());

const invalidDate = computed(() => {
  const time = new Date(customDate.value).getTime();
  return !Number.isFinite(time) || time <= now.value;
});
const minDate = computed(() => toLocalMinute(new Date(now.value + 60_000)));

function defaultLocal() {
  const d = new Date();
  d.setHours(d.getHours() + 1, 0, 0, 0);
  return toLocalMinute(d);
}

function chooseCustom() {
  refresh();
  if (invalidDate.value) return;
  const local = new Date(customDate.value);
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
      <label class="custom">
        <span>{{ $t('compose.sendLater.custom') }}</span>
        <input v-model="customDate" type="datetime-local" step="60"
          :min="minDate"
          :aria-invalid="invalidDate ? 'true' : undefined"
          :aria-describedby="invalidDate ? errorId : undefined"
        />
      </label>
      <p v-if="invalidDate" :id="errorId" class="date-error" role="alert">{{ $t('compose.sendLater.pastDate') }}</p>
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

.date-error {
  margin: 0;
  color: var(--accent-red-soft);
  font-size: var(--font-size-sm);
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  align-items: center;
}

.link { margin-right:auto; }
</style>
