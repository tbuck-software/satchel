<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useTemplatesStore } from '@/stores/templates';
import { useCrudActions } from '@/composables/useCrudActions';
import AttachmentDropZone from '@/components/mail/AttachmentDropZone.vue';
import type { BackendOutgoingAttachment } from '@/services/backend';

const { t } = useI18n();
const templatesStore = useTemplatesStore();
const crud = useCrudActions();

const editingId = ref<string | null>(null);
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

function reset() {
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
    <h3>{{ $t('settings.templates.title') }}</h3>
    <p class="hint">
      {{
        $t('settings.templates.variablesHint', {
          recipient: recipientPlaceholder,
          myName: myNamePlaceholder,
          date: datePlaceholder,
        })
      }}
    </p>

    <ul v-if="templatesStore.items.length > 0" class="list">
      <li v-for="tpl in templatesStore.items" :key="tpl.id">
        <button type="button" class="row" @click="edit(tpl.id)">
          <span class="name">{{ tpl.name }}</span>
          <span v-if="tpl.shortcut" class="shortcut">{{ tpl.shortcut }}</span>
          <span v-if="tpl.subject" class="subject">{{ tpl.subject }}</span>
          <span class="preview">{{ tpl.body_text.slice(0, 80) }}</span>
        </button>
        <button type="button" class="delete" @click="remove(tpl.id)">{{ $t('common.delete') }}</button>
      </li>
    </ul>
    <p v-else class="empty">{{ $t('settings.templates.empty') }}</p>

    <div class="editor">
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
h3,
h4 {
  margin: 0;
  color: var(--text-primary);
}
.hint {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}
code {
  background: var(--bg-raised);
  padding: 1px 6px;
  border-radius: var(--radius-xs);
  font-size: 0.9em;
}
.field {
  display: grid;
  gap: 4px;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
}
.field input,
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
  font-weight: var(--weight-medium);
}
.shortcut {
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
  color: var(--accent-primary);
}
.subject {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
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
