<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useToast } from '@/composables/useToast';
import { ACCOUNT_COLORS } from '@/config/accountColors';
import { avatarDataUrl } from '@/utils/avatarImage';
import AccountAvatar from '@/components/common/AccountAvatar.vue';
import AppButton from '@/components/ui/AppButton.vue';
import ColorSwatchPicker from '@/components/ui/ColorSwatchPicker.vue';

const props = defineProps<{ accountId: string }>();
const { t } = useI18n();
const accounts = useAccountsStore();
const toast = useToast();
const account = computed(() => accounts.accounts.find((item) => item.id === props.accountId));
const swatches = computed(() => ACCOUNT_COLORS.map((color) => ({ value: color.value, label: t(`accountAppearance.colors.${color.id}`) })));
const fileInput = ref<HTMLInputElement | null>(null);
const busy = ref(false);

async function onFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = '';
  if (!file) return;
  busy.value = true;
  try {
    let picture: string;
    try {
      picture = await avatarDataUrl(file);
    } catch {
      toast.error(t('accountAppearance.pictureUnreadable'));
      return;
    }
    await accounts.setAvatar(props.accountId, picture);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section v-if="account" class="appearance-settings" :aria-label="t('accountAppearance.title')">
    <span class="appearance-settings__title">{{ t('accountAppearance.title') }}</span>
    <div class="appearance-settings__body">
      <AccountAvatar :name="account.displayName" :color="account.color" :avatar="account.avatar" :size="44" />
      <div class="appearance-settings__controls">
        <ColorSwatchPicker
          :model-value="account.color"
          :options="swatches"
          :label="t('accountAppearance.colorFor', { account: account.displayName })"
          :custom-label="t('accountAppearance.customColor')"
          @update:model-value="accounts.setColor(accountId, $event)"
        />
        <div class="appearance-settings__picture">
          <AppButton variant="secondary" size="sm" :disabled="busy" @click="fileInput?.click()">{{ t('accountAppearance.choosePicture') }}</AppButton>
          <AppButton v-if="account.avatar" variant="ghost" size="sm" :disabled="busy" @click="accounts.setAvatar(accountId, null)">{{ t('accountAppearance.removePicture') }}</AppButton>
          <input ref="fileInput" class="appearance-settings__file" type="file" accept="image/png,image/jpeg,image/webp,image/gif,image/heic" tabindex="-1" :aria-label="t('accountAppearance.picture')" @change="onFile" />
        </div>
        <p class="appearance-settings__help">{{ t('accountAppearance.pictureHelp') }}</p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.appearance-settings { display: grid; gap: var(--space-sm); flex-basis: 100%; padding-top: var(--space-md); border-top: 1px solid var(--border-color-muted); }
.appearance-settings__title { color: var(--text-primary); font-size: var(--font-size-sm); font-weight: var(--weight-semibold); }
.appearance-settings__body { display: flex; align-items: flex-start; gap: var(--space-md); }
.appearance-settings__controls { display: grid; gap: var(--space-sm); min-width: 0; padding-top: var(--space-2xs); }
.appearance-settings__picture { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-sm); }
.appearance-settings__file { display: none; }
.appearance-settings__help { margin: 0; color: var(--text-secondary); font-size: var(--font-size-xs); }
</style>
