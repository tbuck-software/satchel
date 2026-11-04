<script setup lang="ts">
/*
 * The one filter control of a thread list: which account it is drawn from and
 * how it is laid out. The account choice used to sit in the sidebar and the
 * layout behind a second funnel beside the title — two funnels for one list.
 */
import { computed, defineAsyncComponent, nextTick, ref } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import FilterPill from '@/components/ui/FilterPill.vue';
import { useAccountsStore } from '@/stores/accounts';
import { useUiStore } from '@/stores/ui';
import type { AnchorBox } from '@/utils/popoverPlacement';

const props = defineProps<{
  /** Folders have one layout; the grouped view is a mailbox feature. */
  showLayout: boolean;
}>();

// Mounted on first open and then kept, so closing still plays its transition.
const ListFilterMenuPanel = defineAsyncComponent(() => import('./ListFilterMenuPanel.vue'));
const menuMounted = ref(false);

const accounts = useAccountsStore();
const ui = useUiStore();

/*
 * The phone's chosen account as a chip before the filter button, only when
 * one of several is chosen ("All accounts" has nothing to point at); it opens
 * the mailbox sheet. The desktop sidebar already marks the scope in view, and
 * a chip there only repeated it and narrowed the search field.
 */
const scope = computed(() => (ui.isMobile && accounts.emailAccounts.length > 1 ? accounts.activeAccount : undefined));

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
    <FilterPill
      v-if="scope"
      class="list-filter-menu__scope"
      :label="scope.displayName"
      :dot="scope.color"
      :aria-pressed="undefined"
      aria-haspopup="true"
      @click="ui.openMailboxSwitcher()"
    />
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
.list-filter-menu__scope {
  min-width: 0;
  max-width: 160px;
  background: var(--bg-raised);
}

/* The phone's title row (below the 768px of `ui.isMobile`) is shared with
   the title itself; the chip yields. */
@media (max-width: 767px) {
  .list-filter-menu__scope {
    max-width: 104px;
  }
}

.list-filter-menu__scope :deep(.filter-pill__label) {
  overflow: hidden;
  text-overflow: ellipsis;
}

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
