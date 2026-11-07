<script setup lang="ts">
/**
 * The Tasks area's sidebar on a wide window: every list, grouped by where it
 * lives (this device, then each account), with its open count. A phone shows
 * the same lists as pills in the view.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import NavItem from '@/components/ui/NavItem.vue';
import AppButton from '@/components/ui/AppButton.vue';
import { usePrompt } from '@/composables/usePrompt';
import { useToast } from '@/composables/useToast';
import { useAccountsStore } from '@/stores/accounts';
import { ALL_LISTS, useTasksStore, type TaskList } from '@/stores/tasks';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { listName } from './taskLabels';

const { t } = useI18n();
const tasks = useTasksStore();
const accounts = useAccountsStore();
const prompt = usePrompt();
const toast = useToast();

const allOpen = computed(() => tasks.lists.reduce((sum, list) => sum + list.open_count, 0));

const sections = computed(() => {
  const groups = new Map<string, { key: string; title: string; lists: TaskList[] }>();
  for (const list of tasks.lists) {
    const key = list.account_id ?? 'local';
    if (!groups.has(key)) {
      const account = accounts.accounts.find((item) => item.id === list.account_id);
      groups.set(key, {
        key,
        title: list.account_id ? (account?.email ?? t('tasks.lists.account')) : t('tasks.lists.onThisDevice'),
        lists: [],
      });
    }
    groups.get(key)!.lists.push(list);
  }
  return [...groups.values()];
});

async function newList() {
  const name = await prompt.ask({
    title: t('tasks.lists.newTitle'),
    label: t('tasks.lists.name'),
    confirmLabel: t('tasks.lists.create'),
  });
  if (!name?.trim()) return;
  try {
    await tasks.createList(name.trim());
  } catch (error) {
    toast.error(backendErrorLabel(error));
  }
}
</script>

<template>
  <nav class="tasks-sidebar" :aria-label="$t('tasks.lists.title')">
    <h2>{{ $t('tasks.title') }}</h2>
    <ul>
      <li>
        <NavItem
          :label="$t('tasks.lists.all')"
          :active="tasks.activeListId === ALL_LISTS"
          :count="allOpen || undefined"
          :aria-current="tasks.activeListId === ALL_LISTS ? 'page' : undefined"
          @click="tasks.activeListId = ALL_LISTS"
        />
      </li>
    </ul>
    <template v-for="section in sections" :key="section.key">
      <h3>{{ section.title }}</h3>
      <ul>
        <li v-for="list in section.lists" :key="list.id">
          <NavItem
            :label="listName(list)"
            :active="tasks.activeListId === list.id"
            :count="list.open_count || undefined"
            :aria-current="tasks.activeListId === list.id ? 'page' : undefined"
            @click="tasks.activeListId = list.id"
          >
            <template #icon>
              <span class="tasks-sidebar__dot" :style="list.color ? { background: list.color } : undefined" aria-hidden="true" />
            </template>
          </NavItem>
        </li>
      </ul>
    </template>
    <AppButton class="tasks-sidebar__new" variant="ghost" size="sm" @click="newList">
      {{ $t('tasks.lists.new') }}
    </AppButton>
  </nav>
</template>

<style scoped>
.tasks-sidebar {
  display: flex;
  flex-direction: column;
  gap: var(--space-2xs);
  padding: var(--space-lg) var(--space-2-5) var(--space-md);
}

h2 {
  margin: 0 0 var(--space-2-5);
  padding: 0 var(--space-2-5);
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
}

h3 {
  margin: var(--space-2-5) var(--space-md) var(--space-2xs);
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
  text-overflow: ellipsis;
  white-space: nowrap;
}

ul {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.tasks-sidebar__dot {
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--text-tertiary);
}

.tasks-sidebar__new {
  align-self: flex-start;
  margin: var(--space-sm) var(--space-sm) 0;
}
</style>
