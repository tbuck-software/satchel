<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useAliasesStore } from '@/stores/aliases';
import { useSignaturesStore } from '@/stores/signatures';
import { useToast } from '@/composables/useToast';
import { useCrudActions } from '@/composables/useCrudActions';
import RichTextEditor from '@/components/mail/RichTextEditor.vue';
import IsolatedEmailDocument from '@/components/mail/IsolatedEmailDocument.vue';
import type { InlineImagePayload } from '@/types/composer';
import type { BackendSignatureSuggestion } from '@/services/backend';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const { t } = useI18n();
const accountsStore = useAccountsStore();
const aliasesStore = useAliasesStore();
const signaturesStore = useSignaturesStore();

const accountId = ref<string>('');
const editingId = ref<string | null>(null);
const name = ref('');
const bodyText = ref('');
const bodyHtml = ref('');
const shortBodyText = ref('');
const shortBodyHtml = ref('');
const assets = ref<Array<{
  filename: string;
  mimeType: string;
  contentId: string;
  contentBase64: string;
}>>([]);
const isDefault = ref(false);
const identityEmail = ref('');
const firstContactMode = ref<'always_full' | 'full_then_short' | 'always_short'>('always_full');
const adoptedSuggestionId = ref<string | null>(null);
const status = ref('');
const editorRef = ref<HTMLElement | null>(null);
const nameInputRef = ref<HTMLInputElement | null>(null);
const longEditorRef = ref<InstanceType<typeof RichTextEditor> | null>(null);
const shortEditorRef = ref<InstanceType<typeof RichTextEditor> | null>(null);
const toast = useToast();
const crud = useCrudActions();

/**
 * The WYSIWYG editors work on display HTML (data: URLs); stored signatures
 * reference their assets as cid:. Hydrate on load, serialize on save — the
 * same convention the composer uses for inline images.
 */
function assetDataUrl(asset: { mimeType: string; contentBase64: string }) {
  return `data:${asset.mimeType};base64,${asset.contentBase64}`;
}

function hydrateHtml(html: string) {
  let hydrated = html;
  for (const asset of assets.value) {
    hydrated = hydrated.split(`cid:${asset.contentId}`).join(assetDataUrl(asset));
  }
  return hydrated;
}

function serializeHtml(html: string) {
  let serialized = html;
  for (const asset of assets.value) {
    serialized = serialized.split(assetDataUrl(asset)).join(`cid:${asset.contentId}`);
  }
  return serialized;
}

function textToHtml(text: string) {
  if (!text.trim()) return '';
  return `<p>${escapeHtml(text).replace(/\n/g, '<br>')}</p>`;
}

onMounted(async () => {
  await Promise.all([accountsStore.load(), aliasesStore.load(), signaturesStore.load()]);
  accountId.value = accountsStore.activeAccountId ?? accountsStore.accounts[0]?.id ?? '';
  identityEmail.value = identityOptions.value[0]?.email ?? '';
  void discoverForIdentities();
});

const accountSignatures = computed(() =>
  accountId.value ? signaturesStore.byAccount.get(accountId.value) ?? [] : [],
);
const activeAccount = computed(() =>
  accountsStore.accounts.find((account) => account.id === accountId.value),
);
const identityOptions = computed(() => {
  if (!activeAccount.value) return [];
  return [
    { email: activeAccount.value.email, label: activeAccount.value.email },
    ...aliasesStore.aliasesFor(activeAccount.value.id).map((alias) => ({
      email: alias.email,
      label: alias.display_name ? `${alias.display_name} <${alias.email}>` : alias.email,
    })),
  ];
});
const accountSuggestions = computed(() =>
  signaturesStore.suggestions.filter((suggestion) => suggestion.account_id === accountId.value),
);

watch(accountId, () => {
  identityEmail.value = identityOptions.value[0]?.email ?? '';
  reset();
  void discoverForIdentities();
});

watch(identityEmail, () => {
  const assignment = signaturesStore.assignmentFor(accountId.value, identityEmail.value);
  firstContactMode.value = assignment?.first_contact_mode ?? 'always_full';
});

