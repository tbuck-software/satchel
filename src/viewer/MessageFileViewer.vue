<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import EmailRenderer from '@/components/mail/EmailRenderer.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import { useToast } from '@/composables/useToast';
import type { Attachment } from '@/types/mail';
// Submodules, not the barrel: `services/backend/index.ts` re-exports all
// seventeen domain modules, so one barrel import pulls the whole service
// surface into this window's static graph.
import { backendListAccounts, type BackendAccount } from '@/services/backend/accounts';
import { backendImportMbox } from '@/services/backend/backup';
import {
  backendOpenMessageFile,
  backendReadMessageFileAttachment,
  type BackendMessageFilePreview,
} from '@/services/backend/system';
import { saveAttachmentWithDialog } from '@/utils/saveAttachment';
import { translateBackendError } from '@/utils/backendErrors';
import { formatBytes } from '@/utils/formatBytes';

const { locale, t } = useI18n();
const toast = useToast();
// A viewer only needs notifications after an attachment or import action.
const ToastStack = defineAsyncComponent(() => import('@/components/ui/ToastStack.vue'));
const toastMounted = ref(false);
watch(() => toast.state.toasts.length, (count) => { if (count > 0) toastMounted.value = true; }, { immediate: true });

const preview = ref<BackendMessageFilePreview | null>(null);
const error = ref('');
const loading = ref(true);
const importing = ref(false);
const accounts = ref<BackendAccount[]>([]);
const importAccountId = ref('');

/**
 * The path arrives base64url-encoded in the query string. Rust encodes it that
 * way because mail file names carry spaces, umlauts and `#` as a matter of
 * course, and a percent-encoded round trip through the URL parser is a worse
 * bet than an alphabet that needs no escaping at all.
 */
function filePathFromLocation(): string {
  const encoded = new URLSearchParams(window.location.search).get('p');
  if (!encoded) return '';
  const padded = encoded.replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(padded);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

const filePath = filePathFromLocation();

/** Commands answer with a `backend.errors.*` key; anything else is shown bare. */
function errorText(err: unknown): string {
  return translateBackendError(err) ?? (err instanceof Error ? err.message : String(err));
}

/** Inline parts the renderer resolves `cid:` references against. */
const rendererAttachments = computed<Attachment[]>(() =>
  (preview.value?.attachments ?? []).map((attachment) => ({
    id: String(attachment.index),
    filename: attachment.filename,
    contentType: attachment.content_type,
    size: attachment.size,
    isInline: attachment.is_inline,
    contentId: attachment.content_id ?? null,
    dataUrl: attachment.data_url ?? null,
  })),
);

/** Inline parts without an embedded preview must remain available to save. */
const listedAttachments = computed(() =>
  (preview.value?.attachments ?? []).filter((attachment) => !attachment.is_inline || !attachment.data_url),
);

const recipients = computed(() => {
  const to = preview.value?.to_addresses ?? [];
  const cc = preview.value?.cc_addresses ?? [];
  return { to: to.join(', '), cc: cc.join(', ') };
});

const formattedDate = computed(() => {
  const raw = preview.value?.date_rfc3339;
  if (!raw) return '';
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) return raw;
  return parsed.toLocaleString(locale.value, { dateStyle: 'full', timeStyle: 'short' });
});

onMounted(async () => {
  if (!filePath) {
    error.value = t('viewer.noPath');
    loading.value = false;
    return;
  }
  document.title = filePath.split('/').pop() ?? t('viewer.untitled');
  try {
    preview.value = await backendOpenMessageFile(filePath);
  } catch (err) {
    error.value = errorText(err);
  } finally {
    loading.value = false;
  }

  // Deliberately after the body: the import target is a secondary concern and
  // must not delay the mail the user actually opened the file for.
  try {
    accounts.value = (await backendListAccounts()).filter((account) => account.kind === 'email');
    importAccountId.value = accounts.value[0]?.id ?? '';
  } catch {
    // No accounts reachable just means the import button stays hidden.
  }
});

async function saveAttachment(index: number, filename: string) {
  try {
    const data = await backendReadMessageFileAttachment(filePath, index);
    await saveAttachmentWithDialog(data.filename || filename, data.data_base64);
  } catch (err) {
    toast.error(t('mail.attachment.saveFailed', { error: errorText(err) }));
  }
}

async function importIntoAccount() {
  if (!importAccountId.value || importing.value) return;
  importing.value = true;
  try {
    const result = await backendImportMbox({ path: filePath, accountId: importAccountId.value });
    if (result.message_count > 0) {
      toast.success(t('viewer.imported'));
    } else {
      toast.info(t('viewer.alreadyImported'));
    }
  } catch (err) {
    toast.error(t('viewer.importFailed', { error: errorText(err) }));
  } finally {
    importing.value = false;
  }
}
</script>

