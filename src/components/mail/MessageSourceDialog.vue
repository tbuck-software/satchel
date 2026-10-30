<script setup lang="ts">
import IconButton from '@/components/ui/IconButton.vue';
import { useDismissable } from '@/composables/useDismissable';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useToast } from '@/composables/useToast';
import AppButton from '@/components/ui/AppButton.vue';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import ModalShell from '@/components/ui/ModalShell.vue';
import { useMessagesStore } from '@/stores/messages';
import { formatBytes } from '@/utils/formatBytes';

const props = defineProps<{
  open: boolean;
  messageId: string | null;
}>();

const emit = defineEmits<{ close: [] }>();

type Tab = 'source' | 'headers' | 'text' | 'html';

const toast = useToast();
const { t } = useI18n();
const messages = useMessagesStore();
const loading = ref(false);
const errorText = ref('');
const activeTab = ref<Tab>('source');
const rawSource = ref('');
const exporting = ref(false);
const rawSourceStatus = ref<'loaded' | 'unavailable' | 'unsupported'>('unsupported');
const rawHeaders = ref('');
const bodyText = ref('');
const bodyHtml = ref('');
const subject = ref('');
const rawSize = ref<number | null>(null);
let loadRequest: symbol | null = null;

const tabs = computed(() => [
  { id: 'source' as Tab, label: t('mail.source.tabSource'), available: rawSource.value.length > 0 },
  { id: 'headers' as Tab, label: t('mail.source.tabHeaders'), available: rawHeaders.value.length > 0 },
  { id: 'text' as Tab, label: t('mail.source.tabText'), available: bodyText.value.length > 0 },
  { id: 'html' as Tab, label: t('mail.source.tabHtml'), available: bodyHtml.value.length > 0 },
]);

const activeContent = computed(() => {
  if (activeTab.value === 'source') return rawSource.value;
  if (activeTab.value === 'headers') return rawHeaders.value;
  if (activeTab.value === 'text') return bodyText.value;
  return bodyHtml.value;
});
const rawSourceNotice = computed(() => {
  if (rawSource.value || rawSourceStatus.value === 'loaded') return '';
  return rawSourceStatus.value === 'unavailable'
    ? t('mail.source.remoteUnavailable')
    : t('mail.source.providerUnsupported');
});

const sizeLabel = computed(() => (rawSize.value ? formatBytes(rawSize.value) : ''));

function clearSource() {
  loading.value = false;
  errorText.value = '';
  subject.value = '';
  rawSize.value = null;
  activeTab.value = 'source';
  rawSource.value = '';
  rawSourceStatus.value = 'unsupported';
  rawHeaders.value = '';
  bodyText.value = '';
  bodyHtml.value = '';
}

async function load(messageId: string) {
  const request = Symbol();
  loadRequest = request;
  clearSource();
  loading.value = true;

  try {
    const source = await messages.loadMessageSource(messageId);
    if (loadRequest !== request) return;
    if (!source) {
      errorText.value = t('mail.source.desktopOnly');
      return;
    }
    subject.value = source.subject ?? '';
    rawSource.value = source.raw_source ?? '';
    rawSourceStatus.value = source.raw_source_status ?? 'unsupported';
    rawHeaders.value = source.raw_headers ?? '';
    bodyText.value = source.body_text ?? '';
    bodyHtml.value = source.body_html ?? '';
    rawSize.value = source.raw_size ?? null;
    activeTab.value = tabs.value.find((tab) => tab.available)?.id ?? 'source';
  } catch (err) {
    if (loadRequest !== request) return;
    errorText.value = err instanceof Error ? err.message : t('mail.source.loadFailed');
  } finally {
    if (loadRequest === request) loading.value = false;
  }
}

watch(
  () => [props.open, props.messageId] as const,
  ([open, messageId]) => {
    if (open && messageId) void load(messageId);
    else {
      loadRequest = null;
      clearSource();
    }
  },
  { immediate: true, flush: 'sync' },
);

onBeforeUnmount(() => { loadRequest = null; });

