<script setup lang="ts">
/**
 * Who a mail merge goes to: a CSV file (header row = field names) or a
 * selection of contacts. Applying it puts the composer into merge mode; the
 * rows are only checked against the message in the review step.
 */
import { computed, onMounted, ref, useId } from 'vue';
import { useI18n } from 'vue-i18n';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import SearchInput from '@/components/ui/SearchInput.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import { useContactsStore } from '@/stores/contacts';
import { useMailMergeStore } from '@/stores/mailMerge';
import type { BackendMergeCsv } from '@/types/mailMerge';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { contactsMergeSource, fileToBase64, type MailMergeSource } from '@/utils/mailMerge';

const props = defineProps<{
  /** The list in use, so "Change list" opens on it. */
  current?: MailMergeSource | null;
}>();

const emit = defineEmits<{
  apply: [source: MailMergeSource];
  close: [];
}>();

const { t, locale } = useI18n();
const contactsStore = useContactsStore();
const mailMerge = useMailMergeStore();
const emailFieldId = useId();
/** `satchel_core::mail_merge::csv::MAX_CSV_BYTES`. */
const MAX_CSV_BYTES = 5 * 1024 * 1024;

const mode = ref<'csv' | 'contacts'>(props.current?.kind ?? 'csv');
const modeOptions = computed(() => [
  { value: 'csv', label: t('mailMerge.source.csv') },
  { value: 'contacts', label: t('mailMerge.source.contacts') },
]);

/* CSV ---------------------------------------------------------------------- */

const fileInput = ref<HTMLInputElement | null>(null);
const parsed = ref<BackendMergeCsv | null>(null);
const fileName = ref(props.current?.kind === 'csv' ? props.current.label : '');
const emailField = ref(props.current?.kind === 'csv' ? props.current.emailField : '');
const reading = ref(false);
const csvError = ref('');

if (props.current?.kind === 'csv') {
  parsed.value = {
    fields: props.current.fields,
    rows: props.current.rows.map((row) => ({ line: row.line ?? 0, values: row.values })),
    email_field: props.current.emailField,
    delimiter: '',
    encoding: 'utf8',
    truncated: false,
  };
}

function pickFile() {
  fileInput.value?.click();
}

async function onFileChosen(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = '';
  if (!file) return;
  // The backend refuses more than 5 MiB as well; reading it first would only
  // hold a large file in memory twice to hear that.
  if (file.size > MAX_CSV_BYTES) {
    parsed.value = null;
    csvError.value = t('backend.errors.mailMergeCsvTooLarge');
    return;
  }
  reading.value = true;
  csvError.value = '';
  try {
    const result = await mailMerge.parseCsv(await fileToBase64(file));
    parsed.value = result;
    fileName.value = file.name;
    emailField.value = result.email_field ?? '';
  } catch (error) {
    parsed.value = null;
    csvError.value = backendErrorLabel(error);
  } finally {
    reading.value = false;
  }
}

const encodingLabel = computed(() => {
  switch (parsed.value?.encoding) {
    case 'utf8_bom': return t('mailMerge.source.encoding.utf8Bom');
    case 'utf16': return t('mailMerge.source.encoding.utf16');
    case 'windows1252': return t('mailMerge.source.encoding.windows1252');
    default: return t('mailMerge.source.encoding.utf8');
  }
});

const delimiterLabel = computed(() => {
  switch (parsed.value?.delimiter) {
    case ';': return t('mailMerge.source.delimiter.semicolon');
    case '\t': return t('mailMerge.source.delimiter.tab');
    case ',': return t('mailMerge.source.delimiter.comma');
    default: return '';
  }
});

/* Contacts ----------------------------------------------------------------- */

const search = ref('');
const selected = ref<Set<string>>(new Set(
  props.current?.kind === 'contacts' ? props.current.rows.map((row) => row.values.Email?.toLowerCase() ?? '') : [],
));

onMounted(() => {
  if (!contactsStore.loaded) void contactsStore.load();
});

