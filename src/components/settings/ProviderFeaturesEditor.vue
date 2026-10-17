<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import {
  backendCreateProviderLabel,
  backendListProviderLabels,
  backendRefreshProviderLabels,
  type BackendProviderLabel,
} from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';

const { t } = useI18n();
const accounts = useAccountsStore();
const accountId = ref('');
const labels = ref<BackendProviderLabel[]>([]);
const name = ref('');
const color = ref('');
const status = ref('');
const busy = ref(false);

const account = computed(() => accounts.emailAccounts.find((item) => item.id === accountId.value));

onMounted(async () => {
  await accounts.load();
  accountId.value = accounts.emailAccounts.find((item) => item.id === accounts.activeAccountId)?.id
    || accounts.emailAccounts[0]?.id
    || '';
  await loadCached();
});

watch(accountId, () => {
  labels.value = [];
  status.value = '';
  void loadCached();
});

/**
 * Show what is already stored before anyone presses Refresh. Refresh talks to
 * the provider; opening the pane must not.
 */
async function loadCached() {
  if (!accountId.value) return;
  try {
    labels.value = await backendListProviderLabels(accountId.value);
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
}

async function refresh() {
  if (!accountId.value) return;
  busy.value = true;
  status.value = '';
  try {
    labels.value = await backendRefreshProviderLabels(accountId.value);
    status.value = t('settings.providerFeatures.refreshed', { count: labels.value.length });
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}

async function create() {
  if (!accountId.value || !name.value.trim()) return;
  busy.value = true;
  status.value = '';
  try {
    const label = await backendCreateProviderLabel({
      accountId: accountId.value,
      name: name.value.trim(),
      color: color.value.trim() || null,
    });
    labels.value = [label, ...labels.value.filter((item) => item.id !== label.id)];
    name.value = '';
    color.value = '';
    status.value = t('settings.providerFeatures.created');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section>
    <h3>{{ $t('settings.providerFeatures.title') }}</h3>
    <p class="hint">{{ $t('settings.providerFeatures.hint') }}</p>

    <label class="field">
      <span>{{ $t('settings.providerFeatures.account') }}</span>
      <select v-model="accountId">
        <option v-for="a in accounts.emailAccounts" :key="a.id" :value="a.id">
          {{ a.displayName }} &lt;{{ a.email }}&gt;
        </option>
      </select>
    </label>

    <div class="create-row">
      <input v-model="name" :placeholder="$t('settings.providerFeatures.namePlaceholder')" />
      <input v-model="color" :placeholder="$t('settings.providerFeatures.colorPlaceholder')" />
      <AppButton variant="primary" :disabled="busy || !name.trim()" @click="create">
        {{ $t('common.create') }}
      </AppButton>
      <AppButton variant="secondary" :disabled="busy" @click="refresh">
        {{ $t('common.refresh') }}
      </AppButton>
    </div>

    <p v-if="account" class="hint">
      {{ $t('settings.providerFeatures.provider', { provider: account.provider }) }}
    </p>
    <ul v-if="labels.length > 0">
      <li v-for="label in labels" :key="label.id">
        <span class="swatch" :style="{ background: label.color || 'var(--bg-raised)' }" />
        <strong>{{ label.name }}</strong>
        <small>{{ label.kind || label.provider }} · {{ label.remote_id }}</small>
      </li>
    </ul>
    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
section { display: grid; gap: 14px; }
h3 { margin: 0; color: var(--text-primary); }
.hint, .status { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
.field { display: grid; gap: 4px; font-size: var(--font-size-sm); color: var(--text-secondary); }
.field select,
.create-row input {
  border: 1px solid var(--border-color);
  background: var(--bg-raised);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  color: var(--text-primary);
  font-family: inherit;
}
.create-row { display: grid; grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) auto auto; gap: 8px; }
ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
li { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 8px; align-items: center; padding: 8px 10px; background: var(--bg-raised); border-radius: var(--radius-sm); }
li small { grid-column: 2; color: var(--text-tertiary); }
.swatch { width: 12px; height: 12px; border-radius: 50%; border: 1px solid var(--border-color); }
</style>