async function exportMessage() {
  if (!props.messageId || exporting.value) return;
  exporting.value = true;
  try {
    if (await messages.exportMessage(props.messageId, subject.value)) {
      toast.success(t('mail.source.exported'));
    }
  } catch (error) {
    toast.error(backendErrorLabel(error));
  } finally {
    exporting.value = false;
  }
}

async function copyToClipboard() {
  try {
    await navigator.clipboard.writeText(activeContent.value);
    toast.success(t('mail.source.copied'));
  } catch {
    toast.error(t('mail.source.copyFailed'));
  }
}

// Back closes this before whatever it was opened from.
useDismissable(() => props.open, () => emit('close'));
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <ModalShell
        v-if="props.open"
        :label="$t('mail.source.dialogLabel')"
        width="900px"
        height="760px"
        @dismiss="emit('close')"
      >
        <div class="source-body">
          <header class="panel-header">
            <div class="titles">
              <strong>{{ $t('mail.source.title') }}</strong>
              <span v-if="subject" class="subject">{{ subject }}</span>
            </div>
            <span v-if="sizeLabel" class="size">{{ sizeLabel }}</span>
            <IconButton class="close" :title="$t('common.close')" @click="emit('close')">✕</IconButton>
          </header>

          <nav v-if="!loading && !errorText" class="tabs" role="tablist">
            <AppButton
              v-for="tab in tabs"
              :key="tab.id"
              role="tab"
              :aria-selected="activeTab === tab.id"
              :disabled="!tab.available"
              :class="{ active: activeTab === tab.id }"
              @click="activeTab = tab.id"
              variant="ghost"
              size="sm"
            >
              {{ tab.label }}
            </AppButton>
          </nav>

          <p v-if="!loading && !errorText && rawSourceNotice" class="source-notice">
            {{ rawSourceNotice }}
          </p>

          <p v-if="loading" class="state">{{ $t('common.loading') }}</p>
          <p v-else-if="errorText" class="state state--error" role="alert">{{ errorText }}</p>
          <p v-else-if="!activeContent" class="state">
            {{ $t('mail.source.empty') }}
          </p>
          <pre v-else class="source" tabindex="0">{{ activeContent }}</pre>

          <footer class="panel-footer">
            <AppButton variant="soft" size="sm" :disabled="loading || exporting || !props.messageId" @click="void exportMessage()">
              {{ exporting ? $t('mail.source.exporting') : $t('mail.source.exportEml') }}
            </AppButton>
            <AppButton v-if="activeContent" variant="soft" size="sm" @click="void copyToClipboard()">
              {{ $t('common.copy') }}
            </AppButton>
            <AppButton variant="primary" size="sm" @click="emit('close')">{{ $t('common.close') }}</AppButton>
          </footer>
        </div>
      </ModalShell>
    </Transition>
  </Teleport>
</template>

<style scoped>
.source-body {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr) auto;
}

.panel-header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-color-muted);
}

.titles {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
  flex: 1;
}

.titles strong {
  font-size: var(--font-size-md);
  font-weight: var(--weight-bold);
}

.subject {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.size {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.tabs {
  display: flex;
  gap: 4px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color-muted);
}

.tabs button {
  border: 0;
  background: transparent;
  border-radius: var(--radius-xs);
  padding: 5px 10px;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-sm);
  cursor: pointer;
}

.tabs button.active {
  background: var(--bg-hover);
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
}

.tabs button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.source {
  margin: 0;
  padding: 14px 16px;
  overflow: auto;
  font-family: var(--font-mono, monospace);
  font-size: var(--font-size-xs);
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-primary);
  background: var(--bg-inset, var(--bg-base));
}

.source-notice {
  margin: 0;
  border-bottom: var(--border-muted);
  padding: var(--space-sm) var(--space-lg);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.state {
  margin: 0;
  padding: 28px 16px;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  text-align: center;
}

.state--error {
  color: var(--accent-red);
}

.panel-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 10px 16px;
  border-top: 1px solid var(--border-color-muted);
}
</style>