function reset() {
  editingId.value = null;
  name.value = '';
  bodyText.value = '';
  bodyHtml.value = '';
  shortBodyText.value = '';
  shortBodyHtml.value = '';
  assets.value = [];
  isDefault.value = accountSignatures.value.length === 0;
  adoptedSuggestionId.value = null;
}

function edit(id: string) {
  const sig = signaturesStore.items.find((s) => s.id === id);
  if (!sig) return;
  editingId.value = id;
  name.value = sig.name;
  bodyText.value = sig.body_text;
  shortBodyText.value = sig.short_body_text ?? '';
  assets.value = (sig.assets ?? []).map((asset) => ({
    filename: asset.filename,
    mimeType: asset.mime_type,
    contentId: asset.content_id,
    contentBase64: asset.content_base64,
  }));
  // Always hand the editors real HTML so switching signatures replaces the
  // previous content even when the new one is plain text.
  bodyHtml.value = sig.body_html ? hydrateHtml(sig.body_html) : textToHtml(sig.body_text);
  shortBodyHtml.value = sig.short_body_html
    ? hydrateHtml(sig.short_body_html)
    : textToHtml(sig.short_body_text ?? '');
  isDefault.value = sig.is_default === 1;
}

async function save() {
  if (!accountId.value || !name.value.trim()) {
    status.value = t('settings.signatures.nameRequired');
    return;
  }
  try {
    const longHtml = normalizedHtml(serializeHtml(bodyHtml.value), bodyText.value);
    const shortHtml = normalizedHtml(serializeHtml(shortBodyHtml.value), shortBodyText.value);
    const referencedAssets = assets.value.filter(
      (asset) =>
        (longHtml ?? '').includes(`cid:${asset.contentId}`)
        || (shortHtml ?? '').includes(`cid:${asset.contentId}`),
    );
    const signature = await signaturesStore.upsert({
      id: editingId.value,
      accountId: accountId.value,
      name: name.value.trim(),
      bodyText: bodyText.value,
      bodyHtml: longHtml,
      shortBodyText: shortBodyText.value,
      shortBodyHtml: shortHtml,
      assets: referencedAssets,
      isDefault: isDefault.value,
    });
    if (identityEmail.value) {
      await signaturesStore.assign({
        accountId: accountId.value,
        identityEmail: identityEmail.value,
        signatureId: signature.id,
        firstContactMode: firstContactMode.value,
      });
    }
    if (adoptedSuggestionId.value) {
      await signaturesStore.setSuggestionStatus(adoptedSuggestionId.value, 'accepted');
    }
    status.value = t('settings.signatures.saved');
    reset();
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function remove(id: string) {
  const signature = signaturesStore.items.find((s) => s.id === id);
  if (!signature) return;
  const removed = await crud.removeWithUndo({
    id,
    name: signature.name,
    remove: () => signaturesStore.remove(id),
    restore: (snapshot) => signaturesStore.restore(snapshot),
  });
  if (removed && editingId.value === id) reset();
}

const discovering = ref(false);

async function discoverForIdentities() {
  if (!accountId.value || discovering.value) return;
  discovering.value = true;
  try {
    await Promise.allSettled(
      identityOptions.value.map((identity) =>
        signaturesStore.discover(accountId.value, identity.email),
      ),
    );
  } finally {
    discovering.value = false;
  }
}

function adoptSuggestion(suggestion: BackendSignatureSuggestion) {
  editingId.value = null;
  identityEmail.value = suggestion.identity_email;
  name.value = t('settings.signatures.suggestionName', { email: suggestion.identity_email });
  bodyText.value = suggestion.body_text;
  bodyHtml.value = suggestion.body_html || textToHtml(suggestion.body_text);
  shortBodyText.value = '';
  shortBodyHtml.value = '';
  assets.value = [];
  adoptedSuggestionId.value = suggestion.id;
  status.value = t('settings.signatures.suggestionLoaded');
  // The editor sits below the fold — without this the click looks like a no-op.
  void nextTick(() => {
    editorRef.value?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    nameInputRef.value?.focus();
    nameInputRef.value?.select();
  });
}

async function dismissSuggestion(suggestion: BackendSignatureSuggestion) {
  await signaturesStore.setSuggestionStatus(suggestion.id, 'dismissed');
  if (adoptedSuggestionId.value === suggestion.id) adoptedSuggestionId.value = null;
  toast.info(t('settings.signatures.suggestionDismissed'), {
    action: {
      label: t('settings.signatures.undo'),
      handler: () => void signaturesStore.restoreSuggestion(suggestion),
    },
  });
}

async function addImages(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files ?? []);
  for (const file of files) {
    if (!file.type.startsWith('image/')) continue;
    const contentBase64 = await fileToBase64(file);
    const asset = {
      filename: file.name,
      mimeType: file.type,
      contentId: `signature-${crypto.randomUUID()}@satchel`,
      contentBase64,
    };
    assets.value.push(asset);
    longEditorRef.value?.insertAttachmentImage({
      filename: asset.filename,
      mime_type: asset.mimeType,
      content_base64: asset.contentBase64,
    });
  }
  input.value = '';
}

/** Images pasted or dropped straight into an editor become assets too. */
function onEditorImageAdded(image: InlineImagePayload) {
  if (assets.value.some((asset) => asset.contentBase64 === image.contentBase64)) return;
  assets.value.push({
    filename: image.filename,
    mimeType: image.mimeType,
    contentId: `signature-${crypto.randomUUID()}@satchel`,
    contentBase64: image.contentBase64,
  });
}

function normalizedHtml(html: string, text: string) {
  if (!text.trim() && !/<img/i.test(html)) return null;
  return html || null;
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? '').split(',')[1] ?? '');
    reader.onerror = () => reject(reader.error ?? new Error(t('settings.signatures.imageReadError')));
    reader.readAsDataURL(file);
  });
}

