<script setup lang="ts">
/**
 * One message's actions on the phone, as rows: the commands, the body's
 * Original / Auto-dark / Text choice and, while its images are blocked, the
 * always-allow choices. The message's own ⋯ sheet and the thread's More
 * sheet both render this; the message runs every command (see MessageMenu).
 *
 * Its own chunk: it loads with the first sheet that opens.
 */
import {
  ChevronsDownUp, ChevronsUpDown, Code, Forward, Languages, Layers, Moon, PanelsTopLeft, Reply,
  ReplyAll, TextAlignStart, type IconNode,
} from 'lucide';
import AppIcon from '@/components/ui/AppIcon.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import type { MessageMenu, MessageMenuCommand } from './threadMessageMenu';

const props = withDefaults(defineProps<{
  menu: MessageMenu;
  /** The More sheet leaves Reply to the bottom bar's own cell. */
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

/** The glyph for one of the body's render modes. */
function bodyViewIcon(mode: string): IconNode {
  if (mode === 'auto-dark') return Moon;
  if (mode === 'text' || mode === 'markdown') return TextAlignStart;
  return PanelsTopLeft;
}
</script>

<template>
  <MenuItem label="" v-if="props.withReply && props.menu.canReply" @select="run('reply')">
    <template #icon><AppIcon :icon="Reply" /></template>
    {{ $t('common.reply') }}
  </MenuItem>
  <MenuItem label="" v-if="props.menu.canReply && props.menu.canReplyAll" @select="run('replyAll')">
    <template #icon><AppIcon :icon="ReplyAll" /></template>
    {{ $t('common.replyAll') }}
  </MenuItem>
  <MenuItem label="" v-if="props.menu.canReply" @select="run('forward')">
    <template #icon><AppIcon :icon="Forward" /></template>
    {{ $t('common.forward') }}
  </MenuItem>
  <MenuItem label="" @select="run('topic')">
    <template #icon><AppIcon :icon="Layers" /></template>
    {{ $t('mail.message.addToTopic') }}
  </MenuItem>
  <MenuItem label="" @select="run('source')">
    <template #icon><AppIcon :icon="Code" /></template>
    {{ $t('mail.message.showSource') }}
  </MenuItem>
  <MenuItem label="" v-if="props.menu.translation" @select="run('translate')">
    <template #icon><AppIcon :icon="Languages" /></template>
    {{ props.menu.translation.shown ? $t('translation.reader.showOriginal') : $t('translation.reader.translate') }}
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
          <AppIcon :icon="bodyViewIcon(option.value)" />
        </template>
      </MenuItem>
    </div>
  </template>
  <MenuItem label="" @select="run('toggleCollapsed')">
    <template #icon><AppIcon :icon="props.menu.collapsed ? ChevronsUpDown : ChevronsDownUp" /></template>
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