<template>
  <main class="viewer">
    <EmptyState v-if="loading" :title="t('viewer.loading')">
      <template #icon><AppIcon size="lg"><rect x="2.6" y="3.8" width="10.8" height="8.4" rx="1.4" /><path d="m3 5 5 3.4L13 5" /></AppIcon></template>
    </EmptyState>
    <EmptyState
      v-else-if="error"
      :title="t('viewer.openFailed')"
      :description="error"
    >
      <template #icon><AppIcon size="lg"><rect x="2.6" y="3.8" width="10.8" height="8.4" rx="1.4" /><path d="m3 5 5 3.4L13 5" /></AppIcon></template>
    </EmptyState>
    <template v-else-if="preview">
      <header class="head">
        <h1 class="subject">{{ preview.subject }}</h1>
        <dl class="meta">
          <dt>{{ t('common.from') }}</dt>
          <dd>
            <span v-if="preview.from_name" class="name">{{ preview.from_name }}</span>
            <span class="address">{{ preview.from_address }}</span>
          </dd>
          <template v-if="recipients.to">
            <dt>{{ t('common.to') }}</dt>
            <dd>{{ recipients.to }}</dd>
          </template>
          <template v-if="recipients.cc">
            <dt>{{ t('viewer.cc') }}</dt>
            <dd>{{ recipients.cc }}</dd>
          </template>
          <template v-if="formattedDate">
            <dt>{{ t('common.date') }}</dt>
            <dd>{{ formattedDate }}</dd>
          </template>
        </dl>
        <div v-if="accounts.length > 0" class="actions">
          <select
            v-if="accounts.length > 1"
            v-model="importAccountId"
            class="account"
            :aria-label="t('viewer.importAccount')"
          >
            <option v-for="account in accounts" :key="account.id" :value="account.id">
              {{ account.email }}
            </option>
          </select>
          <AppButton
            variant="soft"
            size="sm"
            :disabled="importing"
            :title="t('viewer.importTitle')"
            @click="importIntoAccount"
          >
            {{ importing ? t('viewer.importing') : t('viewer.import') }}
          </AppButton>
        </div>
      </header>

      <section v-if="listedAttachments.length > 0" class="attachments">
        <h2>{{ t('viewer.attachments', listedAttachments.length) }}</h2>
        <ul>
          <li v-for="attachment in listedAttachments" :key="attachment.index">
            <span class="file">{{ attachment.filename }}</span>
            <span class="size">{{ formatBytes(attachment.size) }}</span>
            <AppButton
              variant="ghost"
              size="sm"
              @click="saveAttachment(attachment.index, attachment.filename)"
            >
              {{ t('mail.attachment.saveEllipsis') }}
            </AppButton>
          </li>
        </ul>
      </section>

      <article class="card">
        <EmailRenderer
          class="body"
          :html="preview.body_html ?? undefined"
          :text="preview.body_text ?? undefined"
          :attachments="rendererAttachments"
          :sender-email="preview.from_address"
        />
      </article>
    </template>
    <ToastStack v-if="toastMounted" />
  </main>
</template>

<style scoped>
/* The same metaphor as the client: an inset canvas with the message as a card
   on it. An .eml opened from the file manager is the same mail. */
.viewer {
  height: 100vh;
  overflow: auto;
  background: var(--bg-surface);
  color: var(--text-primary);
  padding: var(--space-xl) var(--space-xl) var(--space-2xl);
  display: flex;
  flex-direction: column;
  gap: var(--space-lg);
}
.card {
  flex: 1;
  padding: var(--space-lg);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
}
.head {
  display: flex;
  flex-direction: column;
  gap: var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
  padding-bottom: var(--space-lg);
}
.subject {
  font-size: var(--font-size-xl);
  font-weight: var(--weight-semibold);
  margin: 0;
  line-height: 1.3;
}
.meta {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-xs) var(--space-md);
  margin: 0;
  font-size: var(--font-size-sm);
}
.meta dt {
  color: var(--text-secondary);
}
.meta dd {
  margin: 0;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}
.meta .name {
  font-weight: var(--weight-medium);
  margin-right: var(--space-xs);
}
.meta .address {
  color: var(--text-secondary);
}
.actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}
.account {
  background: var(--bg-surface);
  color: var(--text-primary);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: var(--space-xs) var(--space-sm);
  font-size: var(--font-size-sm);
}
.attachments h2 {
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  font-weight: var(--weight-medium);
  margin: 0 0 var(--space-xs);
}
.attachments ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
}
.attachments li {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  background: var(--bg-surface);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  padding: var(--space-xs) var(--space-sm);
}
.attachments .file {
  flex: 1;
  overflow-wrap: anywhere;
}
.attachments .size {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
</style>
