<script setup lang="ts">
/**
 * How full the account's mailbox is on the server, as it last reported. Shown
 * only when the server sets a limit: Gmail and Outlook keep theirs behind
 * scopes Satchel does not request, POP3 has none, and some IMAP servers do
 * not offer the QUOTA extension.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import AppButton from '@/components/ui/AppButton.vue';
import UsageMeter from '@/components/ui/UsageMeter.vue';
import { useAccountQuotaStore } from '@/stores/accountQuota';
import { useUiStore } from '@/stores/ui';
import {
  formatQuotaAmounts,
  formatQuotaPercent,
  quotaFigures,
  quotaLevel,
  quotaLevelForRatio,
  type QuotaFigure,
  type QuotaLevel,
} from '@/utils/mailboxQuota';
import { formatMoment } from '@/utils/listTime';

const props = defineProps<{ accountId: string }>();

const { t, locale } = useI18n();
const quotaStore = useAccountQuotaStore();
const router = useRouter();
const ui = useUiStore();

const quota = computed(() => quotaStore.quotaFor(props.accountId));
const level = computed(() => quotaLevel(quota.value));

const TONES: Record<QuotaLevel, 'neutral' | 'warning' | 'danger'> = { ok: 'neutral', warning: 'warning', critical: 'danger' };

const rows = computed(() => quotaFigures(quota.value).map((entry: QuotaFigure) => {
  const amounts = formatQuotaAmounts(entry, locale.value);
  return {
    kind: entry.kind,
    used: entry.used,
    limit: entry.limit,
    percent: formatQuotaPercent(entry.ratio, locale.value),
    tone: TONES[quotaLevelForRatio(entry.ratio)],
    label: entry.kind === 'storage' ? t('accountQuota.storageMeter') : t('accountQuota.messagesMeter'),
    text: entry.kind === 'storage' ? t('accountQuota.storageUsed', amounts) : t('accountQuota.messagesUsed', amounts),
  };
}));

/** The cleanup assistant, scoped to this account: the quickest way to make room. */
function openCleanup() {
  ui.closeSettings();
  void router.push({ name: 'cleanup', query: { account: props.accountId } });
}

const asOf = computed(() => {
  const moment = formatMoment(quota.value?.fetched_at);
  return moment ? t('accountQuota.asOf', { time: moment }) : null;
});
</script>

<template>
  <section v-if="rows.length" class="quota-settings" :aria-label="t('accountQuota.title')">
    <div class="quota-settings__heading">
      <span class="quota-settings__title">{{ t('accountQuota.title') }}</span>
      <span v-if="asOf" class="quota-settings__muted">{{ asOf }}</span>
    </div>
    <div v-for="row in rows" :key="row.kind" class="quota-settings__row" :data-quota-kind="row.kind">
      <UsageMeter :value="row.used" :max="row.limit" :label="row.label" :value-text="row.text" :tone="row.tone" />
      <p class="quota-settings__line">
        <span>{{ row.text }}</span>
        <span class="quota-settings__muted">{{ row.percent }}</span>
      </p>
    </div>
    <p v-if="level === 'warning' || level === 'critical'" class="quota-settings__note" :class="`quota-settings__note--${level}`">
      {{ level === 'critical' ? t('accountQuota.critical') : t('accountQuota.warning') }}
      {{ t('accountQuota.freeUp') }}
    </p>
    <div v-if="level === 'warning' || level === 'critical'">
      <AppButton class="quota-settings__cleanup" variant="secondary" size="sm" @click="openCleanup">
        {{ t('cleanup.openForAccount') }}
      </AppButton>
    </div>
  </section>
</template>

<style scoped>
.quota-settings { display: grid; gap: var(--space-sm); flex-basis: 100%; padding-top: var(--space-md); border-top: 1px solid var(--border-color-muted); }
.quota-settings__heading { display: flex; flex-wrap: wrap; justify-content: space-between; gap: var(--space-sm); }
.quota-settings__title { color: var(--text-primary); font-size: var(--font-size-sm); font-weight: var(--weight-semibold); }
.quota-settings__row { display: grid; gap: var(--space-xs); }
.quota-settings__line { display: flex; flex-wrap: wrap; justify-content: space-between; gap: var(--space-sm); margin: 0; color: var(--text-secondary); font-size: var(--font-size-xs); font-variant-numeric: tabular-nums; }
.quota-settings__muted { color: var(--text-tertiary); font-size: var(--font-size-xs); }
.quota-settings__note { margin: 0; color: var(--text-secondary); font-size: var(--font-size-xs); }
.quota-settings__note--warning { color: var(--accent-amber-soft); }
.quota-settings__note--critical { color: var(--accent-red-soft); }
</style>
