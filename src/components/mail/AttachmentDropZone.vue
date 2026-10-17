<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { BackendOutgoingAttachment } from '@/types/composer';
import { formatBytes } from '@/utils/formatBytes';

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

const MAX_BYTES = 25_000_000;
/**
 * Total budget across all attachments. Most providers reject well before this
 * (Gmail ~25 MB, Graph ~35 MB) and base64 adds ~33 % on the wire, so refusing
 * here is far kinder than a failed send after a long upload.
 */
const MAX_TOTAL_BYTES = 20_000_000;

const visibleAttachments = computed(() =>
  props.modelValue.map((attachment, index) => ({ attachment, index })),
);

/** Approximate original byte size of an already-encoded attachment. */
function encodedSize(attachment: BackendOutgoingAttachment): number {
  return Math.floor((attachment.content_base64.length * 3) / 4);
}

const usedBytes = computed(() =>
  props.modelValue.reduce((sum, attachment) => sum + encodedSize(attachment), 0),
);

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
      <button type="button" class="link" @click="onPick">{{ $t('mail.attachment.chooseFile') }}</button>
    </div>
    <div v-else class="list">
      <div v-for="entry in visibleAttachments" :key="entry.index" class="item">
        <span class="name">{{ entry.attachment.filename }}</span>
        <span class="size">{{ formatBytes(encodedSize(entry.attachment)) }}</span>
        <span v-if="entry.attachment.inline" class="mode">{{ $t('mail.attachment.inline') }}</span>
        <button
          v-if="props.inlineEnabled && entry.attachment.inline"
          type="button"
          class="inline"
          :title="$t('mail.attachment.useAsAttachment')"
          @click="moveToAttachment(entry.index)"
        >{{ $t('mail.attachment.attachment') }}</button>
        <button
          v-else-if="props.inlineEnabled && entry.attachment.mime_type.startsWith('image/')"
          type="button"
          class="inline"
          :title="$t('mail.attachment.insertAtCursor')"
          @click="moveInline(entry.index)"
        >{{ $t('mail.attachment.inline') }}</button>
        <button type="button" class="remove" @click="remove(entry.index)">{{ $t('common.remove') }}</button>
      </div>
      <button type="button" class="add" @click="onPick">{{ $t('mail.attachment.moreFiles') }}</button>
    </div>
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

.link {
  background: transparent;
  border: 0;
  color: var(--accent-primary);
  cursor: pointer;
  font-size: inherit;
  padding: 0;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.item {
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--bg-raised);
  padding: 6px 10px;
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
}

.name {
  flex: 1;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.size {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.mode {
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-primary) 10%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-3xs);
  line-height: 18px;
  padding: 0 7px;
}

.remove {
  background: transparent;
  border: 0;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-xs);
}
.inline {
  border: 0;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
  color: var(--accent-primary);
  cursor: pointer;
  font-size: var(--font-size-xs);
  padding: 4px 7px;
}
.inline:hover {
  background: color-mix(in srgb, var(--accent-primary) 20%, transparent);
}
.remove:hover {
  color: var(--accent-red);
}

.add {
  background: transparent;
  border: 1px dashed var(--border-color);
  color: var(--text-secondary);
  padding: 6px 12px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: var(--font-size-sm);
  align-self: flex-start;
}
.add:hover {
  color: var(--text-primary);
  border-color: var(--text-secondary);
}

.error {
  margin: 6px 0 0;
  color: var(--accent-red);
  font-size: var(--font-size-xs);
}
</style>
