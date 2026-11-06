<script setup lang="ts">
/**
 * Translation: the engine, its API key, the language mail goes into, which
 * accounts may be sent, what is sent, and the log of what was (ADR 0016).
 *
 * The key is write-only from here. Saving sends it to the backend, which
 * stores it encrypted; the page only ever learns whether one is stored and
 * which DeepL plan it belongs to.
 */
import { computed, onMounted, ref, shallowRef } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import UsageMeter from '@/components/ui/UsageMeter.vue';
import {
  backendCheckTranslationEngine,
  backendClearOutboundRequests,
  backendListOutboundRequests,
  backendListTranslationLanguages,
  backendSaveTranslationSettings,
  backendSetTranslationApiKey,
  type BackendOutboundRequest,
  type BackendTranslationLanguage,
  type BackendTranslationSettings,
  type BackendTranslationUsage,
} from '@/services/backend/translation';
import { useAccountsStore } from '@/stores/accounts';
import { uiTargetLanguage, useTranslationStore } from '@/stores/translation';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import { languageName } from '@/utils/languageName';
import { formatMoment } from '@/utils/listTime';
import { currentLocale } from '@/i18n/runtime';

const ENGINE = 'deepl' as const;

const { t, n, locale } = useI18n();
const store = useTranslationStore();
const accounts = useAccountsStore();
const toast = useToast();
const confirm = useConfirm();

const settings = computed<BackendTranslationSettings | null>(() => store.settings);
const deepl = computed(() => settings.value?.engines.find((engine) => engine.id === ENGINE) ?? null);
const keyDraft = ref('');
const keyBusy = ref(false);
const testBusy = ref(false);
const usage = shallowRef<BackendTranslationUsage | null>(null);
const languages = shallowRef<BackendTranslationLanguage[]>([]);
const log = shallowRef<BackendOutboundRequest[]>([]);
const openEntries = ref(new Set<number>());

function fail(error: unknown) {
  toast.error(error instanceof Error ? error.message : String(error));
}

async function loadLanguages() {
  try {
    languages.value = await backendListTranslationLanguages(ENGINE);
  } catch (error) {
    fail(error);
  }
}

async function loadLog() {
  try {
    log.value = await backendListOutboundRequests();
  } catch (error) {
    fail(error);
  }
}

onMounted(async () => {
  await store.reload();
  await Promise.all([loadLanguages(), loadLog()]);
});

const engineOptions = computed(() => [
  { value: 'off', label: t('settings.translation.engineOff') },
  { value: ENGINE, label: t('settings.translation.engineDeepl') },
]);

async function save(patch: Partial<{ engine: string | null; target_language: string | null; log_sent_text: boolean }>) {
  const current = settings.value;
  try {
    store.apply(await backendSaveTranslationSettings({
      engine: current?.engine ?? null,
      target_language: current?.target_language ?? null,
      log_sent_text: current?.log_sent_text ?? false,
      ...patch,
    }));
  } catch (error) {
    fail(error);
  }
}

function setEngine(value: string) {
  void save({ engine: value === 'off' ? null : value });
}

const planLabel = computed(() => (
  deepl.value?.plan === 'free' ? t('settings.translation.planFree') : t('settings.translation.planPro')
));

async function saveKey() {
  const key = keyDraft.value.trim();
  if (!key || keyBusy.value) return;
  keyBusy.value = true;
  try {
    store.apply(await backendSetTranslationApiKey(ENGINE, key));
    keyDraft.value = '';
    usage.value = null;
    toast.success(t('settings.translation.keySavedToast'));
    // A key usually means "switch it on"; the person can still pick Off.
    if (!settings.value?.engine) await save({ engine: ENGINE });
    await loadLanguages();
  } catch (error) {
    fail(error);
  } finally {
    keyBusy.value = false;
  }
}

async function removeKey() {
  keyBusy.value = true;
  try {
    store.apply(await backendSetTranslationApiKey(ENGINE, null));
    usage.value = null;
    toast.success(t('settings.translation.keyRemovedToast'));
  } catch (error) {
    fail(error);
  } finally {
    keyBusy.value = false;
  }
}

async function test() {
  testBusy.value = true;
  try {
    usage.value = await backendCheckTranslationEngine(ENGINE);
  } catch (error) {
    usage.value = null;
    fail(error);
  } finally {
    testBusy.value = false;
  }
}

const usageText = computed(() => {
  const current = usage.value;
  if (!current) return '';
  return current.limit === null
    ? t('settings.translation.usageUnlimited', { used: n(current.used) })
    : t('settings.translation.usage', { used: n(current.used), limit: n(current.limit) });
});
const usageTone = computed(() => {
  const current = usage.value;
  if (!current?.limit) return 'neutral' as const;
  const ratio = current.used / current.limit;
  return ratio >= 0.95 ? 'danger' as const : ratio >= 0.8 ? 'warning' as const : 'neutral' as const;
});

