<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import {
  backendCommandsAvailable,
  backendGetServerRulesOverview,
  backendGetVacation,
  backendOpenExternalUrl,
  backendSetVacation,
  backendSupportsNativeCommands,
  type BackendServerRulesAccount,
} from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { webmailFor } from '@/utils/webmailLinks';

const { t } = useI18n();
const accounts = useAccountsStore();
const accountId = ref<string>('');
const enabled = ref(false);
const subject = ref('');
const body = ref('');
const startsAt = ref<string>('');
const endsAt = ref<string>('');
const onlyContacts = ref(false);
const status = ref('');
const saving = ref(false);

/** Where an IMAP user sets the reply instead, when their provider is known. */
const webmail = computed(() => {
  const account = accounts.emailAccounts.find((acc) => acc.id === accountId.value);
  return account ? webmailFor(account.email) : null;
});

function openWebmail() {
  if (webmail.value) void backendOpenExternalUrl(webmail.value);
}

const selectedAccount = computed(() => accounts.emailAccounts.find((acc) => acc.id === accountId.value));

const isOauthAccount = computed(() =>
  selectedAccount.value?.provider === 'gmail' || selectedAccount.value?.provider === 'outlook');

/** Server rules (ManageSieve) of each IMAP account; the reply rides in that script. */
const sieveStates = ref<BackendServerRulesAccount[]>([]);
const sieve = computed(() => sieveStates.value.find((state) => state.account_id === accountId.value) ?? null);
const isImapAccount = computed(() => selectedAccount.value?.provider === 'imap');
/** An IMAP account whose server runs Satchel's script and knows `vacation`. */
const serverVacation = computed(() =>
  isImapAccount.value && sieve.value !== null && sieve.value.server_active && sieve.value.vacation_available);
/** Where the reply can be set from here at all. */
const canSetReply = computed(() => isOauthAccount.value || serverVacation.value);
/** A Sieve server without `date`/`relational` cannot honour a start or end. */
const showWindow = computed(() => !serverVacation.value || sieve.value?.vacation_window_available === true);

async function loadServerRules() {
  if (!backendCommandsAvailable()) return;
  try {
    sieveStates.value = (await backendGetServerRulesOverview()).accounts;
  } catch {
    // Without the overview IMAP accounts show the webmail hint, as before.
    sieveStates.value = [];
  }
}

onMounted(async () => {
  await accounts.load();
  await loadServerRules();
  accountId.value = accounts.emailAccounts.find((account) => account.id === accounts.activeAccountId)?.id
    || accounts.emailAccounts[0]?.id
    || '';
});

watch(accountId, async (id) => {
  if (!id || !backendSupportsNativeCommands()) return;
  try {
    const v = await backendGetVacation(id);
    if (v) {
      enabled.value = v.enabled === 1;
      subject.value = v.subject ?? '';
      body.value = v.body;
      startsAt.value = v.starts_at?.slice(0, 16) ?? '';
      endsAt.value = v.ends_at?.slice(0, 16) ?? '';
      onlyContacts.value = v.only_contacts === 1;
    } else {
      enabled.value = false;
      subject.value = t('settings.vacation.defaultSubject');
      body.value = '';
      startsAt.value = '';
      endsAt.value = '';
      onlyContacts.value = false;
    }
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  }
});

