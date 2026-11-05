<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import {
  backendGetVacation,
  backendSetVacation,
  backendSupportsNativeCommands,
} from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

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

const isOauthAccount = computed(() => {
  const a = accounts.emailAccounts.find((acc) => acc.id === accountId.value);
  return a?.provider === 'gmail' || a?.provider === 'outlook';
});

onMounted(async () => {
  await accounts.load();
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
      startsAt: startsAt.value ? new Date(startsAt.value).toISOString() : null,
      endsAt: endsAt.value ? new Date(endsAt.value).toISOString() : null,
      onlyContacts: onlyContacts.value,
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
      <!-- IMAP has no server-side auto-reply to set: say so instead of
           offering a switch that would do nothing. -->
      <div v-if="!isOauthAccount" class="setting-row setting-row--single">
        <p class="notice" role="note">{{ $t('settings.vacation.imapWarning') }}</p>
      </div>
      <div v-else class="setting-row">
        <span class="setting-label">{{ $t('settings.vacation.enable') }}</span>
        <ToggleSwitch v-model="enabled" :label="$t('settings.vacation.enable')" />
      </div>
    </div>

    <div v-if="enabled && isOauthAccount" class="editor">
      <label class="field">
        <span>{{ $t('common.subject') }}</span>
        <input v-model="subject" />
      </label>
      <label class="field">
        <span>{{ $t('settings.vacation.bodyLabel') }}</span>
        <textarea v-model="body" rows="6" />
      </label>
      <div class="pair">
        <label class="field">
          <span>{{ $t('common.from') }}</span>
          <input v-model="startsAt" type="datetime-local" step="60" />
        </label>
        <label class="field">
          <span>{{ $t('settings.vacation.until') }}</span>
          <input v-model="endsAt" type="datetime-local" step="60" />
        </label>
      </div>
      <div class="setting-row setting-row--inline">
        <span class="setting-label">{{ $t('settings.vacation.onlyContacts') }}</span>
        <ToggleSwitch v-model="onlyContacts" :label="$t('settings.vacation.onlyContacts')" />
      </div>
    </div>

    <div v-if="isOauthAccount" class="actions">
      <AppButton variant="primary" size="sm" :disabled="saving" @click="save">
        {{ saving ? $t('common.saving') : $t('common.save') }}
      </AppButton>
    </div>
    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
.notice {
  margin: 0;
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-amber) 12%, transparent);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  line-height: 1.45;
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
