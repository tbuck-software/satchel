<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConfirm } from '@/composables/useConfirm';
import { usePrompt } from '@/composables/usePrompt';
import { useSmimeStore } from '@/stores/smime';
import type { BackendSmimeCert } from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';

const { t } = useI18n();
const { ask } = useConfirm();
const prompt = usePrompt();
const store = useSmimeStore();
/** Which add form is open: import, generate, or none (the list alone). */
const panel = ref<'import' | 'generate' | null>(null);
const pem = ref('');
const label = ref('');
const passphrase = ref('');
const status = ref('');
const generating = ref(false);
const generateName = ref('');
const generateEmail = ref('');
const generatePassphrase = ref('');
const pkcs12Input = ref<HTMLInputElement | null>(null);

const certs = computed(() => store.certs);

onMounted(() => {
  void store.load();
});

async function importPem() {
  if (!pem.value.trim()) return;
  status.value = '';
  try {
    await store.addPem({
      label: label.value,
      pem: pem.value,
      passphrase: passphrase.value,
    });
    pem.value = '';
    label.value = '';
    passphrase.value = '';
    status.value = t('settings.smime.imported');
    panel.value = null;
  } catch (err) {
    status.value = t('settings.smime.importFailed', {
      error: err instanceof Error ? err.message : String(err),
    });
  }
}

async function importPkcs12(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  const phrase = await prompt.ask({
    title: t('settings.smime.importPkcs12'),
    body: file.name,
    label: t('settings.smime.pkcs12Prompt'),
    initialValue: passphrase.value,
    confirmLabel: t('settings.smime.importPkcs12'),
    inputType: 'password',
  });
  if (phrase === null) {
    // Choosing the same file again must fire `change` again.
    input.value = '';
    return;
  }
  status.value = '';
  try {
    const buffer = await file.arrayBuffer();
    const bytes = Array.from(new Uint8Array(buffer));
    const base64 = btoa(String.fromCharCode(...bytes));
    await store.importPkcs12({ label: label.value || file.name, p12Base64: base64, passphrase: phrase });
    status.value = t('settings.smime.pkcs12Imported');
    panel.value = null;
  } catch (err) {
    status.value = t('settings.smime.pkcs12ImportFailed', {
      error: err instanceof Error ? err.message : String(err),
    });
  } finally {
    input.value = '';
  }
}

async function generateSelfSigned() {
  if (!generateName.value.trim() || !generateEmail.value.includes('@')) return;
  generating.value = true;
  status.value = '';
  try {
    await store.generate({
      name: generateName.value,
      email: generateEmail.value,
      label: generateName.value,
      passphrase: generatePassphrase.value,
    });
    status.value = t('settings.smime.selfSignedCreated');
    panel.value = null;
  } catch (err) {
    status.value = t('settings.smime.generateFailed', {
      error: err instanceof Error ? err.message : String(err),
    });
  } finally {
    generating.value = false;
  }
}

async function unlock(cert: BackendSmimeCert) {
  const phrase = cert.has_passphrase
    ? await prompt.ask({
      title: t('settings.smime.unlockTitle'),
      body: cert.label,
      label: t('settings.smime.unlockPrompt'),
      confirmLabel: t('settings.smime.unlock'),
      inputType: 'password',
    })
    : '';
  if (phrase === null) return;
  try {
    await store.unlock(cert.id, phrase);
    status.value = t('settings.smime.unlockedStatus');
  } catch (err) {
    status.value = t('settings.smime.unlockFailed', {
      error: err instanceof Error ? err.message : String(err),
    });
  }
}

async function remove(cert: BackendSmimeCert) {
  const confirmed = await ask({
    title: t('settings.smime.deleteConfirm', { label: cert.label }),
    confirmLabel: t('common.delete'),
    tone: 'danger',
  });
  if (!confirmed) return;
  await store.remove(cert.id);
}
</script>

