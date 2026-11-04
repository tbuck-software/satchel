<script setup lang="ts">
/**
 * One message's actions on the phone, as rows: the commands, the body's
 * Original / Auto-dark / Text choice and, while its images are blocked, the
 * always-allow choices. The message's own ⋯ sheet and the thread's nav-bar
 * sheet both render this; the message runs every command (see MessageMenu).
 *
 * Its own chunk: it loads with the first sheet that opens.
 */
import MenuItem from '@/components/ui/MenuItem.vue';
import type { MessageMenu, MessageMenuCommand } from './threadMessageMenu';

const props = withDefaults(defineProps<{
  menu: MessageMenu;
  /** The nav-bar sheet leaves Reply to the bottom bar. */
  withReply?: boolean;
}>(), { withReply: true });

/** Before the action runs, so the sheet is gone when a dialog opens. */
const emit = defineEmits<{ done: [] }>();

function act(action: () => void) {
  emit('done');
  action();
}

function run(command: MessageMenuCommand) {
  act(() => props.menu.run(command));
}
</script>

<template>
  <MenuItem label="" v-if="props.withReply && props.menu.canReply" @select="run('reply')">
    <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m8 4-5 5 5 5" /><path d="M3 9h9a5 5 0 0 1 5 5" /></svg></template>
    {{ $t('common.reply') }}
  </MenuItem>
  <MenuItem label="" v-if="props.menu.canReply && props.menu.canReplyAll" @select="run('replyAll')">
    <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M7 5 2 10l5 5" /><path d="M11 5 6 10l5 5M6 10h6a5 5 0 0 1 5 5" /></svg></template>
    {{ $t('common.replyAll') }}
  </MenuItem>
  <MenuItem label="" v-if="props.menu.canReply" @select="run('forward')">
    <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 4 5 5-5 5" /><path d="M17 9H8a5 5 0 0 0-5 5" /></svg></template>
    {{ $t('common.forward') }}
  </MenuItem>
  <MenuItem label="" @select="run('topic')">
    <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 5a2 2 0 0 1 2-2h3l2 2h5a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /><path d="M10 8v6M7 11h6" /></svg></template>
    {{ $t('mail.message.addToTopic') }}
  </MenuItem>
  <MenuItem label="" @select="run('source')">
    <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m8 5-4 5 4 5M12 5l4 5-4 5" /></svg></template>
    {{ $t('mail.message.showSource') }}
  </MenuItem>
  <template v-if="props.menu.bodyView">
    <h3 class="message-actions__section" aria-hidden="true">{{ $t('mail.viewMode.showAs') }}</h3>
    <div role="group" :aria-label="$t('mail.viewMode.showAs')" class="message-actions__group">
      <MenuItem
        v-for="option in props.menu.bodyView.options"
        :key="option.value"
        :label="option.label"
        :checked="option.value === props.menu.bodyView.mode"
        selection="radio"
        @select="act(() => props.menu.bodyView?.set(option.value))"
      >
        <template #icon>
          <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path v-if="option.value === 'auto-dark'" d="M15.5 12.5A6 6 0 0 1 7.5 4.5a6 6 0 1 0 8 8z" />
            <path v-else-if="option.value === 'text' || option.value === 'markdown'" d="M4 5h12M4 9h12M4 13h8" />
            <template v-else><rect x="3.5" y="3.5" width="13" height="13" rx="2" /><path d="M3.5 8h13M8 8v8.5" /></template>
          </svg>
        </template>
      </MenuItem>
    </div>
  </template>
  <MenuItem label="" @select="run('toggleCollapsed')">
    <template #icon><svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path v-if="props.menu.collapsed" d="m5 8 5 5 5-5" /><path v-else d="m5 12 5-5 5 5" /></svg></template>
    {{ props.menu.collapsed ? $t('mail.message.expand') : $t('mail.message.collapse') }}
  </MenuItem>
  <template v-if="props.menu.images">
    <h3 class="message-actions__section" aria-hidden="true">{{ $t('mail.imagePolicy.choicesLabel') }}</h3>
    <div role="group" :aria-label="$t('mail.imagePolicy.choicesLabel')" class="message-actions__group">
      <MenuItem
        v-if="props.menu.images.senderEmail"
        :label="$t('mail.imagePolicy.allowSender')"
        icon="person"
        @select="act(() => props.menu.images?.allowSender())"
      >
        <span class="message-actions__label">
          {{ $t('mail.imagePolicy.allowSender') }}
          <small>{{ props.menu.images.senderEmail }}</small>
        </span>
      </MenuItem>
      <MenuItem
        v-if="props.menu.images.domain"
        :label="$t('mail.imagePolicy.allowDomain', { domain: props.menu.images.domain })"
        icon="link"
        @select="act(() => props.menu.images?.allowDomain())"
      >
        <span class="message-actions__label">
          {{ $t('mail.imagePolicy.allowDomain', { domain: props.menu.images.domain }) }}
          <small>{{ $t('mail.imagePolicy.allowDomainTitle', { domain: props.menu.images.domain }) }}</small>
        </span>
      </MenuItem>
    </div>
  </template>
</template>

<style scoped>
/* Thumb-sized rows: on the phone a sheet is the only way to these actions. */
.menu-item {
  min-height: var(--touch-target);
}

/* A choice among the commands: a quiet label and a hairline above it set
   the group apart. */
.message-actions__section {
  margin: var(--space-xs) 0 0;
  padding: var(--space-md) var(--space-sm) var(--space-xs);
  border-top: 1px solid var(--border-color-muted);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}

.message-actions__group {
  display: grid;
}

/* A hairline closes the group when a command follows it. */
.message-actions__group:not(:last-child) {
  padding-bottom: var(--space-xs);
  margin-bottom: var(--space-xs);
  border-bottom: 1px solid var(--border-color-muted);
}

.message-actions__label {
  display: grid;
  min-width: 0;
  gap: var(--space-2xs);
}

.message-actions__label small {
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