const FOLLOW_UI = '';
const targetOptions = computed(() => {
  const follow = {
    value: FOLLOW_UI,
    label: t('settings.translation.targetFollowUi', {
      language: languageName(uiTargetLanguage(currentLocale()), locale.value),
    }),
  };
  const named = languages.value
    .map((language) => ({ value: language.code, label: languageName(language.code, locale.value) || language.name || language.code }))
    .sort((a, b) => a.label.localeCompare(b.label, locale.value));
  return [follow, ...named];
});

function setTarget(event: Event) {
  const value = (event.target as HTMLSelectElement).value;
  void save({ target_language: value === FOLLOW_UI ? null : value });
}

const mailAccounts = computed(() => accounts.emailAccounts);

function consented(accountId: string): boolean {
  return Boolean(settings.value?.consents.some((consent) => consent.engine === ENGINE && consent.account_id === accountId));
}

async function setConsent(accountId: string, granted: boolean) {
  try {
    if (!settings.value?.engine) await save({ engine: ENGINE });
    await store.setConsent(accountId, granted);
  } catch (error) {
    fail(error);
  }
}

async function setKeepText(value: boolean) {
  await save({ log_sent_text: value });
  await loadLog();
}

async function clearLog() {
  const yes = await confirm.ask({
    title: t('settings.translation.logClearConfirm'),
    body: t('settings.translation.logClearConfirmBody'),
    confirmLabel: t('settings.translation.logClear'),
    tone: 'danger',
  });
  if (!yes) return;
  try {
    await backendClearOutboundRequests();
    log.value = [];
  } catch (error) {
    fail(error);
  }
}

function accountLabel(accountId: string | null): string {
  if (!accountId) return '';
  const account = accounts.accounts.find((entry) => entry.id === accountId);
  return account?.email || account?.displayName || '';
}

function entryLine(entry: BackendOutboundRequest): string {
  const provider = entry.provider === ENGINE ? t('settings.translation.engineDeepl') : entry.provider;
  return t('settings.translation.logEntry', {
    provider,
    units: t('settings.translation.logUnits', { count: n(entry.units) }),
  });
}

function entryOutcome(entry: BackendOutboundRequest): string {
  return entry.outcome === 'ok' ? t('settings.translation.logOk') : t(entry.outcome);
}

