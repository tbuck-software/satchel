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
    <h3>{{ $t('settings.vacation.title') }}</h3>
    <p class="hint">{{ $t('settings.vacation.hint') }}</p>

    <label class="field">
      <span>{{ $t('settings.vacation.account') }}</span>
      <select v-model="accountId">
        <option v-for="a in accounts.emailAccounts" :key="a.id" :value="a.id">
          {{ a.displayName }} &lt;{{ a.email }}&gt;
        </option>
      </select>
    </label>

    <p v-if="!isOauthAccount" class="warn">
      ⚠ {{ $t('settings.vacation.imapWarning') }}
    </p>

    <ToggleSwitch v-model="enabled">{{ $t('settings.vacation.enable') }}</ToggleSwitch>

    <label class="field">
      <span>{{ $t('common.subject') }}</span>
      <input v-model="subject" :disabled="!enabled" />
    </label>

    <label class="field">
      <span>{{ $t('settings.vacation.bodyLabel') }}</span>
      <textarea v-model="body" :disabled="!enabled" rows="6" />
    </label>

    <div class="row">
      <label class="field">
        <span>{{ $t('common.from') }}</span>
        <input v-model="startsAt" type="datetime-local" :disabled="!enabled" />
      </label>
      <label class="field">
        <span>{{ $t('settings.vacation.until') }}</span>
        <input v-model="endsAt" type="datetime-local" :disabled="!enabled" />
      </label>
    </div>

    <ToggleSwitch v-model="onlyContacts" :disabled="!enabled">
      {{ $t('settings.vacation.onlyContacts') }}
    </ToggleSwitch>

    <div class="actions">
      <AppButton variant="primary" :disabled="saving" @click="save">
        {{ saving ? $t('common.saving') : $t('common.save') }}
      </AppButton>
    </div>
    <p v-if="status" class="status">{{ status }}</p>
  </section>
</template>

<style scoped>
section { display: grid; gap: 14px; }
h3 { margin: 0; color: var(--text-primary); }
.hint { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
.warn { margin: 0; color: var(--accent-amber-soft); font-size: var(--font-size-sm); }
.field { display: grid; gap: 4px; font-size: var(--font-size-sm); color: var(--text-secondary); }
.field input, .field select, .field textarea { border: 1px solid var(--border-color); background: var(--bg-raised); border-radius: var(--radius-sm); padding: 8px 10px; color: var(--text-primary); font-family: inherit; }
.row { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.actions { display: flex; justify-content: flex-end; }
.status { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-xs); }
</style>
