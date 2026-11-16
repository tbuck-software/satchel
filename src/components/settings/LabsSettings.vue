<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import AppBanner from '@/components/ui/AppBanner.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useLabsStore } from '@/stores/labs';
import { useToast } from '@/composables/useToast';
import { LABS_FLAGS, LABS_FLAG_IDS, type LabsFlagId } from '@/labs/flags';
import { LABS_FLAG_COPY } from '@/labs/flagCopy';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

/** Settings → Labs: one switch per experiment, off until someone turns it on. */
const { t } = useI18n();
const labs = useLabsStore();
const toast = useToast();

const flags = computed(() => LABS_FLAG_IDS.map((id) => ({
  id,
  title: t(LABS_FLAG_COPY[id].titleKey),
  description: t(LABS_FLAG_COPY[id].descriptionKey),
  requiresRestart: LABS_FLAGS[id].requiresRestart,
})));

onMounted(() => {
  // The app loads the flags at start; a failed start load gets another try here.
  if (labs.loadFailed) void labs.load();
  else void labs.loadOnce();
});

async function toggle(id: LabsFlagId, value: boolean) {
  try {
    await labs.setEnabled(id, value);
  } catch (error) {
    toast.error(`${t('settings.labs.saveFailed')} ${backendErrorLabel(error)}`);
  }
}
</script>

<template>
  <section class="labs-settings">
    <h2 class="settings-page-title">{{ $t('settings.sections.labs.label') }}</h2>
    <p class="hint">{{ $t('settings.sections.labs.description') }}</p>

    <AppBanner tone="warning" :title="$t('settings.labs.experimental')" :detail="$t('settings.labs.note')" />

    <ErrorState v-if="labs.loadFailed" :title="$t('settings.labs.loadFailed')" :busy="labs.loading" @retry="labs.load()" />
    <div v-else class="setting-group">
      <div v-for="flag in flags" :key="flag.id" class="setting-row" :data-labs-flag="flag.id">
        <div class="setting-info">
          <span class="setting-label">{{ flag.title }}</span>
          <span class="setting-hint">{{ flag.description }}</span>
          <span v-if="flag.requiresRestart" class="setting-hint">{{ $t('settings.labs.restart') }}</span>
        </div>
        <ToggleSwitch
          :model-value="labs.enabled[flag.id]"
          :disabled="labs.saving === flag.id"
          :label="flag.title"
          @update:model-value="toggle(flag.id, $event)"
        />
      </div>
    </div>
  </section>
</template>

<style scoped src="./settingsRows.css"></style>
<style scoped>
.labs-settings {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: var(--space-lg);
}

.labs-settings > .hint {
  margin: 0;
}
</style>
