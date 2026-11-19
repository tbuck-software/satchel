<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import { backendPushDevices, type BackendPushDevices } from '@/services/backend';
import { writeClipboardText } from '@/services/clipboard';

const { t } = useI18n();
const state = ref<BackendPushDevices>();
const busy = ref(false);
const error = ref('');
const copied = ref(false);
let timer: ReturnType<typeof setTimeout>;
let mounted = true;
let version = 0;

async function update(action: Parameters<typeof backendPushDevices>[0], id?: string) {
  clearTimeout(timer);
  const current = ++version;
  busy.value = action !== 'status';
  error.value = '';
  if (action === 'start') copied.value = false;
  try {
    const next = await backendPushDevices(action, id);
    if (!mounted || current !== version) return;
    state.value = next;
    if (['searching', 'found', 'connecting'].includes(next.phase) ||
        next.devices.some((device) => device.state === 'paired')) {
      timer = setTimeout(() => void update('status'), next.phase === 'idle' ? 5000 : 1000);
    }
  } catch {
    if (current === version) error.value = t('settings.pushDevices.failed');
  } finally {
    if (current === version) busy.value = false;
  }
}

async function copy() {
  if (!state.value?.command) return;
  try {
    await writeClipboardText(state.value.command);
    copied.value = true;
  } catch {
    error.value = t('settings.pushDevices.copyFailed');
  }
}

onMounted(() => void update('status'));
onBeforeUnmount(() => {
  mounted = false;
  clearTimeout(timer);
  if (state.value && ['searching', 'found'].includes(state.value.phase))
    void backendPushDevices('cancel').catch(() => {});
});
</script>

<template>
  <section class="devices">
    <header>
      <h3 class="settings-page-title">{{ t('settings.pushDevices.title') }}</h3>
      <p>{{ t('settings.pushDevices.description') }}</p>
    </header>
    <div v-for="device in state?.devices" :key="device.id" class="setting-row">
      <div class="setting-info">
        <strong>{{ device.name }}</strong>
        <span>{{ device.state === 'verified' ? t('settings.pushDevices.verified') : t('settings.pushDevices.paired') }}</span>
      </div>
      <AppButton :disabled="busy" @click="update('remove', device.id)">{{ t('settings.pushDevices.remove') }}</AppButton>
    </div>
    <template v-if="state?.phase === 'searching'">
      <p>{{ t('settings.pushDevices.instruction') }}</p>
      <code>{{ state.command }}</code>
      <div class="actions">
        <AppButton variant="primary" @click="copy">{{ copied ? t('settings.pushDevices.copied') : t('settings.pushDevices.copy') }}</AppButton>
        <AppButton @click="update('cancel')">{{ t('common.cancel') }}</AppButton>
      </div>
    </template>
    <template v-else-if="state?.phase === 'found'">
      <p role="status">{{ t('settings.pushDevices.found', { name: state.candidate }) }}</p>
      <div class="actions">
        <AppButton variant="primary" :disabled="busy" @click="update('confirm')">{{ t('settings.pushDevices.connect') }}</AppButton>
        <AppButton :disabled="busy" @click="update('cancel')">{{ t('common.cancel') }}</AppButton>
      </div>
    </template>
    <p v-else-if="state?.phase === 'connecting'" role="status">{{ t('settings.pushDevices.connecting') }}</p>
    <template v-else>
      <p v-if="state?.phase === 'failed'" role="alert">{{ t('settings.pushDevices.failed') }}</p>
      <p v-if="state?.phase === 'expired'" role="status">{{ t('settings.pushDevices.expired') }}</p>
      <AppButton :disabled="busy" @click="update('start')">{{ t('settings.pushDevices.add') }}</AppButton>
    </template>
    <p v-if="error" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.devices { display: grid; justify-items: start; gap: var(--space-md); }
.devices :is(header, .setting-row, code) { width: 100%; }
.devices p { margin: 0; color: var(--text-secondary); }
.devices code { overflow-wrap: anywhere; user-select: all; font-family: var(--font-mono); }
.actions { display: flex; gap: var(--space-sm); flex-wrap: wrap; }
.devices :deep(.app-button) { min-height: calc(2 * var(--space-xl)); }
</style>