<template>
  <section class="smime-settings">
    <header>
      <div>
        <h2 class="settings-page-title">{{ $t('settings.sections.smime.label') }}</h2>
      </div>
      <AppButton variant="secondary" @click="store.load">{{ $t('common.refresh') }}</AppButton>
    </header>

    <!-- Certificates first, then the two ways to add one, each opening its
         form on demand instead of two forms standing open above the list. -->
    <div class="setting-group cert-list">
      <article v-for="cert in certs" :key="cert.id" class="setting-row cert-row">
        <div class="setting-info">
          <span class="setting-label">{{ cert.label }}</span>
          <span class="setting-hint">{{ cert.subject_email || cert.subject_cn || $t('settings.smime.noSubject') }}</span>
          <code>{{ cert.fingerprint }}</code>
        </div>
        <div class="cert-actions">
          <span class="badge">{{ cert.cert_type === 'private' ? $t('settings.smime.typePrivate') : $t('settings.smime.typePublic') }}</span>
          <span v-if="cert.is_unlocked" class="badge">{{ $t('settings.smime.unlockedBadge') }}</span>
          <AppButton
            v-if="cert.cert_type === 'private' && !cert.is_unlocked"
            variant="secondary"
            size="sm"
            @click="unlock(cert)"
          >
            {{ $t('settings.smime.unlock') }}
          </AppButton>
          <AppButton variant="ghost" size="sm" @click="remove(cert)">{{ $t('common.delete') }}</AppButton>
        </div>
      </article>
      <div v-if="!store.loading && certs.length === 0" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.smime.empty') }}</span>
      </div>
      <div v-if="panel === null" class="setting-row setting-row--single">
        <div class="cert-add">
          <AppButton variant="secondary" size="sm" @click="panel = 'import'">{{ $t('settings.smime.importTitle') }}</AppButton>
          <AppButton variant="ghost" size="sm" @click="panel = 'generate'">{{ $t('settings.smime.selfSignedTitle') }}</AppButton>
        </div>
      </div>
    </div>

    <div v-if="panel === 'import'" class="panel">
      <h3>{{ $t('settings.smime.importTitle') }}</h3>
      <p class="hint">{{ $t('settings.smime.importHint') }}</p>
      <div class="field-grid">
        <label>
          <span>{{ $t('settings.smime.label') }}</span>
          <input v-model="label" :placeholder="$t('settings.smime.labelPlaceholder')" />
        </label>
        <label>
          <span>{{ $t('settings.smime.passphraseOptional') }}</span>
          <input v-model="passphrase" type="password" placeholder="••••••" />
        </label>
      </div>
      <label class="pem-field">
        <span>{{ $t('settings.smime.pemContent') }}</span>
        <textarea v-model="pem" rows="7" :placeholder="$t('settings.smime.pemPlaceholder')" />
      </label>
      <div class="actions">
        <AppButton variant="ghost" size="sm" @click="panel = null">{{ $t('common.cancel') }}</AppButton>
        <AppButton variant="primary" :disabled="!pem.trim()" @click="importPem">
          {{ $t('settings.smime.importPem') }}
        </AppButton>
        <AppButton variant="secondary" @click="pkcs12Input?.click()">
          {{ $t('settings.smime.importPkcs12') }}
        </AppButton>
        <input
          ref="pkcs12Input"
          class="file-input"
          type="file"
          accept=".p12,.pfx,application/x-pkcs12"
          :aria-label="$t('settings.smime.importPkcs12')"
          @change="importPkcs12"
        />
      </div>
    </div>

    <div v-if="panel === 'generate'" class="panel">
      <h3>{{ $t('settings.smime.selfSignedTitle') }}</h3>
      <p class="hint">{{ $t('settings.smime.selfSignedHint') }}</p>
      <div class="field-grid">
        <label>
          <span>{{ $t('common.name') }}</span>
          <input v-model="generateName" :placeholder="$t('settings.smime.namePlaceholder')" />
        </label>
        <label>
          <span>{{ $t('common.email') }}</span>
          <input v-model="generateEmail" type="email" :placeholder="$t('settings.smime.emailPlaceholder')" />
        </label>
        <label class="span-full">
          <span>{{ $t('settings.smime.passphraseOptional') }}</span>
          <input v-model="generatePassphrase" type="password" :placeholder="$t('settings.smime.generatePassphrasePlaceholder')" />
        </label>
      </div>
      <div class="actions">
        <AppButton variant="ghost" size="sm" @click="panel = null">{{ $t('common.cancel') }}</AppButton>
        <AppButton variant="primary" :disabled="generating" @click="generateSelfSigned">
          {{ generating ? $t('settings.smime.generating') : $t('settings.smime.selfSignedTitle') }}
        </AppButton>
      </div>
    </div>

    <p v-if="status" class="status">{{ status }}</p>

  </section>
</template>

<style scoped>
.smime-settings {
  display: grid;
  gap: 18px;
}
header,
.actions,
.cert-actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: var(--space-xs); }
.cert-add { display: flex; flex-wrap: wrap; gap: var(--space-sm); }
.badge { padding: 1px 8px; border-radius: var(--radius-full); background: var(--bg-raised); color: var(--text-secondary); font-size: var(--font-size-xs); }
.cert-row code { overflow: hidden; color: var(--text-tertiary); font-size: var(--font-size-xs); text-overflow: ellipsis; white-space: nowrap; }
.actions {
  flex-wrap: wrap;
}
header {
  justify-content: space-between;
}
h2,
h3 {
  margin: 0;
  color: var(--text-primary);
}
h3 {
  font-size: var(--font-size-md);
}
.hint {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}
.panel {
  display: grid;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}
.field-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}
.span-full {
  grid-column: 1 / -1;
}
.field-grid label,
.pem-field {
  display: grid;
  gap: 5px;
  min-width: 0;
}
.field-grid label span,
.pem-field span {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
}
input,
textarea {
  width: 100%;
  min-width: 0;
  box-sizing: border-box;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  padding: 8px 10px;
  font: inherit;
}
textarea {
  font-family: var(--font-mono, monospace);
  font-size: var(--font-size-xs);
  resize: vertical;
}
/* The picker is opened by the button next to it; only the file dialog is
   wanted, never the browser's own control. */
.file-input {
  display: none;
}
.status,
.empty {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
/* Labels in sentence case at body size, like every other form. */
</style>

<style scoped src="./settingsRows.css"></style>
