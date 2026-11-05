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
    <h3 class="settings-page-title">{{ $t('settings.sections.tray.label') }}</h3>
    <p class="hint">{{ $t('settings.tray.hint') }}</p>

    <div class="setting-group">
      <div class="setting-row">
        <span class="setting-label">{{ $t('settings.tray.minimizeToTray') }}</span>
        <ToggleSwitch
          :model-value="!prefs.quit_on_close"
          :label="$t('settings.tray.minimizeToTray')"
          @update:model-value="toggleQuit(!$event)"
        />
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ $t('settings.tray.autostart') }}</span>
        <ToggleSwitch
          :model-value="prefs.autostart_enabled"
          :label="$t('settings.tray.autostart')"
          @update:model-value="toggleAutostart($event)"
        />
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ $t('settings.tray.startMinimized') }}</span>
        <ToggleSwitch
          :model-value="prefs.start_minimized"
          :disabled="!prefs.autostart_enabled"
          :label="$t('settings.tray.startMinimized')"
          @update:model-value="toggleStartMinimized($event)"
        />
      </div>
    </div>

    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
