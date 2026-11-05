<script setup lang="ts">
import { defineAsyncComponent, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import ModalShell from '@/components/ui/ModalShell.vue';
import { useUiStore } from '@/stores/ui';
import { resolveSettingsSection } from './settingsSections';

const SettingsSidebar = defineAsyncComponent(() => import('./SettingsSidebar.vue'));
const SettingsView = defineAsyncComponent(() => import('@/views/SettingsView.vue'));

const route = useRoute();
const router = useRouter();
const ui = useUiStore();

/**
 * Settings over the mail.
 *
 * With both panes on screen it is a dialog: the section list on the left, the
 * section on the right, the mailbox untouched underneath. Escape, the close
 * button or a click beside it puts the user back where they were. A single
 * pane has no room beside the dialog, so there Settings stays the `/settings`
 * route with its overview and Back; `openSettings(section)` hands over to it.
 */
async function bridgeToRoute() {
  const requested = ui.settingsSection;
  const section = requested ? resolveSettingsSection(requested) : null;
  const account = ui.settingsAccount ?? undefined;

  ui.settingsOpen = false;
  ui.settingsDetailOpen = false;
  ui.settingsSection = null;
  ui.settingsAccount = null;

  if (!section) {
    if (route.name === 'settings' && route.query.section === undefined) return;
    await router.push({ name: 'settings' });
    return;
  }

  if (route.name === 'settings' && route.query.section === section) return;
  if (route.name !== 'settings') {
    // Preserve the settings overview as the intermediate history step:
    // Android Back from a deep-linked section must return there first.
    await router.push({ name: 'settings' });
  }
  await router.push({ name: 'settings', query: { section, ...(account ? { account } : {}) } });
}

watch(
  () => [ui.settingsOpen, ui.isSinglePane] as const,
  ([open, singlePane]) => {
    if (open && singlePane) void bridgeToRoute();
  },
  { immediate: true },
);
</script>

<template>
  <ModalShell
    v-if="!ui.isSinglePane"
    :label="$t('common.settings')"
    width="1180px"
    height="820px"
    @dismiss="ui.closeSettings()"
  >
    <div class="settings-dialog">
      <div class="settings-dialog__nav">
        <SettingsSidebar />
      </div>
      <div class="settings-dialog__main">
        <SettingsView />
        <IconButton
          class="settings-dialog__close"
          :title="$t('common.close')"
          :shortcut="{ label: 'Esc', keys: ['Esc'], aria: 'Escape' }"
          @click="ui.closeSettings()"
        >
          <AppIcon><path d="M4 4l8 8M12 4l-8 8" /></AppIcon>
        </IconButton>
      </div>
    </div>
  </ModalShell>
</template>

<style scoped>
/* A dark dialog over a dimmed dark app needs a drawn edge to read as a sheet. */
:global(.shell-panel:has(> .settings-dialog)) {
  box-shadow: var(--shadow-lg), 0 0 0 1px var(--border-color);
}

.settings-dialog {
  display: grid;
  grid-template-columns: 252px minmax(0, 1fr);
  height: 100%;
  min-height: 0;
}

.settings-dialog__nav {
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding-top: var(--space-sm);
  border-right: 1px solid var(--border-color-muted);
  background: var(--bg-sidebar);
}

.settings-dialog__main {
  position: relative;
  min-width: 0;
  min-height: 0;
}

.settings-dialog__close {
  position: absolute;
  top: var(--space-md);
  right: var(--space-md);
  z-index: 1;
}
</style>
