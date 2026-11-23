<script setup lang="ts">
import type { Attachment } from '@/types/mail';
import { Download, ExternalLink, Eye, File as FileIcon } from 'lucide';
import { useAttachmentOpen } from '@/composables/useAttachmentOpen';
import { formatBytes } from '@/utils/formatBytes';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AttachmentPreview from './AttachmentPreview.vue';

const props = defineProps<{
  attachment: Attachment;
}>();

const open = useAttachmentOpen(() => props.attachment);
</script>

<template>
  <div
    class="flex items-center gap-2 bg-surface py-3 pl-3 pr-2 rounded-xl border border-border w-64 group hover:border-accent-primary/50 focus-within:border-accent-primary focus-within:ring-2 focus-within:ring-accent-primary/20 transition-colors"
  >
    <CardButton
      class="items-center gap-3 flex-1"
      :title="open.primaryTitle.value"
      :disabled="open.busy.value"
      :busy="open.opening.value"
      @click="open.openPrimary"
      @keydown="open.handlePreviewKey"
    >
      <div class="w-10 h-10 bg-accent-primary/15 text-accent-primary rounded flex items-center justify-center shrink-0">
        <AppIcon :icon="FileIcon" />
      </div>
      <div class="flex-1 min-w-0 grid gap-px">
        <span class="text-sm font-medium text-primary truncate">{{ props.attachment.filename }}</span>
        <span class="text-xs text-tertiary">{{ formatBytes(props.attachment.size) }}</span>
      </div>
    </CardButton>
    <IconButton
      size="sm"
      :title="open.alternateTitle.value"
      :disabled="open.busy.value"
      @click="open.openAlternate"
    >
      <AppIcon v-if="open.systemFirst.value" :icon="Eye" size="sm" />
      <AppIcon v-else :icon="ExternalLink" size="sm" />
    </IconButton>
    <IconButton
      size="sm"
      :title="$t('mail.attachment.save')"
      :disabled="open.busy.value"
      @click="open.save"
    >
      <AppIcon :icon="Download" size="sm" />
    </IconButton>
  </div>
  <AttachmentPreview
    v-if="open.previewing.value"
    :attachment-id="props.attachment.id"
    :filename="props.attachment.filename"
    :mime-type="props.attachment.contentType"
    @close="open.previewing.value = false"
  />
</template>