function removeAsset(contentId: string) {
  const asset = assets.value.find((item) => item.contentId === contentId);
  assets.value = assets.value.filter((item) => item.contentId !== contentId);
  if (!asset) return;
  const image = { mime_type: asset.mimeType, content_base64: asset.contentBase64 };
  longEditorRef.value?.removeAttachmentImage(image);
  shortEditorRef.value?.removeAttachmentImage(image);
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
</script>

<template>
  <section>
    <h3>{{ $t('settings.signatures.title') }}</h3>

    <label class="field">
      <span>{{ $t('settings.signatures.account') }}</span>
      <select v-model="accountId">
        <option v-for="acc in accountsStore.accounts" :key="acc.id" :value="acc.id">
          {{ acc.displayName }} &lt;{{ acc.email }}&gt;
        </option>
      </select>
    </label>

    <div :class="['suggestions', { 'suggestions--empty': accountSuggestions.length === 0 }]">
      <div class="suggestions-heading">
        <div>
          <h4>{{ accountSuggestions.length ? $t('settings.signatures.suggestionsFound') : $t('settings.signatures.suggestionsTitle') }}</h4>
          <p v-if="accountSuggestions.length">{{ $t('settings.signatures.suggestionsHint') }}</p>
          <p v-else-if="discovering">{{ $t('settings.signatures.discovering') }}</p>
          <p v-else>{{ $t('settings.signatures.noSuggestions') }}</p>
        </div>
        <button
          type="button"
          class="ghost"
          :disabled="discovering"
          @click="discoverForIdentities"
        >
          {{ discovering ? $t('settings.signatures.checking') : $t('settings.signatures.checkAgain') }}
        </button>
      </div>
      <article
        v-for="suggestion in accountSuggestions"
        :key="suggestion.id"
        :class="['suggestion', { 'suggestion--adopted': suggestion.id === adoptedSuggestionId }]"
      >
        <div>
          <strong>{{ suggestion.identity_email }}</strong>
          <!-- Rendered inside the sandboxed, CSP-enforced document frame —
               same isolation as reader content, never v-html. -->
          <IsolatedEmailDocument
            v-if="suggestion.body_html"
            class="suggestion-preview"
            :html="suggestion.body_html"
            :allow-remote-images="false"
          />
          <pre v-else>{{ suggestion.body_text }}</pre>
          <small>
            {{ $t('settings.signatures.foundIn', { count: suggestion.evidence_count }, suggestion.evidence_count) }}<template v-if="!suggestion.body_html">
              · {{ $t('settings.signatures.textOnly') }}</template
            >
          </small>
        </div>
        <div class="suggestion-actions">
          <button type="button" class="ghost" @click="dismissSuggestion(suggestion)">{{ $t('settings.signatures.ignore') }}</button>
          <button
            type="button"
            class="primary"
            :disabled="suggestion.id === adoptedSuggestionId"
            @click="adoptSuggestion(suggestion)"
          >
            {{ suggestion.id === adoptedSuggestionId ? $t('settings.signatures.loadedInEditor') : $t('settings.signatures.adopt') }}
          </button>
        </div>
      </article>
    </div>

    <ul v-if="accountSignatures.length > 0" class="list">
      <li v-for="sig in accountSignatures" :key="sig.id">
        <button type="button" class="row" @click="edit(sig.id)">
          <span class="name">
            {{ sig.name }}
            <span v-if="sig.is_default === 1" class="badge">{{ $t('common.default') }}</span>
          </span>
          <span class="preview">{{ sig.body_text.slice(0, 80) }}</span>
        </button>
        <button type="button" class="delete" @click="remove(sig.id)">{{ $t('common.delete') }}</button>
      </li>
    </ul>
    <p v-else class="empty">{{ $t('settings.signatures.emptyList') }}</p>

    <div ref="editorRef" class="editor">
      <h4>{{ editingId ? $t('settings.signatures.editTitle') : $t('settings.signatures.newTitle') }}</h4>
      <label class="field">
        <span>{{ $t('common.name') }}</span>
        <input ref="nameInputRef" v-model="name" :placeholder="$t('settings.signatures.namePlaceholder')" />
      </label>
      <div class="field">
        <span>{{ $t('settings.signatures.longVersion') }}</span>
        <RichTextEditor
          ref="longEditorRef"
          v-model="bodyText"
          :html="bodyHtml"
          min-height="130px"
          :placeholder="$t('settings.signatures.longPlaceholder')"
          class="signature-editor"
          @update:html="bodyHtml = $event"
          @inline-image-added="onEditorImageAdded"
        />
        <div class="logo-row">
          <label class="logo-upload">
            {{ $t('settings.signatures.insertLogo') }}
            <input type="file" accept="image/*" multiple @change="addImages" />
          </label>
          <ul v-if="assets.length" class="asset-list">
            <li v-for="asset in assets" :key="asset.contentId">
              <span>{{ asset.filename }}</span>
              <button type="button" class="delete" @click="removeAsset(asset.contentId)">{{ $t('common.remove') }}</button>
            </li>
          </ul>
        </div>
      </div>
      <div class="field">
        <span>{{ $t('settings.signatures.shortVersion') }}</span>
        <RichTextEditor
          ref="shortEditorRef"
          v-model="shortBodyText"
          :html="shortBodyHtml"
          min-height="80px"
          :placeholder="$t('settings.signatures.shortPlaceholder')"
          class="signature-editor"
          @update:html="shortBodyHtml = $event"
          @inline-image-added="onEditorImageAdded"
        />
      </div>
      <label class="field">
        <span>{{ $t('settings.signatures.identity') }}</span>
        <select v-model="identityEmail">
          <option v-for="identity in identityOptions" :key="identity.email" :value="identity.email">
            {{ identity.label }}
          </option>
        </select>
      </label>
      <label class="field">
        <span>{{ $t('settings.signatures.usageInThread') }}</span>
        <select v-model="firstContactMode">
          <option value="always_full">{{ $t('settings.signatures.modeAlwaysFull') }}</option>
          <option value="full_then_short">{{ $t('settings.signatures.modeFullThenShort') }}</option>
          <option value="always_short">{{ $t('settings.signatures.modeAlwaysShort') }}</option>
        </select>
      </label>
      <ToggleSwitch v-model="isDefault">{{ $t('settings.signatures.useAsDefault') }}</ToggleSwitch>

      <div class="actions">
        <button v-if="editingId" type="button" class="ghost" @click="reset">{{ $t('common.cancel') }}</button>
        <button type="button" class="primary" @click="save">{{ $t('common.save') }}</button>
      </div>
      <p v-if="status" class="status">{{ status }}</p>
    </div>
  </section>
</template>

<style scoped>
section {
  display: grid;
  gap: 14px;
}

h3 {
  margin: 0;
  color: var(--text-primary);
}

h4 {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-md);
}

