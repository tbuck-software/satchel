<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import AppIcon from '@/components/ui/AppIcon.vue';
import TabBar from '@/components/ui/TabBar.vue';
import { scrollListToTop } from '@/composables/listScrollTop';
import { useUiStore } from '@/stores/ui';

/**
 * The phone's four areas, always under the thumb.
 *
 * They used to sit at the foot of the drawer, which put the app switcher one
 * level below the mailbox it was switching away from. Mail is every screen
 * that is not one of the other three, as the drawer had it.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const ui = useUiStore();

const AREAS = ['calendar', 'contacts', 'settings'] as const;

const items = computed(() => [
  { id: 'mail', label: t('nav.mail') },
  { id: 'calendar', label: t('nav.calendar') },
  { id: 'contacts', label: t('nav.contacts') },
  { id: 'settings', label: t('common.settings') },
]);

const active = computed(() => {
  const name = String(route.name ?? '');
  return (AREAS as readonly string[]).includes(name) ? name : 'mail';
});

function select(id: string) {
  // Back to mail keeps the list, account and filter the user left behind.
  void router.push({ name: id === 'mail' ? 'inbox' : id });
}

function reselect(id: string) {
  if (id !== 'mail') {
    // Inside an area (a settings section, a contacts filter): back to its top.
    if (Object.keys(route.query).length > 0) void router.push({ name: id });
    return;
  }
  // On a mail screen that is not the list (the outbox), "Mail" goes to the list.
  if (route.name !== 'inbox') {
    void router.push({ name: 'inbox' });
    return;
  }
  scrollListToTop();
}
</script>

<template>
  <TabBar
    class="mobile-tab-bar"
    :label="$t('nav.modules')"
    :items="items"
    :active="active"
    @select="select"
    @reselect="reselect"
  >
    <template #icon="{ item }">
      <AppIcon v-if="item.id === 'mail'" size="lg"><rect x="2" y="3.5" width="12" height="9" rx="1.5" /><path d="m2.5 4.5 5.5 4 5.5-4" /></AppIcon>
      <AppIcon v-else-if="item.id === 'calendar'" size="lg"><rect x="2" y="3" width="12" height="11" rx="2" /><path d="M2 6.5h12M5.5 1.5v3M10.5 1.5v3" /></AppIcon>
      <AppIcon v-else-if="item.id === 'contacts'" size="lg"><circle cx="6" cy="5.5" r="2.5" /><path d="M1.5 13.5c0-2.2 2-4 4.5-4s4.5 1.8 4.5 4M10.5 3.2a2.5 2.5 0 0 1 0 4.6M11.5 9.7c1.7.5 3 2 3 3.8" /></AppIcon>
      <!-- The cog the desktop sidebar uses (Lucide `settings`); the rayed
           circle it replaced read as a sun. -->
      <AppIcon v-else size="lg" :grid="24"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" /><circle cx="12" cy="12" r="3" /></AppIcon>
      <!-- An account needs a look (its certificate is rejected, it cannot
           sync): the mailbox sheet behind Mail says which. -->
      <span v-if="item.id === 'mail' && ui.drawerAttention" class="mobile-tab-bar__dot" />
    </template>
  </TabBar>
</template>

<style scoped>
<style scoped>
.mobile-tab-bar :deep(.tab-bar__icon) {
  position: relative;
}

.mobile-tab-bar__dot {
  position: absolute;
  top: -2px;
  right: -4px;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--accent-orange);
  box-shadow: 0 0 0 2px var(--bg-surface);
}
</style>
