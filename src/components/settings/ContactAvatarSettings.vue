<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import Avatar from '@/components/common/Avatar.vue';
import { backendSupportsNativeCommands, type BackendContact } from '@/services/backend';
import { useContactsStore } from '@/stores/contacts';
import { useToast } from '@/composables/useToast';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import AppButton from '@/components/ui/AppButton.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useSettingsStore } from '@/stores/settings';

const { t } = useI18n();
const contactsStore = useContactsStore();
const settings = useSettingsStore();
const status = ref('');
const query = ref('');
const urlInputs = ref<Record<string, string>>({});
const fileInput = ref<HTMLInputElement | null>(null);
const fileTarget = ref<BackendContact | null>(null);

const contactCount = computed(() => contactsStore.contacts.length);
const avatarCount = computed(
  () => contactsStore.contacts.filter((contact) => Boolean(contact.avatar_url)).length,
);
const canImport = computed(() => backendSupportsNativeCommands());
const filteredContacts = computed(() => {
  const term = query.value.trim().toLowerCase();
  const contacts = contactsStore.contacts;
  const filtered = term
    ? contacts.filter((contact) =>
        `${contact.display_name ?? ''} ${contact.email}`.toLowerCase().includes(term),
      )
    : contacts;
  return filtered
    .slice()
    .sort((a, b) => Number(Boolean(b.avatar_url)) - Number(Boolean(a.avatar_url)))
    .slice(0, 24);
});

onMounted(async () => {
  if (!contactsStore.loaded) {
    await contactsStore.load();
  }
});

function sourceLabel(source?: string | null): string {
  if (source === 'manual') return t('settings.contacts.source.manual');
  if (source === 'local_contact') return t('settings.contacts.source.localContact');
  if (source === 'provider_contact') return t('settings.contacts.source.provider');
  if (source === 'bimi') return t('settings.contacts.source.bimi');
  if (source === 'gravatar') return t('settings.contacts.source.gravatar');
  if (source === 'favicon') return t('settings.contacts.source.favicon');
  return t('settings.contacts.source.fallback');
}

const toast = useToast();

/** Sets the avatar and reports the outcome; `false` when it failed. */
async function applyAvatar(
  input: Parameters<typeof contactsStore.setAvatar>[0],
  success: string,
  failure: string,
): Promise<boolean> {
  try {
    await contactsStore.setAvatar(input);
    status.value = success;
    return true;
  } catch (error) {
    status.value = failure;
    toast.error(`${failure} ${backendErrorLabel(error)}`);
    return false;
  }
}

async function importAvatars() {
  status.value = '';
  const result = await contactsStore.syncAvatars(settings.thirdPartyContactAvatars);

  if (!result) {
    status.value = contactsStore.lastAvatarSyncError || t('settings.contacts.importFailed');
    return;
  }

  const updatedTotal = result.avatars_updated + result.fallback_avatars_updated;

  if (result.errors.length > 0 && updatedTotal === 0) {
    status.value = result.errors.map((key) => t(key)).join(' ');
    return;
  }

  status.value = t('settings.contacts.updatedCount', { count: updatedTotal }, updatedTotal);
}

function chooseFile(contact: BackendContact) {
  fileTarget.value = contact;
  fileInput.value?.click();
}

async function onFileSelected(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  const contact = fileTarget.value;
  input.value = '';
  fileTarget.value = null;
  if (!file || !contact) return;

  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ''));
    reader.onerror = () => reject(reader.error ?? new Error('file read failed'));
    reader.readAsDataURL(file);
  }).catch(() => '');

  if (!dataUrl) {
    status.value = t('settings.contacts.imageReadFailed');
    return;
  }

  await applyAvatar(
    {
      email: contact.email,
      displayName: contact.display_name,
      avatarUrl: dataUrl,
      locked: true,
    },
    t('settings.contacts.avatarSet'),
    t('settings.contacts.avatarSetFailed'),
  );
}

async function setUrlAvatar(contact: BackendContact) {
  const avatarUrl = urlInputs.value[contact.email]?.trim();
  if (!avatarUrl) return;
  const updated = await applyAvatar(
    {
      email: contact.email,
      displayName: contact.display_name,
      avatarUrl,
      locked: true,
    },
    t('settings.contacts.avatarSet'),
    t('settings.contacts.avatarLoadFailed'),
  );
  if (updated) {
    urlInputs.value[contact.email] = '';
  }
}

async function clearAvatar(contact: BackendContact, locked: boolean) {
  await applyAvatar(
    {
      email: contact.email,
      displayName: contact.display_name,
      avatarUrl: null,
      locked,
    },
    locked ? t('settings.contacts.avatarLocked') : t('settings.contacts.avatarReset'),
    t('settings.contacts.avatarChangeFailed'),
  );
}
</script>

