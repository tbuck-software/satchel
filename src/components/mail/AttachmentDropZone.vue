<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { BackendOutgoingAttachment } from '@/types/composer';
import { formatBytes } from '@/utils/formatBytes';
import {
  attachmentsSize,
  encodedAttachmentSize as encodedSize,
  MAX_ATTACHMENT_BYTES as MAX_BYTES,
  MAX_TOTAL_ATTACHMENT_BYTES as MAX_TOTAL_BYTES,
} from '@/utils/attachmentLimits';

const props = withDefaults(defineProps<{
  modelValue: BackendOutgoingAttachment[];
  inlineEnabled?: boolean;
  showEmptyState?: boolean;
}>(), {
  inlineEnabled: false,
  showEmptyState: true,
});

const emit = defineEmits<{
  (e: 'update:modelValue', value: BackendOutgoingAttachment[]): void;
  (e: 'insert-inline', value: BackendOutgoingAttachment): void;
  (e: 'remove-inline', value: BackendOutgoingAttachment): void;
  (e: 'move-to-attachment', value: BackendOutgoingAttachment): void;
}>();

const { t } = useI18n();
const isOver = ref(false);
const fileInput = ref<HTMLInputElement | null>(null);
const errorText = ref('');

const visibleAttachments = computed(() =>
  props.modelValue.map((attachment, index) => ({ attachment, index })),
);

const usedBytes = computed(() => attachmentsSize(props.modelValue));

/** The extension as a short badge, so a row of chips still tells mail from image. */
function fileKind(filename: string): string {
  const dot = filename.lastIndexOf('.');
  const extension = dot > 0 ? filename.slice(dot + 1) : '';
  return /^[a-z0-9]{1,4}$/i.test(extension) ? extension.toUpperCase() : '·';
}

async function readFile(file: File, alreadyUsed: number): Promise<BackendOutgoingAttachment | null> {
  if (file.size > MAX_BYTES) {
    errorText.value = t('mail.attachment.tooLarge', {
      name: file.name,
      limit: formatBytes(MAX_BYTES),
    });
    return null;
  }
  if (alreadyUsed + file.size > MAX_TOTAL_BYTES) {
    errorText.value = t('mail.attachment.totalExceeded', {
      name: file.name,
      limit: formatBytes(MAX_TOTAL_BYTES),
      used: formatBytes(alreadyUsed),
    });
    return null;
  }
  let base64: string;
  try {
    base64 = bufferToBase64(new Uint8Array(await file.arrayBuffer()));
  } catch {
    errorText.value = t('mail.attachment.readFailed', { name: file.name });
    return null;
  }
  return {
    filename: file.name,
    mime_type: file.type || 'application/octet-stream',
    content_base64: base64,
    inline: false,
  };
}

