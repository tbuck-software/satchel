<script setup lang="ts">
/*
 * The one filter control of a thread list: which account it is drawn from and
 * how it is laid out. The account choice used to sit in the sidebar and the
 * layout behind a second funnel beside the title — two funnels for one list.
 */
import { computed, defineAsyncComponent, nextTick, ref } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useAccountsStore } from '@/stores/accounts';
import type { AnchorBox } from '@/utils/popoverPlacement';

const props = defineProps<{
  /** Folders have one layout; the grouped view is a mailbox feature. */
  showLayout: boolean;
}>();

// Mounted on first open and then kept, so closing still plays its transition.
const ListFilterMenuPanel = defineAsyncComponent(() => import('./ListFilterMenuPanel.vue'));
const menuMounted = ref(false);

const accounts = useAccountsStore();

const anchor = ref<AnchorBox | null>(null);
let trigger: HTMLElement | null = null;

// Even with one account: "All accounts" is how an account scope is left.
const showAccounts = computed(() => accounts.accounts.length > 0);
const visible = computed(() => showAccounts.value || props.showLayout);

function openMenu(event: MouseEvent) {
  trigger = event.currentTarget as HTMLElement;
  const { right, bottom } = trigger.getBoundingClientRect();
  // Right-aligned under the trigger: it sits at the pane's trailing edge.
  anchor.value = { left: right - 232, right: right - 232, top: bottom + 6, bottom: bottom + 6 };
  menuMounted.value = true;
}

function closeMenu() {
  anchor.value = null;
  void nextTick(() => trigger?.focus({ preventScroll: true }));
}
</script>

<template>
  <template v-if="visible">
    <IconButton
      class="list-filter-menu__trigger"
      :title="$t('inbox.filterMenu')"
      :active="(!!accounts.activeAccountId && accounts.emailAccounts.length > 1) || !!anchor"
      aria-haspopup="menu"
      :aria-expanded="!!anchor"
      @click="openMenu"
    >
      <AppIcon><path d="M2.5 4.5h11M4.5 8h7M6 11.5h4" /></AppIcon>
    </IconButton>
    <ListFilterMenuPanel
      v-if="menuMounted"
      :anchor="anchor"
      :show-accounts="showAccounts"
      :show-layout="props.showLayout"
      @close="closeMenu"
    />
  </template>
</template>

<style scoped>
.list-filter-menu__trigger {
  flex-shrink: 0;
  width: var(--input-height);
  height: var(--input-height);
  border-radius: var(--radius-sm);
}

.list-filter-menu__trigger:not(.icon-button--active) {
  background: var(--bg-raised);
}
</style>