<template>
  <section>
    <header class="section-header">
      <div>
        <h3>{{ $t('settings.sections.contacts.label') }}</h3>
        <p class="hint">
          {{ $t('settings.contacts.priorityHint') }}
        </p>
      </div>
      <div class="stats" :aria-label="$t('settings.contacts.statsLabel')">
        <strong>{{ avatarCount }}</strong>
        <span>{{ $t('settings.contacts.ofTotal', { count: contactCount }) }}</span>
      </div>
    </header>

    <div class="permission-note">
      <div class="note-icon" aria-hidden="true">
        <svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="8" cy="5" r="2.5" />
          <path d="M3 14c0-2.6 2.2-4.5 5-4.5s5 1.9 5 4.5" />
        </svg>
      </div>
      <div>
        <strong>{{ $t('settings.contacts.noAutoImportTitle') }}</strong>
        <p>
          {{ $t('settings.contacts.noAutoImportBody') }}
        </p>
      </div>
    </div>

    <div class="third-party-setting">
      <ToggleSwitch v-model="settings.thirdPartyContactAvatars" :label="t('settings.contacts.thirdPartyLabel')">
        {{ t('settings.contacts.thirdPartyLabel') }}
      </ToggleSwitch>
      <p class="hint">{{ t('settings.contacts.thirdPartyHint') }}</p>
    </div>

    <div class="toolbar">
      <input
        v-model="query"
        type="search"
        :placeholder="$t('settings.contacts.searchPlaceholder')"
      />
      <AppButton
        variant="primary"
        :disabled="contactsStore.avatarSyncRunning || !canImport"
        @click="importAvatars"
      >
        {{
          contactsStore.avatarSyncRunning
            ? $t('settings.contacts.importing')
            : $t('settings.contacts.refreshImport')
        }}
      </AppButton>
    </div>

    <input
      ref="fileInput"
      class="file-input"
      type="file"
      accept="image/png,image/jpeg,image/webp,image/gif,image/svg+xml"
      @change="onFileSelected"
    />

    <div class="contact-list">
      <article v-for="contact in filteredContacts" :key="contact.id" class="contact-row">
        <Avatar
          :name="contact.display_name || contact.email"
          :email="contact.email"
          :image-url="contact.avatar_url"
          :size="36"
        />
        <div class="contact-main">
          <div class="contact-title">
            <strong>{{ contact.display_name || contact.email }}</strong>
            <span :class="['source', contact.avatar_locked ? 'source--locked' : '']">
              {{
                contact.avatar_locked
                  ? $t('settings.contacts.pinned')
                  : sourceLabel(contact.avatar_source)
              }}
            </span>
          </div>
          <span class="email">{{ contact.email }}</span>
          <div class="url-row">
            <input
              v-model="urlInputs[contact.email]"
              type="url"
              :placeholder="$t('settings.contacts.imageUrlPlaceholder')"
              @keydown.enter.prevent="setUrlAvatar(contact)"
            />
            <AppButton variant="secondary" size="sm" @click="setUrlAvatar(contact)">
              {{ $t('settings.contacts.set') }}
            </AppButton>
          </div>
        </div>
        <div class="row-actions">
          <AppButton variant="secondary" size="sm" @click="chooseFile(contact)">
            {{ $t('settings.contacts.file') }}
          </AppButton>
          <AppButton variant="secondary" size="sm" @click="clearAvatar(contact, false)">
            {{ $t('settings.contacts.auto') }}
          </AppButton>
          <AppButton variant="secondary" size="sm" @click="clearAvatar(contact, true)">
            {{ $t('settings.contacts.lock') }}
          </AppButton>
        </div>
      </article>
    </div>

    <p v-if="!canImport" class="status">
      {{ $t('settings.contacts.desktopOnly') }}
    </p>
    <p v-else-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
.third-party-setting {
  display: grid;
  gap: var(--space-2);
}

section {
  display: grid;
  gap: 16px;
}

.section-header {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 16px;
  align-items: start;
}

h3 {
  margin: 0;
  color: var(--text-primary);
}

.hint,
.status {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.hint {
  margin-top: 6px;
}

.stats {
  display: grid;
  justify-items: end;
  gap: 2px;
  padding: 8px 10px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.stats strong {
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  line-height: 1;
}

.permission-note {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
}

.permission-note strong {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.permission-note p {
  margin: 4px 0 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.note-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-sm);
  background: var(--bg-active);
  color: var(--accent-primary);
}

.toolbar {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 8px;
}

.toolbar input,
.url-row input {
  min-width: 0;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-primary);
  font-family: inherit;
  padding: 8px 10px;
}

.file-input {
  display: none;
}

.contact-list {
  display: grid;
  gap: 8px;
}

.contact-row {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  gap: 12px;
  align-items: center;
  padding: 10px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-raised);
}

.contact-main {
  min-width: 0;
  display: grid;
  gap: 5px;
}

.contact-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.contact-title strong,
.email {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.contact-title strong {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}

.email {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.source {
  flex-shrink: 0;
  border-radius: var(--radius-xs);
  background: var(--bg-overlay);
  color: var(--text-tertiary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  padding: 2px 6px;
}

.source--locked {
  color: var(--accent-primary);
}

.url-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 6px;
}

.row-actions {
  display: grid;
  grid-template-columns: repeat(3, auto);
  gap: 6px;
}

@media (max-width: 760px) {
  .contact-row,
  .toolbar {
    grid-template-columns: 1fr;
  }

  .row-actions {
    justify-content: start;
  }
}
</style>
