<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import type { BackendAccountVisibilityPolicy } from '@/services/backend';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import FilterPill from '@/components/ui/FilterPill.vue';
import AppButton from '@/components/ui/AppButton.vue';

const props = defineProps<{ accountId: string }>();
const { t, locale } = useI18n();
const accounts = useAccountsStore();
const draft = ref<BackendAccountVisibilityPolicy>({ mode: 'always', weekdays: [1, 2, 3, 4, 5], from: null, to: null, override_date: null, override_visible: null });
const dirty = ref(false);
watch(() => accounts.visibilityFor(props.accountId)?.policy, (policy) => {
  if (policy && !dirty.value) draft.value = { ...policy, weekdays: [...policy.weekdays] };
}, { immediate: true });
const modes = computed(() => [
  { value: 'always', label: t('accountVisibility.always') },
  { value: 'schedule', label: t('accountVisibility.schedule') },
  { value: 'manual', label: t('accountVisibility.manual') },
]);
const weekdays = computed(() => [1, 2, 3, 4, 5, 6, 0].map(day => ({
  value: day,
  label: new Intl.DateTimeFormat(locale.value, { weekday: 'short' }).format(new Date(2026, 0, 4 + day)),
  title: new Intl.DateTimeFormat(locale.value, { weekday: 'long' }).format(new Date(2026, 0, 4 + day)),
})));
const valid = computed(() => !draft.value.from || !draft.value.to || draft.value.from < draft.value.to);
function setMode(mode: string) { draft.value.mode = mode; dirty.value = true; }
function toggleDay(day: number) {
  draft.value.weekdays = draft.value.weekdays.includes(day)
    ? draft.value.weekdays.filter(value => value !== day) : [...draft.value.weekdays, day];
  dirty.value = true;
}
function setTime(field: 'from' | 'to', event: Event) {
  draft.value[field] = (event.target as HTMLInputElement).value || null;
  dirty.value = true;
}
async function save() {
  if (!valid.value) return;
  const policy = { ...draft.value, override_date: null, override_visible: null };
  if (await accounts.setVisibility(props.accountId, policy)) {
    dirty.value = false;
    draft.value = policy;
  }
}
</script>

<template>
  <section class="visibility-settings" :aria-label="t('accountVisibility.title')">
    <div class="visibility-settings__heading">
      <span class="visibility-settings__title">{{ t('accountVisibility.title') }}</span>
      <span class="visibility-settings__status">{{ accounts.isVisible(accountId) ? t('accountVisibility.visible') : t('accountVisibility.resting') }}</span>
    </div>
    <SegmentedControl class="visibility-settings__modes" :model-value="draft.mode" :options="modes" :aria-label="t('accountVisibility.title')" @update:model-value="setMode" />
    <template v-if="draft.mode === 'schedule'">
      <div class="visibility-settings__days" role="group" :aria-label="t('accountVisibility.weekdays')">
        <FilterPill v-for="day in weekdays" :key="day.value" :label="day.label" :title="day.title" :aria-label="day.title" :pressed="draft.weekdays.includes(day.value)" @click="toggleDay(day.value)" />
      </div>
      <div class="visibility-settings__times">
        <label>{{ t('accountVisibility.from') }}<input type="time" :value="draft.from ?? ''" @input="setTime('from', $event)" /></label>
        <label>{{ t('accountVisibility.to') }}<input type="time" :value="draft.to ?? ''" @input="setTime('to', $event)" /></label>
        <span>{{ t('accountVisibility.localTime') }}</span>
      </div>
      <p v-if="!valid" class="visibility-settings__error" role="alert">{{ t('accountVisibility.invalidTime') }}</p>
    </template>
    <p class="visibility-settings__help">{{ t('accountVisibility.help') }}</p>
    <div v-if="dirty" class="visibility-settings__actions">
      <AppButton variant="primary" size="sm" :disabled="!valid || accounts.visibilityBusy || !accounts.visibilityReady" @click="save">{{ t('common.save') }}</AppButton>
    </div>
  </section>
</template>

<style scoped>
.visibility-settings { display: grid; gap: var(--space-sm); flex-basis: 100%; padding-top: var(--space-md); border-top: 1px solid var(--border-color-muted); }
.visibility-settings__modes { justify-self: start; max-width: 100%; }
.visibility-settings__heading { display: flex; justify-content: space-between; gap: var(--space-sm); }
.visibility-settings__title { color: var(--text-primary); font-size: var(--font-size-sm); font-weight: var(--weight-semibold); }
.visibility-settings__status, .visibility-settings__help, .visibility-settings__times { color: var(--text-secondary); font-size: var(--font-size-xs); }
.visibility-settings__days { display: flex; flex-wrap: wrap; gap: var(--space-xs); }
.visibility-settings__times { display: flex; flex-wrap: wrap; align-items: end; gap: var(--space-md); }
.visibility-settings__times label { display: grid; gap: var(--space-xs); }
.visibility-settings__times input { padding: var(--space-xs) var(--space-sm); border: 1px solid var(--border-color); border-radius: var(--radius-sm); background: var(--bg-inset); color: var(--text-primary); font: inherit; }
.visibility-settings__help { margin: 0;  }
.visibility-settings__error { margin: 0; color: var(--accent-red-soft); font-size: var(--font-size-xs); }
.visibility-settings__actions { display: flex; justify-content: flex-end; }
</style>