async function save() {
  saving.value = true;
  status.value = '';
  try {
    await backendSetVacation({
      accountId: accountId.value,
      enabled: enabled.value,
      subject: subject.value || null,
      body: body.value,
      startsAt: showWindow.value && startsAt.value ? new Date(startsAt.value).toISOString() : null,
      endsAt: showWindow.value && endsAt.value ? new Date(endsAt.value).toISOString() : null,
      // Sieve cannot know who a contact is; only Gmail honours this.
      onlyContacts: isOauthAccount.value && onlyContacts.value,
    });
    status.value = t('settings.vacation.saved');
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <section>
    <h3 class="settings-page-title">{{ $t('settings.sections.vacation.label') }}</h3>
    <p class="hint">{{ $t('settings.vacation.hint') }}</p>

    <!-- Rows like every other section: the switch on the right, the reply
         itself as one card that opens up while the switch is on. -->
    <div class="setting-group">
      <label class="setting-row">
        <span class="setting-label">{{ $t('settings.vacation.account') }}</span>
        <select v-model="accountId">
          <option v-for="a in accounts.emailAccounts" :key="a.id" :value="a.id">
            {{ a.displayName }} &lt;{{ a.email }}&gt;
          </option>
        </select>
      </label>
      <!-- An account without a server-side reply to set: say so instead of
           offering a switch that would do nothing. IMAP gets one through
           its server rules (Sieve). -->
      <div v-if="!canSetReply" class="setting-row setting-row--single">
        <p class="notice" role="note">
          <AppIcon class="notice__icon"><path d="M8 2.5 14 13H2Z" /><path d="M8 6.5v3M8 11.2h.01" /></AppIcon>
          <span v-if="isImapAccount && sieve?.server_active">{{ $t('settings.vacation.imapNoVacation') }}</span>
          <span v-else-if="isImapAccount">{{ $t('settings.vacation.imapServerRulesHint') }}</span>
          <span v-else>{{ $t('settings.vacation.imapWarning') }}</span>
          <AppButton v-if="webmail" class="notice__action" variant="secondary" size="sm" @click="openWebmail">
            {{ $t('settings.vacation.openWebmail') }}
          </AppButton>
        </p>
      </div>
      <div v-else class="setting-row">
        <span class="setting-label">{{ $t('settings.vacation.enable') }}</span>
        <ToggleSwitch v-model="enabled" :label="$t('settings.vacation.enable')" />
      </div>
    </div>

    <div v-if="enabled && canSetReply" class="editor">
      <label class="field">
        <span>{{ $t('common.subject') }}</span>
        <input v-model="subject" />
      </label>
      <label class="field">
        <span>{{ $t('settings.vacation.bodyLabel') }}</span>
        <textarea v-model="body" rows="6" />
      </label>
      <p v-if="!showWindow" class="hint">{{ $t('settings.vacation.imapNoWindow') }}</p>
      <div v-if="showWindow" class="pair">
        <label class="field">
          <span>{{ $t('common.from') }}</span>
          <input v-model="startsAt" type="datetime-local" step="60" />
        </label>
        <label class="field">
          <span>{{ $t('settings.vacation.until') }}</span>
          <input v-model="endsAt" type="datetime-local" step="60" />
        </label>
      </div>
      <div v-if="isOauthAccount" class="setting-row setting-row--inline">
        <span class="setting-label">{{ $t('settings.vacation.onlyContacts') }}</span>
        <ToggleSwitch v-model="onlyContacts" :label="$t('settings.vacation.onlyContacts')" />
      </div>
    </div>

    <div v-if="canSetReply" class="actions">
      <AppButton variant="primary" size="sm" :disabled="saving" @click="save">
        {{ saving ? $t('common.saving') : $t('common.save') }}
      </AppButton>
    </div>
    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
.notice {
  display: flex;
  align-items: flex-start;
  gap: var(--space-sm);
  margin: 0;
  padding: var(--space-sm) var(--space-md);
  border: 1px solid color-mix(in srgb, var(--accent-amber) 35%, transparent);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-amber) 8%, transparent);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  line-height: 1.45;
}
.notice__action {
  flex-shrink: 0;
  margin-inline-start: auto;
}

.notice__icon {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--accent-amber);
}

.pair { display: grid; grid-template-columns: 1fr 1fr; gap: var(--space-md); }
/* A switch inside the form card: no row padding of its own. */
.setting-row.setting-row--inline { padding: 0; }

@media (max-width: 520px) {
  .pair { grid-template-columns: minmax(0, 1fr); }
}
</style>

<style scoped src="./settingsForms.css"></style>

<style scoped src="./settingsRows.css"></style>
