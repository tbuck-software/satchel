<script setup lang="ts">
import type { Attachment } from '@/types/mail';
import { useAttachmentOpen } from '@/composables/useAttachmentOpen';
import { formatBytes } from '@/utils/formatBytes';
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
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <path d="M14 4.5V14a2 2 0 01-2 2H4a2 2 0 01-2-2V2a2 2 0 012-2h5.5L14 4.5z"/>
          <path d="M14 4.5H9.5V0"/>
        </svg>
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
      <svg v-if="open.systemFirst.value" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M1.5 8s2.4-4 6.5-4 6.5 4 6.5 4-2.4 4-6.5 4-6.5-4-6.5-4z"/>
        <circle cx="8" cy="8" r="1.8"/>
      </svg>
      <svg v-else width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M9.5 2.5h4v4M7 9l6.5-6.5"/>
        <path d="M13 9.5V13a1 1 0 01-1 1H3a1 1 0 01-1-1V4a1 1 0 011-1h3.5"/>
      </svg>
    </IconButton>
    <IconButton
      size="sm"
      :title="$t('mail.attachment.save')"
      :disabled="open.busy.value"
      @click="open.save"
    >
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M8 2v8M4.5 6.5L8 10l3.5-3.5M2.5 13.5h11"/>
      </svg>
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
