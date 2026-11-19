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
    <div v-for="device in state?.devices" :key="device.id" class="device-row">
      <div class="setting-info">
        <strong>{{ device.name }}</strong>
        <span>{{ device.state === 'verified' ? t('settings.pushDevices.verified') : t('settings.pushDevices.paired') }}</span>
      </div>
      <AppButton variant="ghost" :disabled="busy" @click="update('remove', device.id)">{{ t('settings.pushDevices.remove') }}</AppButton>
    </div>
    <template v-if="state?.phase === 'searching'">
      <p>{{ t('settings.pushDevices.instruction') }}</p>
      <code>{{ state.command }}</code>
      <div class="actions">
        <AppButton @click="update('cancel')">{{ t('common.cancel') }}</AppButton>
        <AppButton variant="primary" @click="copy">{{ copied ? t('settings.pushDevices.copied') : t('settings.pushDevices.copy') }}</AppButton>
      </div>
    </template>
    <template v-else-if="state?.phase === 'found'">
      <p role="status">{{ t('settings.pushDevices.found', { name: state.candidate }) }}</p>
      <div class="actions">
        <AppButton :disabled="busy" @click="update('cancel')">{{ t('common.cancel') }}</AppButton>
        <AppButton variant="primary" :disabled="busy" @click="update('confirm')">{{ t('settings.pushDevices.connect') }}</AppButton>
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
.devices { display: grid; gap: var(--space-md); min-width: 0; }
.devices header, .setting-info { display: grid; gap: var(--space-xs); min-width: 0; }
.devices p { margin: 0; color: var(--text-secondary); font-size: var(--font-size-md); line-height: 1.5; }
.device-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  padding-block: var(--space-sm);
  border-bottom: 1px solid var(--border-color-muted);
}
.setting-info strong { overflow-wrap: anywhere; font-size: var(--font-size-md); }
.setting-info span { color: var(--text-secondary); font-size: var(--font-size-xs); }
.devices code {
  display: block;
  min-width: 0;
  overflow-x: auto;
  padding: var(--space-md);
  border-radius: var(--radius-sm);
  background: var(--bg-inset);
  user-select: all;
  white-space: pre;
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
}
.devices > :deep(.app-button) { justify-self: start; }
.actions { display: flex; justify-content: flex-end; gap: var(--space-sm); flex-wrap: wrap; }
.devices :deep(.app-button) { min-height: calc(2 * var(--space-xl)); }
</style>
