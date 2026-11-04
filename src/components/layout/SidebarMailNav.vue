<script setup lang="ts">
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import NavItem from '@/components/ui/NavItem.vue';
import PluginSlotHost from '@/components/plugins/PluginSlotHost.vue';
import { computed, defineAsyncComponent } from 'vue';
import { useRouter } from 'vue-router';
import { useAccountsStore } from '@/stores/accounts';
import { useSmartFoldersStore } from '@/stores/smartFolders';
import { useListSearchStore } from '@/stores/listSearch';
import { useThreadsStore } from '@/stores/threads';
import MailboxTree from './MailboxTree.vue';

/**
 * Everything the app offers for picking a mail list: accounts, the mailbox
 * sections and favourite folders, smart folders, feeds and sources.
 *
 * The desktop sidebar renders it in its scroll area; the phone shows the same
 * rows in the mailbox switcher sheet. `navigate` fires on every pick so the
 * sheet can close, and `openFoldersPanel` asks the host for the folder browser.
 */
const emit = defineEmits<{
  navigate: [];
  openFoldersPanel: [];
}>();

const SidebarAccounts = defineAsyncComponent(() => import('./SidebarAccounts.vue'));

const router = useRouter();
const accounts = useAccountsStore();
const threads = useThreadsStore();
const smartFolders = useSmartFoldersStore();
const listSearch = useListSearchStore();

const sourceAccounts = computed(() => accounts.accounts.filter((account) => account.kind !== 'email'));

function openSource(id: string) {
  emit('navigate');
  accounts.setActiveAccount(id);
  threads.workflowFilter = 'all';
  threads.setMailboxFilter({ kind: 'section', value: 'inbox' });
  void router.push({ name: 'inbox' });
}

/**
 * A smart folder is a saved search: it opens as the list search over all
 * mail, the same backend search with its operator syntax (`from:`,
 * `has:attachment`, …) that `parse_search_query` understands.
 */
function openSmartFolder(query: string) {
  emit('navigate');
  listSearch.open(query, 'all');
  void router.push({ name: 'inbox' });
}
</script>

<template>
  <SidebarAccounts @navigate="emit('navigate')" />
  <nav class="px-3">
    <MailboxTree @open-folders-panel="emit('openFoldersPanel')" @navigate="emit('navigate')" />
    <PluginSlotHost slot-name="sidebar_section" />
  </nav>
  <!-- Smart folders (saved searches) -->
  <nav v-if="smartFolders.items.length > 0 || smartFolders.loadFailed" class="grid gap-0.5 px-3 pb-2 pt-5" :aria-busy="smartFolders.loading">
    <span class="text-2xs font-semibold text-tertiary px-3 mb-1">{{ $t('nav.smartFolders') }}</span>
    <CollectionLoadState
      :loading="smartFolders.loading"
      :failed="smartFolders.loadFailed"
      :has-content="smartFolders.items.length > 0"
      :error-label="$t('settings.smartFolders.loadFailed')"
      @retry="smartFolders.load()"
    />
    <NavItem
      v-for="folder in smartFolders.items"
      :key="folder.id"
      :label="folder.name"
      @click="openSmartFolder(folder.query)"
    >
      <template #icon>
        <span class="text-base leading-none">{{ folder.icon || '🔍' }}</span>
      </template>
    </NavItem>
  </nav>

  <!-- Feeds and other non-mail sources -->
  <nav v-if="sourceAccounts.length" class="grid gap-1 px-2 py-3 border-t border-border-muted">
    <span class="text-2xs font-bold uppercase tracking-[0.08em] text-tertiary px-2">{{ $t('nav.feedsAndSources') }}</span>
    <NavItem v-for="source in sourceAccounts" :key="source.id" :label="source.displayName" :active="accounts.activeAccountId===source.id" @click="openSource(source.id)">
      <template #icon><span class="source-orb" :style="{background:source.color}" /></template>
    </NavItem>
  </nav>
</template>

<style scoped>
.source-orb{display:block;width:10px;height:10px;border-radius:50%;box-shadow:0 0 0 4px color-mix(in srgb,currentColor 10%,transparent)}
</style>
