<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import { backendOauthStatus } from '@/services/backend';
import { useAccountsStore } from '@/stores/accounts';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

const props = defineProps<{ purpose: 'contacts' | 'calendar' }>();
const emit = defineEmits<{ connected: [accountId: string] }>();
const { t } = useI18n();
const accounts = useAccountsStore();
const googleAccounts = computed(() => accounts.accounts.filter((account) => account.provider === 'gmail'));
const connected = ref<Record<string, boolean>>({});
const loading = ref(true);
const busy = ref('');
const error = ref('');
const title = computed(() => props.purpose === 'calendar'
  ? t('settings.google.calendarTitle') : t('settings.google.contactsTitle'));
const hint = computed(() => props.purpose === 'calendar'
  ? t('settings.google.calendarHint') : t('settings.google.contactsHint'));

let revision = 0;
async function refresh() {
  const request = ++revision;
  loading.value = true;
  try {
    const entries = await Promise.all(googleAccounts.value.map(async (account) => {
      const status = await backendOauthStatus(account.id);
      const scope = props.purpose === 'calendar'
        ? 'https://www.googleapis.com/auth/calendar.readonly'
        : 'https://www.googleapis.com/auth/contacts.readonly';
      return [account.id, status.granted_scopes?.includes(scope) ?? false] as const;
    }));
    if (request === revision) connected.value = Object.fromEntries(entries);
  } catch (cause) {
    if (request === revision) error.value = backendErrorLabel(cause);
  } finally {
    if (request === revision) loading.value = false;
  }
}

watch(() => [props.purpose, ...googleAccounts.value.map((account) => account.id)], () => void refresh(), { immediate: true });

async function connect(accountId: string) {
  if (busy.value) return;
  busy.value = accountId;
  error.value = '';
  try {
    await accounts.startReauth(accountId, props.purpose);
    await refresh();
    if (connected.value[accountId]) emit('connected', accountId);
  } catch (cause) {
    error.value = backendErrorLabel(cause);
  } finally {
    busy.value = '';
  }
}
</script>

<template>
  <div v-if="googleAccounts.length" class="google-connections">
    <SectionHeading :label="title" />
    <p class="hint">{{ hint }}</p>
    <ListRowButton
      v-for="account in googleAccounts"
      :key="account.id"
      :disabled="loading || Boolean(busy) || connected[account.id]"
      @click="connect(account.id)"
    >
      <span class="account-email">{{ account.email }}</span>
      <span class="account-state">{{ loading
        ? t('settings.google.checking') : busy === account.id
          ? t('settings.google.authorizing') : connected[account.id]
            ? t('settings.google.connected') : t('settings.google.connect') }}</span>
    </ListRowButton>
    <p v-if="error" role="alert" class="error">{{ error }}</p>
  </div>
</template>

<style scoped>
.google-connections { display: grid; gap: var(--space-sm); }
.hint, .error { margin: 0; font-size: var(--font-size-sm); color: var(--text-secondary); }
.error { color: var(--accent-red-soft); }
.google-connections :deep(.list-row-button) {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-md);
  border: var(--border-default);
}
.account-email { min-width: 0; overflow-wrap: anywhere; }
.account-state { flex-shrink: 0; color: var(--text-secondary); font-size: var(--font-size-sm); }
</style>
