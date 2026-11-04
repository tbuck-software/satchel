<script setup lang="ts">
/**
 * The phone thread's one overflow menu, opened from More in its bottom bar:
 * what the conversation offers beyond the bottom bar, then the newest
 * message's actions — the commands, Show as and, while its images are
 * blocked, the always-allow choices. The message runs its own actions; this
 * sheet only lists them (MessageActionList.vue).
 *
 * Its own chunk, loaded on the first tap of More.
 */
import AppIcon from '@/components/ui/AppIcon.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import MessageActionList from './MessageActionList.vue';
import type { MessageMenu } from './threadMessageMenu';

const props = defineProps<{
  open: boolean;
  isStarred: boolean;
  isAside: boolean;
  /** The newest message's actions; null until it has reported them. */
  message: MessageMenu | null;
}>();

const emit = defineEmits<{
  dismiss: [];
  togglePin: [];
  setAside: [];
  delete: [];
}>();

function act(action: 'togglePin' | 'setAside' | 'delete') {
  emit('dismiss');
  if (action === 'togglePin') emit('togglePin');
  if (action === 'setAside') emit('setAside');
  if (action === 'delete') emit('delete');
}
</script>

<template>
  <BottomSheet
    :label="$t('thread.toolbar.moreActions')"
    :open="props.open"
    @dismiss="emit('dismiss')"
  >
    <nav role="menu" class="thread-menu" :aria-label="$t('thread.toolbar.moreActions')">
      <h2 class="thread-menu__heading" aria-hidden="true">{{ $t('thread.view.menuThread') }}</h2>
      <div role="group" :aria-label="$t('thread.view.menuThread')" class="thread-menu__group">
        <MenuItem
          :label="props.isStarred ? $t('thread.actions.unpin') : $t('thread.actions.pin')"
          :icon="props.isStarred ? 'unpin' : 'pin'"
          @select="act('togglePin')"
        />
        <MenuItem
          :label="props.isAside ? $t('thread.actions.moveToInbox') : $t('thread.actions.setAside')"
          :icon="props.isAside ? 'inbox' : 'aside'"
          @select="act('setAside')"
        />
        <MenuItem :label="$t('common.delete')" tone="danger" @select="act('delete')">
          <template #icon>
            <AppIcon>
              <path d="M2 4h12M5 4V2.5A1.5 1.5 0 016.5 1h3A1.5 1.5 0 0111 2.5V4" />
              <path d="M13 4v9.5a1.5 1.5 0 01-1.5 1.5h-7A1.5 1.5 0 013 13.5V4" />
            </AppIcon>
          </template>
        </MenuItem>
      </div>
      <template v-if="props.message">
        <h2 class="thread-menu__heading thread-menu__heading--rule" aria-hidden="true">{{ $t('thread.view.menuLatest') }}</h2>
        <div role="group" :aria-label="$t('thread.view.menuLatest')" class="thread-menu__group">
          <MessageActionList :menu="props.message" :with-reply="false" @done="emit('dismiss')" />
        </div>
      </template>
    </nav>
  </BottomSheet>
</template>

<style scoped>
.thread-menu {
  display: grid;
  width: min(420px, 100vw);
  padding: var(--space-sm) var(--space-md) calc(var(--space-md) + var(--satchel-safe-area-bottom));
}

.thread-menu__group {
  display: grid;
}

/* Thumb-sized rows: the sheet is the phone's only way to these actions. */
.thread-menu__group > .menu-item {
  min-height: var(--touch-target);
}

.thread-menu__heading {
  margin: 0;
  padding: var(--space-sm) var(--space-sm) var(--space-xs);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.thread-menu__heading--rule {
  margin-top: var(--space-sm);
  padding-top: var(--space-md);
  border-top: 1px solid var(--border-color-muted);
}
</style>
