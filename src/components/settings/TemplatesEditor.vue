<script setup lang="ts">
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { computed, nextTick, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useTemplatesStore } from '@/stores/templates';
import { useCrudActions } from '@/composables/useCrudActions';
import AttachmentDropZone from '@/components/mail/AttachmentDropZone.vue';
import type { BackendOutgoingAttachment } from '@/services/backend';

const { t } = useI18n();
const templatesStore = useTemplatesStore();
const crud = useCrudActions();

const editingId = ref<string | null>(null);
const editorOpen = ref(false);
const editorRef = ref<HTMLElement | null>(null);
const name = ref('');
const subject = ref('');
const bodyText = ref('');
const shortcut = ref('');
const ccText = ref('');
const bccText = ref('');
const attachments = ref<BackendOutgoingAttachment[]>([]);
const status = ref('');

onMounted(() => {
  templatesStore.load();
});

const recipientPlaceholder = '{{recipient_name}}';
const myNamePlaceholder = '{{my_name}}';
const datePlaceholder = '{{date}}';
const placeholderHint = computed(() =>
  t('settings.templates.bodyPlaceholder', { recipient: recipientPlaceholder }),
);

function startNew() {
  reset();
  editorOpen.value = true;
  void nextTick(() => editorRef.value?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }));
}

function reset() {
  editorOpen.value = false;
  editingId.value = null;
  name.value = '';
  subject.value = '';
  bodyText.value = '';
  shortcut.value = '';
  ccText.value = '';
  bccText.value = '';
  attachments.value = [];
  status.value = '';
}

function edit(id: string) {
  editorOpen.value = true;
  const tpl = templatesStore.items.find((t) => t.id === id);
  if (!tpl) return;
  editingId.value = id;
  name.value = tpl.name;
  subject.value = tpl.subject ?? '';
  bodyText.value = tpl.body_text;
  shortcut.value = tpl.shortcut ?? '';
  ccText.value = parseJsonList(tpl.cc_json).join(', ');
  bccText.value = parseJsonList(tpl.bcc_json).join(', ');
  attachments.value = parseAttachmentsJson(tpl.attachments_json);
}

async function save() {
  if (!name.value.trim()) {
    status.value = t('settings.templates.nameRequired');
    return;
  }
  try {
    await templatesStore.upsert({
      id: editingId.value,
      name: name.value.trim(),
      subject: subject.value.trim() || null,
      bodyText: bodyText.value,
      shortcut: shortcut.value.trim() || null,
      cc: parseRecipientText(ccText.value),
      bcc: parseRecipientText(bccText.value),
      attachmentsJson: attachments.value.length > 0 ? JSON.stringify(attachments.value) : null,
    });
    status.value = t('settings.templates.saved');
    reset();
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function remove(id: string) {
  const template = templatesStore.items.find((item) => item.id === id);
  if (!template) return;
  const removed = await crud.removeWithUndo({
    id,
    name: template.name,
    remove: () => templatesStore.remove(id),
    restore: (snapshot) => templatesStore.restore(snapshot),
  });
  if (removed && editingId.value === id) reset();
}

function parseRecipientText(value: string): string[] {
  return value
    .split(/[,;\n]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function parseJsonList(value?: string | null): string[] {
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed.filter((item) => typeof item === 'string') : [];
  } catch {
    return [];
  }
}

function parseAttachmentsJson(value?: string | null): BackendOutgoingAttachment[] {
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}
</script>

<template>
  <section>
    <h3 class="settings-page-title">{{ $t('settings.sections.templates.label') }}</h3>
    <p class="hint">
      {{
        $t('settings.templates.variablesHint', {
          recipient: recipientPlaceholder,
          myName: myNamePlaceholder,
          date: datePlaceholder,
        })
      }}
    </p>

    <CollectionLoadState
      :loading="templatesStore.loading"
      :failed="templatesStore.loadFailed"
      :has-content="templatesStore.items.length > 0"
      :error-label="$t('settings.templates.loadFailed')"
      @retry="templatesStore.load()"
    />
    <!-- A list with an Add row; the form opens on demand. -->
    <div class="setting-group list" :aria-busy="templatesStore.loading">
      <div v-for="tpl in templatesStore.items" :key="tpl.id" class="setting-row collection-row">
          <CardButton class="row" @click="edit(tpl.id)">
            <span class="name">{{ tpl.name }}</span>
            <span v-if="tpl.shortcut" class="shortcut">{{ tpl.shortcut }}</span>
            <span v-if="tpl.subject" class="subject">{{ tpl.subject }}</span>
            <span class="preview">{{ tpl.body_text.slice(0, 80) }}</span>
          </CardButton>
          <AppButton class="delete" @click="remove(tpl.id)" variant="ghost" size="sm">{{ $t('common.delete') }}</AppButton>
      </div>
      <div v-if="!(templatesStore.items.length > 0) && !templatesStore.loading && !templatesStore.loadFailed" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.templates.empty') }}</span>
      </div>
      <div v-if="!editorOpen" class="setting-row setting-row--single">
        <AppButton variant="secondary" size="sm" @click="startNew">
          <template #icon><AppIcon><path d="M8 3v10M3 8h10" /></AppIcon></template>
          {{ $t('settings.templates.createTitle') }}
        </AppButton>
      </div>
    </div>

    <div v-if="editorOpen" ref="editorRef" class="editor">
      <h4>{{ editingId ? $t('settings.templates.editTitle') : $t('settings.templates.createTitle') }}</h4>
      <label class="field"><span>{{ $t('common.name') }}</span><input v-model="name" /></label>
      <label class="field"><span>{{ $t('settings.templates.subjectOptional') }}</span><input v-model="subject" /></label>
      <label class="field">
        <span>{{ $t('settings.templates.body') }}</span>
        <textarea v-model="bodyText" rows="6" :placeholder="placeholderHint" />
      </label>
      <label class="field">
        <span>{{ $t('settings.templates.shortcutOptional') }}</span>
        <input v-model="shortcut" :placeholder="$t('settings.templates.shortcutPlaceholder')" />
      </label>
      <label class="field">
        <span>{{ $t('settings.templates.ccAuto') }}</span>
        <input v-model="ccText" :placeholder="$t('settings.templates.ccPlaceholder')" />
      </label>
      <label class="field">
        <span>{{ $t('settings.templates.bccAuto') }}</span>
        <input v-model="bccText" :placeholder="$t('settings.templates.bccPlaceholder')" />
      </label>
      <AttachmentDropZone v-model="attachments" />

      <div class="actions">
        <AppButton class="ghost" @click="reset" variant="ghost" size="sm">{{ $t('common.cancel') }}</AppButton>
        <AppButton class="primary" @click="save" variant="primary" size="sm">{{ $t('common.save') }}</AppButton>
      </div>
    </div>
    <p v-if="status" class="status" role="status">{{ status }}</p>
  </section>
</template>

<style scoped>
.shortcut {
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
  color: var(--accent-primary);
}
</style>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
