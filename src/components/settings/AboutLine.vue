<script setup lang="ts">
import { onMounted, ref } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import { useAppUpdateStore } from '@/stores/appUpdate';
import {
  backendAppBuildInfo,
  backendCommandsAvailable,
  type BackendBuildInfo,
} from '@/services/backend';

const build = ref<BackendBuildInfo | null>(null);
const updates = useAppUpdateStore();

onMounted(async () => {
  if (!backendCommandsAvailable()) return;
  try {
    build.value = await backendAppBuildInfo();
  } catch {
    // Nothing to show; the Diagnostics pane reports the same failure.
  }
});
</script>

<template>
  <p v-if="build" class="about" data-testid="about-line">
    {{ $t('settings.about.line', { version: build.version, commit: build.commit }) }}
  </p>
  <div v-if="build" class="update-controls">
    <AppButton variant="ghost" size="sm" :disabled="updates.checking || updates.visible" @click="updates.checkForUpdates()">
      {{ $t(updates.checking ? 'app.update.checking' : 'app.update.check') }}
    </AppButton>
    <span v-if="updates.checkError" role="status">{{ updates.checkError }}</span>
    <span v-else-if="updates.checked && !updates.visible" role="status">
      {{ $t(updates.channel ? 'app.update.upToDate' : 'app.update.sourceBuild') }}
    </span>
  </div>
</template>

<style scoped>
.update-controls {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.about {
  margin: var(--space-lg) 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  user-select: text;
}
</style>
