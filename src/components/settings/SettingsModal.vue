<script setup lang="ts">
import { defineAsyncComponent, onBeforeUnmount, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import ModalShell from '@/components/ui/ModalShell.vue';
import { useUiStore } from '@/stores/ui';
import { resolveSettingsSection } from './settingsSections';

const SettingsSidebar = defineAsyncComponent(() => import('./SettingsSidebar.vue'));
const SettingsView = defineAsyncComponent(() => import('@/views/SettingsView.vue'));

/** The list fades its foot only while there is more of it below. */
const nav = ref<HTMLElement | null>(null);
const navOverflows = ref(false);
let navObserver: ResizeObserver | null = null;
function measureNav() {
  const element = nav.value;
  navOverflows.value = !!element && element.scrollHeight - element.scrollTop > element.clientHeight + 1;
}
watch(nav, (element) => {
  navObserver?.disconnect();
  if (!element || typeof ResizeObserver === 'undefined') return;
  navObserver = new ResizeObserver(measureNav);
  navObserver.observe(element);
  if (element.firstElementChild) navObserver.observe(element.firstElementChild);
});
onBeforeUnmount(() => navObserver?.disconnect());

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
    height="860px"
    @dismiss="ui.closeSettings()"
  >
    <div class="settings-dialog">
      <div
        ref="nav"
        class="settings-dialog__nav"
        :class="{ 'settings-dialog__nav--more': navOverflows }"
        @scroll.passive="measureNav"
      >
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
  border-right: 1px solid var(--border-color-muted);
  background: var(--bg-sidebar);
}

/* The pinned title lines up with the section title across the dialog. */
.settings-dialog__nav :deep(.settings-sidebar__head) {
  padding-top: calc(18px + var(--space-sm));
}

/* All twenty sections in view at once on a laptop screen: pointer-sized rows
   and tighter group labels, so nothing hides under the list's foot. */
.settings-dialog__nav :deep(.settings-sidebar) {
  gap: 0;
  padding-bottom: var(--space-sm);
}

.settings-dialog__nav :deep(.settings-sidebar h2) {
  margin-bottom: var(--space-1-5);
}

.settings-dialog__nav :deep(.settings-sidebar h3) {
  margin: var(--space-1-5) 12px 2px;
}

.settings-dialog__nav :deep(.settings-sidebar ul) {
  gap: 0;
}

.settings-dialog__nav :deep(.settings-sidebar .nav-item) {
  min-height: 28px;
  padding-block: var(--space-2xs);
}

/* The list runs on below the fold: fade its foot rather than slice a row. */
.settings-dialog__nav--more {
  mask-image: linear-gradient(to bottom, black calc(100% - var(--space-2xl)), transparent);
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
