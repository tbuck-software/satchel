<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import type { Attachment } from '@/types/mail';
import { useAttachmentActions } from '@/composables/useAttachmentActions';
import { useAttachmentOpen } from '@/composables/useAttachmentOpen';
import { formatBytes } from '@/utils/formatBytes';
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AttachmentPreview from './AttachmentPreview.vue';

const props = defineProps<{
  attachment: Attachment;
}>();

const attachmentActions = useAttachmentActions();
const open = useAttachmentOpen(() => props.attachment);

/*
 * A picture shows itself in the tile; everything else shows what it is.
 *
 * The bound is the point: the card fetches the bytes to draw the thumbnail,
 * so a 40 MB raw photo would pull 40 MB over IPC to fill a 36-pixel tile.
 * Past the bound the tile shows the type instead, which costs nothing.
 */
const THUMBNAIL_MAX_BYTES = 2_000_000;

const isThumbnailable = computed(
  () => props.attachment.contentType.startsWith('image/')
    && !props.attachment.contentType.includes('svg')
    && props.attachment.size <= THUMBNAIL_MAX_BYTES,
);

/*
 * The tile's tint says what kind of file it is before the name is read. Only
 * the families someone acts on differently get a colour; the rest stay ink.
 */
const FILE_KINDS: Record<string, string> = {
  PDF: 'pdf',
  DOC: 'document', DOCX: 'document', ODT: 'document', RTF: 'document', PAGE: 'document',
  XLS: 'sheet', XLSX: 'sheet', ODS: 'sheet', CSV: 'sheet', NUMB: 'sheet',
  ZIP: 'archive', GZ: 'archive', TGZ: 'archive', TAR: 'archive', '7Z': 'archive', RAR: 'archive',
};

const extension = computed(() => {
  const dot = props.attachment.filename.lastIndexOf('.');
  if (dot <= 0) return props.attachment.contentType.split('/')[1]?.slice(0, 4).toUpperCase() ?? '';
  return props.attachment.filename.slice(dot + 1).slice(0, 4).toUpperCase();
});

const fileKind = computed(() => FILE_KINDS[extension.value] ?? 'other');

const thumbnailUrl = ref('');
const rootRef = ref<HTMLElement | null>(null);
let observer: IntersectionObserver | null = null;

function revoke() {
  if (thumbnailUrl.value) URL.revokeObjectURL(thumbnailUrl.value);
  thumbnailUrl.value = '';
}

async function loadThumbnail() {
  if (thumbnailUrl.value || !isThumbnailable.value) return;
  try {
    const data = await attachmentActions.downloadAttachment(props.attachment.id);
    const { attachmentBytes } = await import('@/utils/attachmentBytes');
    const bytes = await attachmentBytes(data);
    thumbnailUrl.value = URL.createObjectURL(
      new Blob([bytes as BlobPart], { type: props.attachment.contentType }),
    );
  } catch {
    // A thumbnail is decoration: the card still opens and still saves, and a
    // toast about a picture nobody asked to see would be noise.
  }
}

// Only once the card is on screen. A thread with twenty photos would
// otherwise fetch twenty files before the first one is scrolled to.
watch(rootRef, (element) => {
  observer?.disconnect();
  observer = null;
  if (!element || !isThumbnailable.value) return;
  if (typeof IntersectionObserver === 'undefined') {
    void loadThumbnail();
    return;
  }
  observer = new IntersectionObserver((entries) => {
    if (!entries.some((entry) => entry.isIntersecting)) return;
    observer?.disconnect();
    observer = null;
    void loadThumbnail();
  });
  observer.observe(element);
}, { immediate: true });

onBeforeUnmount(() => {
  observer?.disconnect();
  revoke();
});
</script>