function toggleEntry(id: number) {
  const next = new Set(openEntries.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  openEntries.value = next;
}
</script>

<template>
  <section class="translation-settings">
    <header>
      <h3 class="settings-page-title">{{ $t('settings.translation.title') }}</h3>
      <p class="hint">{{ $t('settings.translation.purpose') }}</p>
    </header>

    <div class="setting-group">
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.translation.engineLabel') }}</span>
          <span class="setting-hint">{{ $t('settings.translation.engineHint') }}</span>
        </div>
        <SegmentedControl
          :options="engineOptions"
          :model-value="settings?.engine ?? 'off'"
          :aria-label="$t('settings.translation.engineAria')"
          @update:model-value="setEngine"
        />
      </div>

      <form class="setting-row setting-row--single translation-settings__key" @submit.prevent="saveKey">
        <label class="field">
          <span class="setting-label">{{ $t('settings.translation.keyLabel') }}</span>
          <span class="setting-hint">
            {{ deepl?.has_key ? $t('settings.translation.keySaved', { plan: planLabel }) : $t('settings.translation.keyHint') }}
          </span>
          <input
            v-model="keyDraft"
            type="password"
            autocomplete="off"
            spellcheck="false"
            :placeholder="$t('settings.translation.keyPlaceholder')"
            data-testid="translation-key-input"
          />
        </label>
        <p class="setting-hint">{{ $t('settings.translation.keyEncrypted') }}</p>
        <div class="translation-settings__actions">
          <AppButton
            v-if="deepl?.has_key"
            variant="ghost"
            size="sm"
            :disabled="keyBusy"
            @click="removeKey"
          >
            {{ $t('settings.translation.removeKey') }}
          </AppButton>
          <AppButton type="submit" variant="primary" size="sm" :disabled="keyBusy || !keyDraft.trim()">
            {{ deepl?.has_key ? $t('settings.translation.replaceKey') : $t('settings.translation.saveKey') }}
          </AppButton>
        </div>
      </form>

      <div v-if="deepl?.has_key" class="setting-row setting-row--single translation-settings__test">
        <div class="translation-settings__test-head">
          <div class="setting-info">
            <span class="setting-label">{{ $t('settings.translation.test') }}</span>
            <span class="setting-hint">{{ $t('settings.translation.testHint') }}</span>
          </div>
          <AppButton variant="secondary" size="sm" :disabled="testBusy" data-testid="translation-test" @click="test">
            {{ testBusy ? $t('settings.translation.testing') : $t('settings.translation.test') }}
          </AppButton>
        </div>
        <div v-if="usage" class="translation-settings__usage" data-testid="translation-usage">
          <UsageMeter
            v-if="usage.limit"
            :value="usage.used"
            :max="usage.limit"
            :label="$t('settings.translation.usageMeter')"
            :value-text="usageText"
            :tone="usageTone"
          />
          <span class="setting-hint">{{ usageText }}</span>
        </div>
      </div>

      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">{{ $t('settings.translation.targetLabel') }}</span>
          <span class="setting-hint">{{ $t('settings.translation.targetHint') }}</span>
        </div>
        <select
          :value="settings?.target_language ?? FOLLOW_UI"
          :aria-label="$t('settings.translation.targetLabel')"
          data-testid="translation-target"
          @change="setTarget"
        >
          <option v-for="option in targetOptions" :key="option.value" :value="option.value">{{ option.label }}</option>
        </select>
      </div>
    </div>

    <section class="translation-settings__block" :aria-label="$t('settings.translation.sentTitle')">
      <h4>{{ $t('settings.translation.sentTitle') }}</h4>
      <p class="hint">{{ $t('settings.translation.sentBody') }}</p>
      <p class="hint">{{ $t('settings.translation.sentCache') }}</p>
      <p class="hint">{{ $t('settings.translation.sentTerms') }}</p>
    </section>

    <section class="translation-settings__block">
      <h4>{{ $t('settings.translation.consentTitle') }}</h4>
      <p class="hint">{{ $t('settings.translation.consentHint') }}</p>
      <div class="setting-group">
        <div v-for="account in mailAccounts" :key="account.id" class="setting-row">
          <div class="setting-info">
            <span class="setting-label">{{ account.displayName || account.email }}</span>
            <span v-if="account.displayName" class="setting-hint">{{ account.email }}</span>
          </div>
          <ToggleSwitch
            :model-value="consented(account.id)"
            :label="$t('settings.translation.consentAria', { account: account.email })"
            :data-testid="`translation-consent-${account.id}`"
            @update:model-value="setConsent(account.id, $event)"
          />
        </div>
        <div v-if="mailAccounts.length === 0" class="setting-row setting-row--single">
          <span class="setting-hint">{{ $t('settings.translation.noAccounts') }}</span>
        </div>
      </div>
    </section>

    <section class="translation-settings__block">
      <h4>{{ $t('settings.translation.logTitle') }}</h4>
      <p class="hint">{{ $t('settings.translation.logHint') }}</p>
      <div class="setting-group">
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">{{ $t('settings.translation.logKeepText') }}</span>
            <span class="setting-hint">{{ $t('settings.translation.logKeepTextHint') }}</span>
          </div>
          <ToggleSwitch
            :model-value="settings?.log_sent_text ?? false"
            :label="$t('settings.translation.logKeepText')"
            @update:model-value="setKeepText"
          />
        </div>
        <div v-for="entry in log" :key="entry.id" class="setting-row setting-row--single translation-log__entry" data-testid="translation-log-entry">
          <div class="setting-info">
            <span class="setting-label">{{ entryLine(entry) }}</span>
            <span class="setting-hint">
              {{ formatMoment(entry.created_at) }}<template v-if="accountLabel(entry.account_id)"> · {{ accountLabel(entry.account_id) }}</template> · {{ entryOutcome(entry) }}
            </span>
          </div>
          <template v-if="entry.sent_text">
            <AppButton variant="link" size="sm" @click="toggleEntry(entry.id)">
              {{ openEntries.has(entry.id) ? $t('settings.translation.logHideText') : $t('settings.translation.logShowText') }}
            </AppButton>
            <pre v-if="openEntries.has(entry.id)" class="translation-log__text">{{ entry.sent_text }}</pre>
          </template>
        </div>
        <div v-if="log.length === 0" class="setting-row setting-row--single">
          <span class="setting-hint">{{ $t('settings.translation.logEmpty') }}</span>
        </div>
        <div v-else class="setting-row setting-row--single">
          <AppButton variant="ghost" size="sm" @click="clearLog">{{ $t('settings.translation.logClear') }}</AppButton>
        </div>
      </div>
    </section>
  </section>
</template>

<style scoped>
.translation-settings {
  display: grid;
  gap: var(--space-lg);
}

.translation-settings__block {
  display: grid;
  gap: var(--space-sm);
}

.translation-settings__key,
.translation-settings__test {
  display: grid;
  gap: var(--space-sm);
}

.translation-settings__key .field {
  display: grid;
  gap: var(--space-xs);
  width: 100%;
}

.translation-settings__key input {
  padding: 8px 10px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.translation-settings__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}

.translation-settings__test-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
}

.translation-settings__usage {
  display: grid;
  gap: var(--space-xs);
}

.translation-log__entry {
  display: grid;
  gap: var(--space-xs);
}

.translation-log__text {
  max-height: 240px;
  margin: 0;
  overflow: auto;
  padding: var(--space-sm);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.setting-row.translation-settings__key > :last-child,
.setting-row.translation-settings__test > :last-child,
.setting-row.translation-log__entry > :last-child {
  justify-self: stretch;
}
</style>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