const contactRows = computed(() => contactsStore.contacts
  .filter((contact) => contact.email.includes('@'))
  .map((contact) => ({
    email: contact.email,
    key: contact.email.trim().toLowerCase(),
    name: contact.display_name?.trim() || contact.email,
  }))
  .sort((left, right) => left.name.localeCompare(right.name, locale.value, { sensitivity: 'base' })));

const visibleContacts = computed(() => {
  const query = search.value.trim().toLocaleLowerCase(locale.value);
  if (!query) return contactRows.value;
  return contactRows.value.filter((contact) =>
    `${contact.name} ${contact.email}`.toLocaleLowerCase(locale.value).includes(query));
});

const allVisibleSelected = computed(() =>
  visibleContacts.value.length > 0 && visibleContacts.value.every((contact) => selected.value.has(contact.key)));

function toggle(key: string, on: boolean) {
  const next = new Set(selected.value);
  if (on) next.add(key);
  else next.delete(key);
  selected.value = next;
}

function toggleAllVisible() {
  const next = new Set(selected.value);
  const select = !allVisibleSelected.value;
  for (const contact of visibleContacts.value) {
    if (select) next.add(contact.key);
    else next.delete(contact.key);
  }
  selected.value = next;
}

/* Apply -------------------------------------------------------------------- */

const recipientCount = computed(() => mode.value === 'csv'
  ? (parsed.value && emailField.value ? parsed.value.rows.length : 0)
  : selected.value.size);

function apply() {
  if (recipientCount.value === 0) return;
  if (mode.value === 'csv' && parsed.value) {
    emit('apply', {
      kind: 'csv',
      label: fileName.value,
      fields: parsed.value.fields,
      emailField: emailField.value,
      rows: parsed.value.rows.map((row) => ({ line: row.line || null, values: row.values })),
    });
    return;
  }
  const chosen = contactRows.value.filter((contact) => selected.value.has(contact.key));
  emit('apply', contactsMergeSource(chosen, t('mailMerge.source.contactsLabel', { count: chosen.length }, chosen.length)));
}

function close() {
  emit('close');
}

useDismissableOnTop(() => true, close);
</script>

