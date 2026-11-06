<script setup lang="ts">
/**
 * Settings → Rules → Sender cleanup: the recurring sweeps set from a mail or
 * the cleanup assistant. Listed and removable here; created where the sender
 * is, because a rule needs a sender and an account to mean anything.
 */
import { computed, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import AppButton from '@/components/ui/AppButton.vue';
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import { useConfirm } from '@/composables/useConfirm';
import { useToast } from '@/composables/useToast';
import { useAccountsStore } from '@/stores/accounts';
import { useSweepRulesStore } from '@/stores/sweepRules';
import { useUiStore } from '@/stores/ui';
import type { SweepRule as BackendSweepRule } from '@/stores/sweepRules';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { formatMoment } from '@/utils/listTime';
import { describeSweepRule } from '@/utils/sweepRules';

const { t } = useI18n();
const router = useRouter();
const rules = useSweepRulesStore();
const accounts = useAccountsStore();
const ui = useUiStore();
const toast = useToast();
const { ask } = useConfirm();

onMounted(() => void rules.load());

const multipleAccounts = computed(() => accounts.accounts.filter((account) => account.kind === 'email').length > 1);

function describe(rule: BackendSweepRule): string {
  return describeSweepRule(rule, t);
}

function lastRun(rule: BackendSweepRule): string {
  if (!rule.last_run_at) return t('settings.sweep.neverRun');
  return t('settings.sweep.lastRun', {
    when: formatMoment(rule.last_run_at) ?? rule.last_run_at,
    count: rule.last_swept_count,
  }, rule.last_swept_count);
}

function accountEmail(id: string): string {
  return accounts.accounts.find((account) => account.id === id)?.email ?? '';
}

async function remove(rule: BackendSweepRule) {
  const confirmed = await ask({
    title: t('settings.sweep.removeTitle', { sender: rule.sender_email }),
    body: t('settings.sweep.removeBody'),
    confirmLabel: t('settings.sweep.remove'),
    tone: 'danger',
  });
  if (!confirmed) return;
  try {
    await rules.remove(rule.id);
    toast.success(t('sweep.rule.removed', { sender: rule.sender_email }));
  } catch (error) {
    toast.error(t('sweep.rule.removeFailed', { error: backendErrorLabel(error) }));
  }
}

function openCleanup() {
  ui.closeSettings();
  void router.push({ name: 'cleanup' });
}
</script>

<template>
  <section>
    <h3 class="settings-page-title">{{ $t('settings.sweep.title') }}</h3>
    <p class="hint">{{ $t('settings.sweep.purpose') }}</p>

    <div class="setting-group list">
      <CollectionLoadState
        :loading="rules.loading"
        :failed="rules.loadFailed"
        :has-content="rules.items.length > 0"
        :error-label="$t('settings.sweep.loadFailed')"
        @retry="rules.load()"
      />
      <div v-for="rule in rules.items" :key="rule.id" class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ rule.sender_email }}</span>
          <span class="setting-hint">
            {{ describe(rule) }}<template v-if="multipleAccounts"> · {{ accountEmail(rule.account_id) }}</template>
          </span>
          <span class="setting-hint">{{ lastRun(rule) }}</span>
        </div>
        <AppButton variant="ghost" size="sm" @click="remove(rule)">{{ $t('settings.sweep.remove') }}</AppButton>
      </div>
      <div v-if="!rules.loading && rules.items.length === 0" class="setting-row setting-row--single">
        <span class="setting-hint">{{ $t('settings.sweep.empty') }}</span>
      </div>
    </div>

    <AppButton class="sweep-open" variant="secondary" size="sm" @click="openCleanup">
      {{ $t('commands.openCleanup') }}
    </AppButton>
  </section>
</template>

<style scoped>
.sweep-open {
  margin-top: var(--space-sm);
}
</style>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
