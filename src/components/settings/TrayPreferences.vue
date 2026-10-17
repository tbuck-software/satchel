<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendSetAutostart,
  backendSetStartMinimized,
  backendSetQuitOnClose,
  backendSupportsNativeCommands,
  backendTrayPreferences,
  type BackendTrayPreferences,
} from '@/services/backend';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const { t } = useI18n();
const prefs = ref<BackendTrayPreferences>({
  quit_on_close: false,
  start_minimized: false,
  autostart_enabled: false,
});
const status = ref('');

onMounted(async () => {
  if (!backendSupportsNativeCommands()) return;
  prefs.value = await backendTrayPreferences();
});

async function toggleQuit(value: boolean) {
  prefs.value.quit_on_close = value;
  try {
    await backendSetQuitOnClose(value);
    status.value = t('settings.tray.updated');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function toggleStartMinimized(value: boolean) {
  prefs.value.start_minimized = value;
  try {
    await backendSetStartMinimized(value);
    status.value = t('settings.tray.updated');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function toggleAutostart(value: boolean) {
  prefs.value.autostart_enabled = value;
  try {
    await backendSetAutostart(value);
    status.value = t('settings.tray.updated');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}
</script>

<template>
  <section>
    <h3>{{ $t('settings.tray.title') }}</h3>
    <p class="hint">{{ $t('settings.tray.hint') }}</p>

    <ToggleSwitch :model-value="!prefs.quit_on_close" @update:model-value="toggleQuit(!$event)">
      {{ $t('settings.tray.minimizeToTray') }}
    </ToggleSwitch>

    <ToggleSwitch
      :model-value="prefs.autostart_enabled"
      @update:model-value="toggleAutostart($event)"
    >
      {{ $t('settings.tray.autostart') }}
    </ToggleSwitch>

    <ToggleSwitch
      :model-value="prefs.start_minimized"
      :disabled="!prefs.autostart_enabled"
      @update:model-value="toggleStartMinimized($event)"
    >
      {{ $t('settings.tray.startMinimized') }}
    </ToggleSwitch>

    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
section {
  display: grid;
  gap: 14px;
}
h3 {
  margin: 0;
  color: var(--text-primary);
}
.hint {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}
.status {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}
</style>
