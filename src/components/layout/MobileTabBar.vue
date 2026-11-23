<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import AppIcon from '@/components/ui/AppIcon.vue';
import { Calendar, Mail, Settings, Users } from 'lucide';
import type { IconNode } from 'lucide';
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

const MODULE_ICONS: Record<string, IconNode> = {
  mail: Mail,
  calendar: Calendar,
  contacts: Users,
  settings: Settings,
};

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
      <AppIcon size="lg" :icon="MODULE_ICONS[item.id] ?? Settings" />
      <!-- An account needs a look (its certificate is rejected, it cannot
           sync): the mailbox sheet behind Mail says which. -->
      <span v-if="item.id === 'mail' && ui.drawerAttention" class="mobile-tab-bar__dot" />
    </template>
  </TabBar>
</template>

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
