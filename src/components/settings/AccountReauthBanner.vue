<script setup lang="ts">
import { ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useToast } from '@/composables/useToast';
import { authErrorLabel } from '@/utils/oauthErrors';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';

const accounts = useAccountsStore();
const toast = useToast();
const busy = ref(new Set<string>());
const { t } = useI18n();

async function reauth(accountId: string) {
  if (busy.value.has(accountId)) return;
  busy.value = new Set(busy.value).add(accountId);
  try {
    await accounts.startReauth(accountId);
    toast.success(t('settings.accounts.reauth.success'));
  } catch (err) {
    toast.error(t('settings.accounts.reauth.failed', {
      error: err instanceof Error ? err.message : String(err),
    }));
  } finally {
    const next = new Set(busy.value);
    next.delete(accountId);
    busy.value = next;
  }
}
</script>

<template>
  <div
    v-if="accounts.credentialLock || accounts.accountsWithAuthError.length > 0"
    class="reauth"
  >
    <AppBanner
      v-if="accounts.credentialLock?.keyUnavailable"
      tone="danger"
      :title="t('settings.accounts.credentialLock.keyTitle')"
      :detail="t('settings.accounts.credentialLock.keyDetail')"
    />
    <AppBanner
      v-else-if="accounts.credentialLock"
      tone="danger"
      :title="t('settings.accounts.credentialLock.unreadableTitle', { count: accounts.credentialLock.unreadableAccounts }, accounts.credentialLock.unreadableAccounts)"
      :detail="t('settings.accounts.credentialLock.unreadableDetail')"
    />
    <AppBanner
      v-for="account in accounts.accountsWithAuthError"
      :key="account.id"
      tone="danger"
      :title="t('settings.accounts.reauth.bannerTitle', { email: account.email })"
      :detail="authErrorLabel(account.lastOauthError) || t('settings.accounts.reauth.expired')"
    >
      <template #actions>
        <AppButton
          size="sm"
          variant="primary"
          :disabled="busy.has(account.id)"
          @click="reauth(account.id)"
        >
          {{ busy.has(account.id) ? t('settings.accounts.reauth.running') : t('settings.accounts.reauth.action') }}
        </AppButton>
      </template>
    </AppBanner>
  </div>
</template>

<style scoped>
.reauth {
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}
</style>
