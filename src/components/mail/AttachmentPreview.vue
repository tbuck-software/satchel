<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import IconButton from '@/components/ui/IconButton.vue';
import { useDismissable } from '@/composables/useDismissable';
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { BackendDownloadAttachmentResult } from '@/types/messages';
import { useToast } from '@/composables/useToast';
import { useAttachmentActions } from '@/composables/useAttachmentActions';
import { attachmentBytes } from '@/utils/attachmentBytes';
import ModalShell from '@/components/ui/ModalShell.vue';

const { t } = useI18n();
const toast = useToast();
const attachmentActions = useAttachmentActions();

const props = defineProps<{
  attachmentId: string;
  filename?: string | null;
  mimeType?: string | null;
}>();

const emit = defineEmits<{
  (e: 'close'): void;
}>();

const data = ref<BackendDownloadAttachmentResult | null>(null);
const error = ref('');
const loading = ref(true);
const openingExternal = ref(false);
const objectUrl = ref('');
// Fetched once: above 4 MiB the payload arrives as a cache path, not base64,
// and both the preview and the text view want the same bytes.
const bytes = ref<Uint8Array | null>(null);
const pdfFailed = ref(false);
let pdfLoadTimer: number | null = null;

function releaseObjectUrl() {
  if (objectUrl.value) URL.revokeObjectURL(objectUrl.value);
  objectUrl.value = '';
}

function prepareObjectUrl(result: BackendDownloadAttachmentResult, payload: Uint8Array) {
  if (!result.mime_type.startsWith('image/') && result.mime_type !== 'application/pdf') return;
  releaseObjectUrl();
  // `Uint8Array<ArrayBufferLike>` is not a `BlobPart`; the slice narrows it.
  const view = payload.slice();
  objectUrl.value = URL.createObjectURL(
    new Blob([view.buffer as ArrayBuffer], { type: result.mime_type }),
  );
  if (result.mime_type === 'application/pdf') {
    pdfLoadTimer = window.setTimeout(() => {
      pdfFailed.value = true;
    }, 8_000);
  }
}

onMounted(async () => {
  try {
    data.value = await attachmentActions.downloadAttachment(props.attachmentId);
    bytes.value = await attachmentBytes(data.value);
    prepareObjectUrl(data.value, bytes.value);
  } catch (err) {
    error.value = backendErrorLabel(err);
  } finally {
    loading.value = false;
  }
});

onBeforeUnmount(() => {
  if (pdfLoadTimer !== null) window.clearTimeout(pdfLoadTimer);
  releaseObjectUrl();
  bytes.value = null;
});

const mime = computed(() => data.value?.mime_type || props.mimeType || '');
const isImage = computed(() => mime.value.startsWith('image/'));
const isPdf = computed(() => mime.value === 'application/pdf');
const isText = computed(
  () => mime.value.startsWith('text/') || mime.value === 'application/json',
);
const decodedText = computed(() => {
  if (!isText.value || !bytes.value) return '';
  try {
    return new TextDecoder().decode(bytes.value);
  } catch {
    return '';
  }
});

/**
 * Forwarded mail arrives as a `message/rfc822` part. The viewer window reads it
 * from the attachment cache, so it needs the part on disk — which it is, right
 * after the download that filled this preview.
 */
const nestedMessagePath = computed(() => attachmentActions.nestedMessagePath(
  data.value,
  props.filename,
  props.mimeType,
));

async function openInViewer() {
  const path = nestedMessagePath.value;
  if (!path) return;
  try {
    await attachmentActions.openMessageFile(path);
    emit('close');
  } catch (err) {
    toast.error(t('mail.attachment.openMessageFailed', { error: String(err) }));
  }
}

async function saveAs() {
  if (!data.value) return;
  await attachmentActions.saveDownloadedAttachment(
    data.value,
    props.filename || t('mail.attachment.fallbackName'),
  );
}

async function openWithSystem() {
  if (openingExternal.value) return;
  openingExternal.value = true;
  try {
    await attachmentActions.openAttachmentDefault(props.attachmentId);
  } catch (err) {
    toast.error(t('mail.attachment.openFailed', { error: backendErrorLabel(err) }));
  } finally {
    openingExternal.value = false;
  }
}

