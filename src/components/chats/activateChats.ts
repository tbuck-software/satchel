import { markRaw } from 'vue';
import { registerViewShortcut } from '@/composables/useKeyboard';
import { t } from '@/i18n/runtime';
import { labsSidebarItems } from '@/labs/state';
import { router } from '@/router';
import ChatsNavItem from './ChatsNavItem.vue';

/**
 * Everything Labs → Chats adds to the app while its flag is on: the `/chats`
 * route, its row under the Inbox and `g h` (`g c` is Calendar). Loaded only
 * then (`src/labs/activation.ts`); the returned function takes it all back.
 */
export function activate(): () => void {
  const removeRoute = router.addRoute({
    path: '/chats',
    name: 'chats',
    component: () => import('@/views/ChatsView.vue'),
  });
  // A start on `/chats` (a restored window) found no route yet; resolve it now.
  const current = router.currentRoute.value;
  if (current.path === '/chats' && current.matched.length === 0) void router.replace(current.fullPath);

  const row = markRaw(ChatsNavItem);
  labsSidebarItems.value = [...labsSidebarItems.value, row];
  const stopShortcut = registerViewShortcut({
    action: 'goChats',
    binding: 'g h',
    view: 'chats',
    label: () => t('chats.title'),
  });

  return () => {
    stopShortcut();
    labsSidebarItems.value = labsSidebarItems.value.filter((item) => item !== row);
    if (router.currentRoute.value.name === 'chats') void router.replace({ name: 'inbox' });
    removeRoute();
  };
}
