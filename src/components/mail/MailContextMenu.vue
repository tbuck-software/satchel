<script setup lang="ts">
import { computed } from 'vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';
import { writeClipboardText } from '@/services/clipboard';
import { emailBodyForExport } from '@/utils/emailExport';
import { assessExternalLink, parseMailtoLink, type LinkInteraction, type MailContextInteraction } from '@/utils/externalLinks';

const props = defineProps<{
  interaction: MailContextInteraction | null;
  html?: string;
  text?: string;
}>();
const emit = defineEmits<{ close: []; 'open-link': [link: LinkInteraction] }>();
const toast = useToast();
const anchor = computed(() => props.interaction ? {
  left: props.interaction.x, right: props.interaction.x,
  top: props.interaction.y, bottom: props.interaction.y,
} : null);
const link = computed(() => props.interaction?.link ?? null);
const canOpen = computed(() => link.value && (parseMailtoLink(link.value.href) || assessExternalLink(link.value.href, link.value.label).allowed));

async function copy(kind: 'link' | 'label' | 'selection' | 'text' | 'markdown') {
  const content = kind === 'link' ? link.value?.href
    : kind === 'label' ? link.value?.label
    : kind === 'selection' ? props.interaction?.selection
    : emailBodyForExport(props.html, props.text ?? '', kind);
  emit('close');
  if (!content) return;
  try {
    await writeClipboardText(content);
    toast.success(t('mail.contextMenu.copied'));
  } catch {
    toast.error(t('mail.contextMenu.copyFailed'));
  }
}

function openLink() {
  const target = link.value;
  emit('close');
  if (target) emit('open-link', target);
}
</script>

<template>
  <ContextMenu
    :open="Boolean(interaction)"
    :anchor="anchor"
    :label="$t('mail.contextMenu.label')"
    :width="280"
    @close="emit('close')"
  >
    <div v-if="interaction?.selection" class="menu-group">
      <MenuItem :label="$t('mail.contextMenu.copySelection')" @select="copy('selection')" />
    </div>
    <div v-if="link" class="menu-group">
      <MenuItem :label="$t('mail.contextMenu.copyLink')" icon="link" @select="copy('link')" />
      <MenuItem v-if="canOpen" :label="parseMailtoLink(link.href) ? $t('mail.link.newEmail') : $t('mail.contextMenu.openLink')" @select="openLink" />
      <MenuItem v-if="link.label" :label="$t('mail.contextMenu.copyLinkText')" @select="copy('label')" />
    </div>
    <div class="menu-group">
      <MenuItem :label="$t('mail.contextMenu.copyMessage')" @select="copy('text')" />
      <MenuItem :label="$t('mail.contextMenu.copyMarkdown')" @select="copy('markdown')" />
    </div>
  </ContextMenu>
</template>
