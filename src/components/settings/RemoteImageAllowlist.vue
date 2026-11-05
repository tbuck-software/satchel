<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useImagePolicy, type ImagePolicyMode } from '@/composables/useImagePolicy';
import AppButton from '@/components/ui/AppButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';

const { t } = useI18n();
const policy = useImagePolicy();
const newEntry = ref('');

const modeOptions = computed(() => [
  {
    value: 'allowlist',
    label: t('settings.images.modeAllowlist'),
    title: t('settings.images.modeAllowlistTitle'),
  },
  {
    value: 'accepted',
    label: t('settings.images.modeAccepted'),
    title: t('settings.images.modeAcceptedTitle'),
  },
  {
    value: 'all',
    label: t('settings.images.modeAll'),
    title: t('settings.images.modeAllTitle'),
  },
]);

const modeHint = computed(
  () =>
    ({
      allowlist: t('settings.images.modeAllowlistHint'),
      accepted: t('settings.images.modeAcceptedHint'),
      all: t('settings.images.modeAllHint'),
    })[policy.state.mode],
);

function setMode(value: string) {
  policy.setMode(value as ImagePolicyMode);
}

const senders = computed(() => Array.from(policy.state.allowedSenders).sort());
const domains = computed(() => Array.from(policy.state.allowedDomains).sort());

function add() {
  const raw = newEntry.value.trim();
  if (!raw) return;
  // `alice@example.com` is a sender; `@example.com` or `example.com` a domain.
  if (raw.includes('@') && !raw.startsWith('@')) {
    policy.allow(raw);
  } else {
    policy.allowDomain(raw);
  }
  newEntry.value = '';
}
</script>

<template>
  <section>
    <h3 class="settings-page-title">{{ $t('settings.sections.images.label') }}</h3>
    <p class="hint">{{ $t('settings.images.hint') }}</p>

    <div class="setting-group">
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.images.modeLabel') }}</span>
          <span class="setting-hint" :class="{ 'hint--warning': policy.state.mode === 'all' }">{{ modeHint }}</span>
        </div>
        <SegmentedControl
          :options="modeOptions"
          :model-value="policy.state.mode"
          :aria-label="$t('settings.images.modeAria')"
          @update:model-value="setMode"
        />
      </div>
    </div>

    <div class="setting-group list">
      <form class="setting-row add-row" @submit.prevent="add">
        <input v-model="newEntry" :placeholder="$t('settings.images.entryPlaceholder')" :aria-label="$t('settings.images.entryPlaceholder')" />
        <AppButton type="submit" variant="secondary" size="sm" :disabled="!newEntry.trim()">{{ $t('settings.images.allow') }}</AppButton>
      </form>
      <div v-for="domain in domains" :key="`d:${domain}`" class="setting-row">
        <div class="setting-info">
          <span class="setting-label">@{{ domain }}</span>
          <span class="setting-hint">{{ $t('settings.images.wholeDomain') }}</span>
        </div>
        <AppButton variant="ghost" size="sm" @click="policy.revokeDomain(domain)">{{ $t('common.remove') }}</AppButton>
      </div>
      <div v-for="sender in senders" :key="`s:${sender}`" class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ sender }}</span>
          <span class="setting-hint">{{ $t('settings.images.sender') }}</span>
        </div>
        <AppButton variant="ghost" size="sm" @click="policy.revoke(sender)">{{ $t('common.remove') }}</AppButton>
      </div>
      <div v-if="domains.length === 0 && senders.length === 0" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.images.empty') }}</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.setting-row.add-row { grid-template-columns: minmax(0, 1fr) auto; }
.add-row input { width: 100%; max-width: none; }
.hint--warning { color: var(--accent-amber-soft); }
</style>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
