<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useToast } from '@/composables/useToast';
import AppButton from '@/components/ui/AppButton.vue';

/**
 * The account's name in Satchel and the name on its outgoing mail. They used
 * to be one field, so naming an account "Work" signed every mail "Work".
 */
const props = defineProps<{ accountId: string }>();
const { t } = useI18n();
const accounts = useAccountsStore();
const toast = useToast();
const account = computed(() => accounts.accounts.find((item) => item.id === props.accountId));

const label = ref('');
const sender = ref('');
const busy = ref(false);

function reset() {
  label.value = account.value?.label ?? '';
  sender.value = account.value?.senderName ?? account.value?.displayName ?? '';
}
watch(() => props.accountId, reset, { immediate: true });

const dirty = computed(() => Boolean(account.value) && (
  label.value.trim() !== (account.value?.label ?? '')
  || sender.value.trim() !== (account.value?.senderName ?? account.value?.displayName ?? '')
));

async function save() {
  if (!dirty.value || busy.value) return;
  busy.value = true;
  try {
    if (await accounts.setNames(props.accountId, label.value, sender.value)) {
      reset();
      toast.success(t('settings.accounts.names.saved'));
    }
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <form v-if="account" class="account-names" @submit.prevent="save">
    <label class="setting-row account-names__row">
      <span class="setting-info">
        <span class="setting-label">{{ t('settings.accounts.names.label') }}</span>
        <span>{{ t('settings.accounts.names.labelHint') }}</span>
      </span>
      <input
        v-model="label"
        class="account-names__input"
        maxlength="120"
        autocomplete="off"
        :placeholder="sender.trim() || account.email"
        data-testid="account-label"
      />
    </label>
    <label class="setting-row account-names__row">
      <span class="setting-info">
        <span class="setting-label">{{ t('settings.accounts.names.sender') }}</span>
        <span>{{ t('settings.accounts.names.senderHint', { email: account.email }) }}</span>
      </span>
      <input
        v-model="sender"
        class="account-names__input"
        maxlength="120"
        autocomplete="name"
        :placeholder="t('settings.accounts.names.senderPlaceholder')"
        data-testid="account-sender-name"
      />
    </label>
    <div v-if="dirty" class="setting-row setting-row--single account-names__actions">
      <div class="account-names__buttons">
        <AppButton size="sm" :disabled="busy" @click="reset">{{ t('common.cancel') }}</AppButton>
        <AppButton type="submit" size="sm" variant="primary" :disabled="busy">{{ t('common.save') }}</AppButton>
      </div>
    </div>
  </form>
</template>

<style scoped src="./settingsRows.css"></style>
<style scoped>
.account-names {
  display: grid;
}

.account-names__input {
  width: min(280px, 100%);
  padding: var(--space-sm) var(--space-2-5);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.account-names__input:focus {
  border-color: var(--accent-primary);
}

.account-names__buttons {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
  width: 100%;
}

.account-names__actions > :last-child {
  justify-self: stretch;
}

@media (max-width: 767px) {
  .account-names__row {
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-sm);
  }

  .account-names__row > :last-child {
    justify-self: stretch;
    width: 100%;
  }
}
</style>
