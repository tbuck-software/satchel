<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { PDFDocumentProxy, RenderTask } from 'pdfjs-dist';
import AppButton from '@/components/ui/AppButton.vue';
import { loadPdfDocument } from '@/utils/pdfDocument';

const props = defineProps<{ bytes: Uint8Array }>();
const emit = defineEmits<{ failed: [] }>();
const container = ref<HTMLElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const pageNumber = ref(1);
const pageCount = ref(0);
const rendering = ref(true);
let document: PDFDocumentProxy | null = null;
let task: ReturnType<typeof loadPdfDocument> | null = null;
let renderTask: RenderTask | null = null;
let generation = 0;
let observer: ResizeObserver | null = null;

async function renderPage() {
  const token = ++generation;
  renderTask?.cancel();
  renderTask = null;
  if (!document || !container.value || !canvas.value) return;
  rendering.value = true;
  try {
    const page = await document.getPage(pageNumber.value);
    if (token !== generation || !canvas.value || !container.value) return;
    const natural = page.getViewport({ scale: 1 });
    const style = getComputedStyle(container.value);
    const padding = (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0);
    const width = Math.max(1, container.value.clientWidth - padding);
    const scale = Math.min(width / natural.width, 2);
    // Bound canvas memory on high-DPI displays and unusually large pages.
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    const viewport = page.getViewport({ scale: Math.min(scale * ratio, 4096 / Math.max(natural.width, natural.height)) });
    const target = canvas.value;
    target.width = Math.ceil(viewport.width);
    target.height = Math.ceil(viewport.height);
    target.style.width = `${viewport.width / ratio}px`;
    target.style.height = `${viewport.height / ratio}px`;
    renderTask = page.render({ canvas: target, viewport });
    await renderTask.promise;
    if (token === generation) rendering.value = false;
    page.cleanup();
  } catch {
    if (token === generation) emit('failed');
  }
}

watch(() => props.bytes, async (bytes, _previous, onCleanup) => {
  let loadingTask: ReturnType<typeof loadPdfDocument>;
  try {
    loadingTask = loadPdfDocument(bytes);
  } catch {
    emit('failed');
    return;
  }
  task = loadingTask;
  document = null;
  pageCount.value = 0;
  pageNumber.value = 1;
  rendering.value = true;
  onCleanup(() => {
    ++generation;
    renderTask?.cancel();
    void loadingTask.destroy();
  });
  try {
    const loaded = await loadingTask.promise;
    if (task !== loadingTask) return;
    document = loaded;
    pageCount.value = loaded.numPages;
    await renderPage();
  } catch {
    if (task === loadingTask) emit('failed');
  }
}, { immediate: true });

watch(pageNumber, () => { void renderPage(); });
onMounted(() => {
  observer = new ResizeObserver(() => { void renderPage(); });
  if (container.value) observer.observe(container.value);
});
onBeforeUnmount(() => {
  task = null;
  document = null;
  observer?.disconnect();
});
</script>

<template>
  <section class="pdf-preview" :aria-label="$t('mail.attachment.pdfPreview')">
    <nav v-if="pageCount > 1" class="pdf-controls" :aria-label="$t('mail.attachment.pdfPages')">
      <AppButton variant="ghost" size="sm" :disabled="pageNumber <= 1" @click="pageNumber--">
        {{ $t('mail.attachment.pdfPrevious') }}
      </AppButton>
      <span aria-live="polite">{{ $t('mail.attachment.pdfPage', { page: pageNumber, total: pageCount }) }}</span>
      <AppButton variant="ghost" size="sm" :disabled="pageNumber >= pageCount" @click="pageNumber++">
        {{ $t('mail.attachment.pdfNext') }}
      </AppButton>
    </nav>
    <div ref="container" class="pdf-page" :aria-busy="rendering">
      <p v-if="rendering" class="pdf-loading" role="status">{{ $t('mail.attachment.loading') }}</p>
      <canvas ref="canvas" v-show="!rendering" role="img" :aria-label="$t('mail.attachment.pdfPage', { page: pageNumber, total: pageCount })" />
    </div>
  </section>
</template>

<style scoped>
.pdf-preview { display: flex; flex-direction: column; width: 100%; height: 100%; min-height: 0; }
.pdf-controls { display: flex; align-items: center; justify-content: center; gap: var(--space-md); padding: var(--space-sm); color: var(--text-primary); }
.pdf-page { flex: 1; min-height: 0; overflow: auto; text-align: center; padding: var(--space-lg); }
.pdf-page canvas { max-width: 100%; height: auto; margin: 0 auto; }
.pdf-loading { color: var(--text-secondary); }
</style>
