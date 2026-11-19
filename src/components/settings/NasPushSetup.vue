<script setup lang="ts">
import { ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import { writeClipboardText } from '@/services/clipboard';

const { t } = useI18n();
const installer = 'https://raw.githubusercontent.com/tbuck-software/satchel/main/tooling/push-device/install.sh';
const command = 'curl -fsSL ' + installer + ' | sh -s -- visible';
const copied = ref(false);
const copyFailed = ref(false);

async function copy() {
  try {
    await writeClipboardText(command);
    copied.value = true;
    copyFailed.value = false;
  } catch {
    copyFailed.value = true;
  }
}
</script>

<template>
  <details class="nas-setup">
    <summary>{{ t('settings.relay.nasTitle') }}</summary>
    <ol>
      <li>{{ t('settings.relay.nasLogin') }}
        <AppButton variant="secondary" @click="copy">{{ copied ? t('settings.pushDevices.copied') : t('settings.relay.nasCopy') }}</AppButton>
        <code v-if="copyFailed">{{ command }}</code>
      </li>
      <li>{{ t('settings.relay.nasMail') }}</li>
      <li>{{ t('settings.relay.nasPair') }}</li>
    </ol>
  </details>
</template>

<style scoped>
.nas-setup { min-width: 0; color: var(--text-secondary); font-size: var(--font-size-sm); line-height: 1.5; }
.nas-setup summary { padding-block: var(--space-sm); cursor: pointer; color: var(--text-primary); font-weight: var(--weight-semibold); }
.nas-setup ol { display: grid; gap: var(--space-sm); padding-inline-start: var(--space-xl); margin: var(--space-xs) 0 0; list-style: decimal; }
.nas-setup :deep(.app-button) { display: flex; margin-block: var(--space-sm) var(--space-xs); }
.nas-setup code { display: block; user-select: all; overflow-wrap: anywhere; font-family: var(--font-mono); }
@media (pointer: coarse) {
  .nas-setup summary, .nas-setup :deep(.app-button) { min-height: calc(2 * var(--space-xl)); }
}
</style>
