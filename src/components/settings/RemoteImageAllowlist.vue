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
    <h3>{{ $t('settings.images.title') }}</h3>
    <p class="hint">{{ $t('settings.images.hint') }}</p>

    <SegmentedControl
      :options="modeOptions"
      :model-value="policy.state.mode"
      :aria-label="$t('settings.images.modeAria')"
      @update:model-value="setMode"
    />
    <p class="hint" :class="{ 'hint--warning': policy.state.mode === 'all' }">
      {{ modeHint }}
    </p>

    <div class="add">
      <input
        v-model="newEntry"
        :placeholder="$t('settings.images.entryPlaceholder')"
        @keydown.enter="add"
      />
      <AppButton variant="primary" @click="add">{{ $t('settings.images.allow') }}</AppButton>
    </div>

    <ul v-if="domains.length > 0 || senders.length > 0" class="list">
      <li v-for="domain in domains" :key="`d:${domain}`">
        <span>
          <strong>@{{ domain }}</strong>
          <small>· {{ $t('settings.images.wholeDomain') }}</small>
        </span>
        <AppButton variant="secondary" size="sm" @click="policy.revokeDomain(domain)">
          {{ $t('common.remove') }}
        </AppButton>
      </li>
      <li v-for="sender in senders" :key="`s:${sender}`">
        <span>
          <strong>{{ sender }}</strong>
          <small>· {{ $t('settings.images.sender') }}</small>
        </span>
        <AppButton variant="secondary" size="sm" @click="policy.revoke(sender)">
          {{ $t('common.remove') }}
        </AppButton>
      </li>
    </ul>
    <p v-else class="empty">{{ $t('settings.images.empty') }}</p>
  </section>
</template>

<style scoped>
section { display: grid; gap: 12px; }
h3 { margin: 0; color: var(--text-primary); }
.hint { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
.hint--warning { color: var(--accent-amber); }
.add { display: flex; gap: 8px; }
.add input { flex: 1; border: 1px solid var(--border-color); background: var(--bg-raised); border-radius: var(--radius-sm); padding: 8px 10px; color: var(--text-primary); font-family: inherit; }
.list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.list li { display: flex; justify-content: space-between; align-items: center; background: var(--bg-raised); padding: 8px 12px; border-radius: var(--radius-sm); color: var(--text-primary); font-size: var(--font-size-sm); }
.list small { color: var(--text-tertiary); margin-left: 6px; }
.empty { color: var(--text-tertiary); font-size: var(--font-size-sm); margin: 0; }
</style>
