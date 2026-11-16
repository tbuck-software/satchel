import { t } from '@/i18n/runtime';
import { useAccountsStore } from '@/stores/accounts';
import type { TaskList } from '@/stores/tasks';

/** A list's name; the default local list has none of its own. */
export function listName(list: Pick<TaskList, 'name'>): string {
  return list.name.trim() || t('tasks.lists.defaultName');
}

/** The name with the account it syncs with, for pickers that mix accounts. */
export function listLabel(list: TaskList): string {
  if (!list.account_id) return listName(list);
  const account = useAccountsStore().accounts.find((item) => item.id === list.account_id);
  return account ? t('tasks.lists.withAccount', { list: listName(list), account: account.email }) : listName(list);
}