<template>
  <Teleport to="body">
    <ModalShell :label="$t('mailMerge.source.title')" width="560px" height="auto" @dismiss="close">
      <div class="merge-source" data-testid="mail-merge-source">
        <header class="merge-source__head">
          <h2 class="merge-source__title">{{ $t('mailMerge.source.title') }}</h2>
          <p class="merge-source__lead">{{ $t('mailMerge.source.lead') }}</p>
        </header>

        <SegmentedControl
          :options="modeOptions"
          :model-value="mode"
          :aria-label="$t('mailMerge.source.kindLabel')"
          @update:model-value="mode = $event as 'csv' | 'contacts'"
        />

        <section v-if="mode === 'csv'" class="merge-source__body">
          <input
            ref="fileInput"
            class="sr-only"
            type="file"
            accept=".csv,.txt,text/csv,text/plain"
            :aria-label="$t('mailMerge.source.chooseFile')"
            data-testid="mail-merge-file"
            @change="onFileChosen"
          />
          <div class="merge-source__file">
            <AppButton variant="secondary" :disabled="reading" @click="pickFile">
              {{ parsed ? $t('mailMerge.source.chooseOther') : $t('mailMerge.source.chooseFile') }}
            </AppButton>
            <span v-if="fileName" class="merge-source__filename">{{ fileName }}</span>
          </div>
          <p class="merge-source__hint">{{ $t('mailMerge.source.csvHint') }}</p>
          <AppBanner v-if="csvError" tone="danger" :title="csvError" />
          <template v-if="parsed">
            <p class="merge-source__summary" role="status">
              {{ $t('mailMerge.source.rows', { count: parsed.rows.length }, parsed.rows.length) }}
              <template v-if="delimiterLabel"> · {{ delimiterLabel }} · {{ encodingLabel }}</template>
            </p>
            <p v-if="parsed.encoding === 'windows1252'" class="merge-source__hint" data-testid="mail-merge-encoding-hint">{{ $t('mailMerge.source.windows1252Hint') }}</p>
            <AppBanner v-if="parsed.truncated" tone="warning" :title="$t('mailMerge.source.truncated', { count: parsed.rows.length })" />
            <div class="merge-source__field">
              <label :for="emailFieldId">{{ $t('mailMerge.source.emailColumn') }}</label>
              <select :id="emailFieldId" v-model="emailField" data-testid="mail-merge-email-field">
                <option value="" disabled>{{ $t('mailMerge.source.emailColumnPick') }}</option>
                <option v-for="field in parsed.fields" :key="field" :value="field">{{ field }}</option>
              </select>
            </div>
            <p v-if="!emailField" class="merge-source__hint merge-source__hint--warn">{{ $t('mailMerge.source.emailColumnRequired') }}</p>
            <p class="merge-source__hint">{{ $t('mailMerge.source.fields', { fields: parsed.fields.join(', ') }) }}</p>
          </template>
        </section>

        <section v-else class="merge-source__body">
          <SearchInput v-model="search" :label="$t('mailMerge.source.searchContacts')" :placeholder="$t('mailMerge.source.searchContacts')" />
          <EmptyState
            v-if="contactRows.length === 0"
            :title="$t('mailMerge.source.noContacts')"
          />
          <template v-else>
            <AppButton size="sm" variant="ghost" class="merge-source__all" @click="toggleAllVisible">
              {{ allVisibleSelected ? $t('mailMerge.source.selectNone') : $t('mailMerge.source.selectAll', { count: visibleContacts.length }, visibleContacts.length) }}
            </AppButton>
            <ul class="merge-source__contacts" :aria-label="$t('mailMerge.source.contacts')">
              <li v-for="contact in visibleContacts" :key="contact.key">
                <label class="merge-source__contact">
                  <input
                    type="checkbox"
                    :checked="selected.has(contact.key)"
                    @change="toggle(contact.key, ($event.target as HTMLInputElement).checked)"
                  />
                  <span class="merge-source__contact-name">{{ contact.name }}</span>
                  <span v-if="contact.name !== contact.email" class="merge-source__contact-email">{{ contact.email }}</span>
                </label>
              </li>
            </ul>
          </template>
        </section>

        <footer class="merge-source__actions">
          <AppButton variant="ghost" @click="close">{{ $t('common.cancel') }}</AppButton>
          <AppButton variant="primary" :disabled="recipientCount === 0" data-testid="mail-merge-source-apply" @click="apply">
            {{ $t('mailMerge.source.apply', { count: recipientCount }, recipientCount) }}
          </AppButton>
        </footer>
      </div>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.merge-source {
  display: grid;
  gap: var(--space-lg);
  padding: var(--space-xl);
  min-width: 0;
}

.merge-source__head {
  display: grid;
  gap: var(--space-2xs);
}

.merge-source__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.merge-source__lead,
.merge-source__hint,
.merge-source__summary {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.merge-source__hint--warn {
  color: var(--accent-amber);
}

.merge-source__body {
  display: grid;
  gap: var(--space-md);
  min-width: 0;
}

.merge-source__file {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-sm);
  min-width: 0;
}

.merge-source__filename {
  min-width: 0;
  overflow: hidden;
  color: var(--text-primary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.merge-source__field {
  display: grid;
  gap: var(--space-2xs);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.merge-source__field select {
  min-height: var(--touch-target);
  padding: 0 var(--space-sm);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
}

.merge-source__all {
  justify-self: start;
}

.merge-source__contacts {
  display: grid;
  max-height: 320px;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
}

.merge-source__contact {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  min-height: var(--touch-target);
  padding: 0 var(--space-xs);
  border-radius: var(--radius-sm);
  cursor: pointer;
  min-width: 0;
}

.merge-source__contact:hover {
  background: var(--bg-hover);
}

.merge-source__contact-name {
  color: var(--text-primary);
  white-space: nowrap;
}

.merge-source__contact-email {
  min-width: 0;
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.merge-source__actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