<template>
  <!--
    One row for every kind of file, so a group of them lines up: a picture
    puts its thumbnail where any other file has its type tile. A preview card
    above the name was four times the height of the row beside it, and the
    group read as two sizes that happened to share a line. The full picture
    is one click away in the preview.
  -->
  <div ref="rootRef" class="attachment-card group">
    <CardButton
      class="attachment-card__open"
      :title="open.primaryTitle.value"
      :disabled="open.busy.value"
      :busy="open.opening.value"
      @click="open.openPrimary"
      @keydown="open.handlePreviewKey"
    >
      <span v-if="isThumbnailable" class="attachment-card__tile attachment-card__tile--image" aria-hidden="true">
        <img v-if="thumbnailUrl" :src="thumbnailUrl" alt="">
        <!-- Until the bytes arrive, or if they never do: still a picture. -->
        <svg v-else width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="2" y="3" width="12" height="10" rx="1.5" />
          <circle cx="5.75" cy="6.25" r="1.25" />
          <path d="m14 10.5-3.5-3.5L4 13" />
        </svg>
      </span>
      <span v-else class="attachment-card__tile" :data-kind="fileKind" aria-hidden="true">
        <template v-if="extension">{{ extension }}</template>
        <svg v-else width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M13 5.5V13a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 3 13V3a1.5 1.5 0 0 1 1.5-1.5H9L13 5.5Z" />
          <path d="M9 1.5v4h4" />
        </svg>
      </span>
      <span class="attachment-card__meta">
        <span class="attachment-card__name">{{ props.attachment.filename }}</span>
        <span class="attachment-card__size">{{ formatBytes(props.attachment.size) }}</span>
      </span>
    </CardButton>

    <span class="attachment-card__actions">
      <IconButton
        size="sm"
        :title="open.alternateTitle.value"
        :disabled="open.busy.value"
        @click="open.openAlternate"
      >
        <svg v-if="open.systemFirst.value" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M1.5 8s2.4-4 6.5-4 6.5 4 6.5 4-2.4 4-6.5 4-6.5-4-6.5-4z" />
          <circle cx="8" cy="8" r="1.8" />
        </svg>
        <svg v-else width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M9.5 2.5h4v4M7 9l6.5-6.5" />
          <path d="M13 9.5V13a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h3.5" />
        </svg>
      </IconButton>
      <IconButton
        size="sm"
        :title="$t('mail.attachment.save')"
        :disabled="open.busy.value"
        @click="open.save"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M8 2v8M4.5 6.5L8 10l3.5-3.5M2.5 13.5h11" />
        </svg>
      </IconButton>
    </span>

    <AttachmentPreview
      v-if="open.previewing.value"
      :attachment-id="props.attachment.id"
      :filename="props.attachment.filename"
      :mime-type="props.attachment.contentType"
      @close="open.previewing.value = false"
    />
  </div>
</template>

<style scoped>
.attachment-card {
  position: relative;
  width: 216px;
  border: var(--border-default);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-raised) 60%, var(--bg-surface));
  overflow: hidden;
  transition: background var(--duration-fast) var(--ease-out);
}

.attachment-card:hover {
  background: var(--bg-raised);
}

.attachment-card__open {
  width: 100%;
  align-items: center;
  gap: var(--space-2-5);
  padding: var(--space-sm);
}

.attachment-card__tile {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--text-tertiary) 14%, transparent);
  color: var(--text-secondary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.04em;
}

.attachment-card__tile--image {
  overflow: hidden;
}

.attachment-card__tile--image img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.attachment-card__tile[data-kind='pdf'] {
  background: color-mix(in srgb, var(--accent-red) 14%, transparent);
  color: var(--accent-red);
}

.attachment-card__tile[data-kind='document'] {
  background: color-mix(in srgb, var(--accent-primary) 16%, transparent);
  color: var(--accent-primary);
}

.attachment-card__tile[data-kind='sheet'] {
  background: color-mix(in srgb, var(--accent-green) 14%, transparent);
  color: var(--accent-green);
}

.attachment-card__tile[data-kind='archive'] {
  background: color-mix(in srgb, var(--accent-amber) 14%, transparent);
  color: var(--accent-amber);
}

.attachment-card__meta {
  display: flex;
  flex-direction: column;
  gap: var(--space-2xs);
  min-width: 0;
}

.attachment-card__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-medium);
}

.attachment-card__size {
  color: var(--text-secondary);
  font-size: var(--font-size-2xs);
}

/* Over the card, not beside the name: the card is one target, and the two
   secondary ways in should not compete with it until the pointer is there. */
.attachment-card__actions {
  position: absolute;
  top: var(--space-1-5);
  right: var(--space-1-5);
  display: flex;
  gap: var(--space-2xs);
  padding: var(--space-2xs);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  box-shadow: var(--shadow-sm);
  opacity: 0;
  transition: opacity var(--duration-fast) var(--ease-out);
}

.attachment-card__actions {
  top: 50%;
  transform: translateY(-50%);
}

.attachment-card:hover .attachment-card__actions,
.attachment-card:focus-within .attachment-card__actions {
  opacity: 1;
}
</style>
