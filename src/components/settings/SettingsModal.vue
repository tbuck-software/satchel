<script setup lang="ts">
import { watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useUiStore } from '@/stores/ui';
import { resolveSettingsSection } from './settingsSections';

const route = useRoute();
const router = useRouter();
const ui = useUiStore();

/**
 * Compatibility bridge for callers that still request a settings overlay.
 * Settings are a route now, but signature discovery, tray actions and account
 * setup can keep their focused `openSettings(section)` API.
 */
watch(
  () => ui.settingsOpen,
  async (open) => {
    if (!open) return;

    const requested = ui.settingsSection;
    const section = requested ? resolveSettingsSection(requested) : null;

    ui.settingsOpen = false;
    ui.settingsDetailOpen = false;
    ui.settingsSection = null;

    if (!section) {
      if (route.name === 'settings' && route.query.section === undefined) return;
      await router.push({ name: 'settings' });
      return;
    }

    if (route.name === 'settings' && route.query.section === section) return;
    if (ui.isSinglePane && route.name !== 'settings') {
      // Preserve the settings overview as the intermediate history step:
      // Android Back from a deep-linked section must return there first.
      await router.push({ name: 'settings' });
    }
    await router.push({ name: 'settings', query: { section } });
  },
  { immediate: true },
);
</script>

<template>
  <span hidden />
</template>