.field {
  display: grid;
  gap: 4px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}

.field input,
.field select,
.field textarea {
  border: 1px solid var(--border-color);
  background: var(--bg-raised);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
}

.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.list li {
  display: flex;
  gap: 6px;
}

.row {
  flex: 1;
  text-align: left;
  background: var(--bg-raised);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  cursor: pointer;
  display: grid;
  gap: 2px;
  color: var(--text-primary);
}
.row:hover {
  background: var(--bg-hover);
}

.name {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: var(--weight-medium);
}

.badge {
  background: color-mix(in srgb, var(--accent-primary) 22%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  padding: 1px 8px;
  border-radius: var(--radius-full);
}

.preview {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.delete {
  background: transparent;
  border: 1px solid var(--border-color-muted);
  color: var(--text-secondary);
  border-radius: var(--radius-sm);
  padding: 0 12px;
  font-size: var(--font-size-xs);
  cursor: pointer;
}
.delete:hover {
  border-color: var(--accent-red);
  color: var(--accent-red);
}

.empty {
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  margin: 0;
}

.editor {
  display: grid;
  gap: 10px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  padding: 14px;
  background: var(--bg-base);
}

.suggestions {
  display: grid;
  gap: 8px;
  border: 1px solid color-mix(in srgb, var(--accent-primary) 45%, var(--border-color));
  border-radius: var(--radius-md);
  padding: 12px;
  background: color-mix(in srgb, var(--accent-primary) 6%, var(--bg-base));
}

.suggestions-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.suggestions-heading h4 {
  margin: 0;
}

.suggestions-heading .ghost {
  flex: none;
  padding: 6px 12px;
  font-size: var(--font-size-sm);
}

.suggestions-heading p,
.suggestion small {
  margin: 3px 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.suggestion {
  display: grid;
  gap: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border-color-muted);
}

.suggestions--empty {
  border-color: var(--border-color-muted);
  background: var(--bg-base);
}

.suggestion--adopted {
  opacity: 0.65;
}

.suggestion pre {
  margin: 6px 0 0;
  max-height: 132px;
  overflow: auto;
  white-space: pre-wrap;
  color: var(--text-secondary);
  font: inherit;
}

.suggestion-actions,
.asset-list li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.signature-editor :deep(.content) {
  font-size: var(--font-size-sm);
}

.logo-row {
  display: grid;
  gap: 6px;
  justify-items: start;
}

.logo-upload {
  display: inline-flex;
  align-items: center;
  padding: 6px 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-xs);
}

.logo-upload:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.logo-upload input {
  display: none;
}

.suggestion-preview {
  margin: 6px 0 0;
  max-height: 132px;
  overflow: auto;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.asset-list {
  display: grid;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.asset-list .delete {
  padding: 5px 9px;
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.primary {
  background: var(--accent-primary);
  color: var(--text-on-accent);
  border: 0;
  padding: 8px 16px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-weight: var(--weight-semibold);
}

.ghost {
  background: transparent;
  border: 1px solid var(--border-color);
  color: var(--text-primary);
  padding: 8px 16px;
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.status {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}
</style>