function bufferToBase64(bytes: Uint8Array): string {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

// Batches run one after another, so each one budgets against what the
// previous drop already added.
let pendingBatch: Promise<void> = Promise.resolve();

function addFiles(list: FileList | null): Promise<void> {
  if (!list || list.length === 0) return pendingBatch;
  const files = Array.from(list);
  pendingBatch = pendingBatch.then(() => addBatch(files));
  return pendingBatch;
}

async function addBatch(files: File[]) {
  errorText.value = '';
  const added: BackendOutgoingAttachment[] = [];
  // Track the running total so a batch drop cannot collectively blow the cap.
  let used = usedBytes.value;
  for (const file of files) {
    const att = await readFile(file, used);
    if (att) {
      added.push(att);
      used += file.size;
    }
  }
  if (added.length === 0) return;
  // Read after the awaits: removals made meanwhile must survive.
  emit('update:modelValue', [...props.modelValue, ...added]);
}

function onDrop(event: DragEvent) {
  event.preventDefault();
  isOver.value = false;
  void addFiles(event.dataTransfer?.files ?? null);
}

function onPick() {
  fileInput.value?.click();
}

function onChange(event: Event) {
  const target = event.target as HTMLInputElement;
  void addFiles(target.files);
  target.value = '';
}

function remove(index: number) {
  const attachment = props.modelValue[index];
  if (!attachment) return;
  const next = [...props.modelValue];
  next.splice(index, 1);
  emit('update:modelValue', next);
  if (attachment.inline) emit('remove-inline', attachment);
}

function moveInline(index: number) {
  const attachment = props.modelValue[index];
  if (!attachment || !attachment.mime_type.startsWith('image/')) return;
  const contentId = attachment.content_id || `satchel-${crypto.randomUUID()}@inline`;
  const inlineAttachment: BackendOutgoingAttachment = {
    ...attachment,
    content_id: contentId,
    inline: true,
  };
  const next = [...props.modelValue];
  next[index] = inlineAttachment;
  emit('update:modelValue', next);
  emit('insert-inline', inlineAttachment);
}

function moveToAttachment(index: number) {
  const attachment = props.modelValue[index];
  if (!attachment?.inline) return;
  const next = [...props.modelValue];
  next[index] = {
    ...attachment,
    content_id: null,
    inline: false,
  };
  emit('update:modelValue', next);
  emit('move-to-attachment', attachment);
}

defineExpose({ openPicker: onPick });
</script>

<template>
  <div
    v-show="props.showEmptyState || visibleAttachments.length > 0 || Boolean(errorText)"
    class="zone"
    :class="{ over: isOver }"
    @dragover.prevent="isOver = true"
    @dragleave.prevent="isOver = false"
    @drop="onDrop"
  >
    <div v-if="visibleAttachments.length === 0" class="empty">
      <span>{{ $t('mail.attachment.dropHere') }}</span>
      <AppButton class="link" @click="onPick" variant="link" size="sm">{{ $t('mail.attachment.chooseFile') }}</AppButton>
    </div>
    <!-- Chips, not full-width rows: four forwarded mails used to take most of
         the composer's height and left two lines to write in. -->
    <ul v-else class="list">
      <li v-for="entry in visibleAttachments" :key="entry.index" class="item" :title="entry.attachment.filename">
        <span class="kind" aria-hidden="true">{{ fileKind(entry.attachment.filename) }}</span>
        <span class="name">{{ entry.attachment.filename }}</span>
        <span class="size">{{ formatBytes(encodedSize(entry.attachment)) }}</span>
        <AppButton
          v-if="props.inlineEnabled && entry.attachment.inline"
          class="mode"
          :title="$t('mail.attachment.useAsAttachment')"
          @click="moveToAttachment(entry.index)"
          variant="link"
          size="sm"
        >{{ $t('mail.attachment.inline') }}</AppButton>
        <AppButton
          v-else-if="props.inlineEnabled && entry.attachment.mime_type.startsWith('image/')"
          class="inline"
          :title="$t('mail.attachment.insertAtCursor')"
          @click="moveInline(entry.index)"
          variant="link"
          size="sm"
        >{{ $t('mail.attachment.inline') }}</AppButton>
        <IconButton
          class="remove"
          size="sm"
          tone="danger"
          :title="$t('mail.attachment.removeNamed', { name: entry.attachment.filename })"
          @click="remove(entry.index)"
        >
          <AppIcon size="sm"><path d="m4.5 4.5 7 7M11.5 4.5l-7 7" /></AppIcon>
        </IconButton>
      </li>
      <li class="add-item">
        <AppButton class="add" @click="onPick" variant="link" size="sm">{{ $t('mail.attachment.moreFiles') }}</AppButton>
      </li>
    </ul>
    <p v-if="errorText" class="error">{{ errorText }}</p>
    <input ref="fileInput" type="file" multiple style="display: none" @change="onChange" />
  </div>
</template>

<style scoped>
.zone {
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-md);
  padding: 12px;
  background: color-mix(in srgb, var(--bg-surface) 92%, transparent);
  transition: border-color var(--duration-normal) var(--ease-out), background var(--duration-normal) var(--ease-out);
}

.zone.over {
  border-color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-surface));
}

.empty {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

/* A grid of equal chips: two side by side in the floating composer, more on
   a wide page, each as wide as its column allows. */
.list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 13rem), 1fr));
  align-items: center;
  gap: var(--space-xs);
  margin: 0;
  padding: 0;
  list-style: none;
}

.item {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  min-width: 0;
  height: 30px;
  padding: 0 2px 0 4px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  font-size: var(--font-size-sm);
}

.kind {
  flex: none;
  min-width: 30px;
  padding: 1px 4px;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-semibold);
  letter-spacing: 0.02em;
  text-align: center;
}

.name {
  flex: 1 1 auto;
  min-width: 0;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.size {
  flex: none;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.item :deep(.inline),
.item :deep(.mode) {
  flex: none;
  font-size: var(--font-size-xs);
}

.item :deep(.mode) {
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-primary) 10%, transparent);
  padding-inline: 7px;
}

.remove { flex: none; }

.add-item { display: inline-flex; }

.error {
  margin: 6px 0 0;
  color: var(--accent-red);
  font-size: var(--font-size-xs);
}
</style>