function markPdfLoaded() {
  if (pdfLoadTimer !== null) window.clearTimeout(pdfLoadTimer);
  pdfLoadTimer = null;
}

function markPdfFailed() {
  if (pdfLoadTimer !== null) window.clearTimeout(pdfLoadTimer);
  pdfLoadTimer = null;
  pdfFailed.value = true;
}

// Back closes this before whatever it was opened from.
useDismissable(() => true, () => emit('close'));
</script>

<template>
  <ModalShell
    :label="$t('mail.attachment.previewLabel')"
    width="960px"
    height="900px"
    @dismiss="emit('close')"
  >
    <div class="preview-body">
      <header>
        <span class="title">{{ data?.filename || filename || $t('mail.attachment.fallbackName') }}</span>
        <div class="actions">
          <AppButton
            v-if="nestedMessagePath"
            class="ghost"
            @click="openInViewer"
            variant="ghost"
            size="sm"
          >
            {{ $t('mail.attachment.openInWindow') }}
          </AppButton>
          <AppButton
            v-if="data"
            class="ghost"
            :disabled="openingExternal"
            @click="openWithSystem"
            variant="ghost"
            size="sm"
          >
            {{ $t('mail.attachment.openWithDefault') }}
          </AppButton>
          <AppButton v-if="data" class="ghost" @click="saveAs" variant="ghost" size="sm">
            {{ $t('mail.attachment.saveEllipsis') }}
          </AppButton>
          <IconButton :title="$t('common.close')" class="close" @click="emit('close')">×</IconButton>
        </div>
      </header>
      <div class="content">
        <div v-if="loading" class="state">{{ $t('mail.attachment.loading') }}</div>
        <div v-else-if="error" class="state error">{{ error }}</div>
        <img v-else-if="isImage" :src="objectUrl" :alt="data?.filename" />
        <iframe
          v-else-if="isPdf && !pdfFailed"
          :src="objectUrl"
          :title="$t('mail.attachment.pdfPreview')"
          @load="markPdfLoaded"
          @error="markPdfFailed"
        />
        <div v-else-if="isPdf" class="state error">
          <p>{{ $t('mail.attachment.pdfFailed') }}</p>
          <AppButton class="primary" @click="openWithSystem" variant="primary" size="sm">
            {{ $t('mail.attachment.openWithDefault') }}
          </AppButton>
        </div>
        <pre v-else-if="isText">{{ decodedText }}</pre>
        <div v-else class="state">
          <i18n-t keypath="mail.attachment.previewUnavailable" tag="p">
            <template #mime><code>{{ mime }}</code></template>
          </i18n-t>
          <AppButton
            v-if="nestedMessagePath"
            class="primary"
            @click="openInViewer"
            variant="primary"
            size="sm"
          >
            {{ $t('mail.attachment.openInWindow') }}
          </AppButton>
          <AppButton class="primary" @click="saveAs" variant="primary" size="sm">
            {{ $t('mail.attachment.saveEllipsis') }}
          </AppButton>
        </div>
      </div>
    </div>
  </ModalShell>
</template>

<style scoped>
.preview-body {
  flex: 1; min-height: 0;
  display: flex; flex-direction: column; overflow: hidden;
}
header {
  padding: 10px 14px; border-bottom: 1px solid var(--border-color-muted);
  display: flex; align-items: center; justify-content: space-between;
}
.title { color: var(--text-primary); font-weight: var(--weight-medium); }
.actions { display: flex; gap: 8px; align-items: center; }
.content {
  flex: 1; overflow: auto; background: var(--bg-base);
  display: flex; align-items: center; justify-content: center;
}
.content img { max-width: 100%; max-height: 100%; object-fit: contain; }
.content iframe { width: 100%; height: 100%; border: 0; background: var(--email-canvas); }
.content pre {
  width: 100%; height: 100%; padding: 16px; margin: 0;
  font-family: var(--font-mono); font-size: var(--font-size-sm);
  color: var(--text-primary); overflow: auto; white-space: pre-wrap;
}
.state { color: var(--text-secondary); padding: 32px; text-align: center; }
.state.error { color: var(--accent-red); }
.primary { margin-top:12px; }
</style>
