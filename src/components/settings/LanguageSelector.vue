<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import { useSettingsStore } from '@/stores/settings';
import { isLocalePreference } from '@/i18n/locale';

const { t } = useI18n();
const settings = useSettingsStore();

function setPreference(value: string) {
  if (isLocalePreference(value)) settings.setLocalePreference(value);
}
</script>

<template>
  <div class="setting-row">
    <div class="setting-info">
      <span class="setting-label">{{ $t('settings.language.label') }}</span>
      <span class="setting-hint">{{ $t('settings.language.hint') }}</span>
    </div>
    <SegmentedControl
      :model-value="settings.localePreference"
      :options="[
        { value: 'system', label: t('settings.language.system') },
        { value: 'en', label: t('settings.language.en') },
        { value: 'de', label: t('settings.language.de') },
      ]"
      :aria-label="$t('settings.language.label')"
      @update:model-value="setPreference"
    />
  </div>
</template>

<style scoped>
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-lg);
  padding: var(--space-md) var(--space-lg);
}

.setting-info {
  display: grid;
  gap: var(--space-2xs);
}

.setting-label {
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.setting-hint {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}
</style>
