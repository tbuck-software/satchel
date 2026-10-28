<script setup lang="ts">
/**
 * Right-click on an account in the sidebar: its colour, and the way to the
 * rest of its settings (picture, custom colour, visibility). Loaded on the
 * first right-click, not with the sidebar.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import ColorSwatchPicker from '@/components/ui/ColorSwatchPicker.vue';
import { ACCOUNT_COLORS } from '@/config/accountColors';
import { useAccountsStore } from '@/stores/accounts';
import { useUiStore } from '@/stores/ui';
import type { AnchorBox } from '@/utils/popoverPlacement';

const props = defineProps<{ accountId: string; anchor: AnchorBox | null }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const router = useRouter();
const accounts = useAccountsStore();
const ui = useUiStore();
const account = computed(() => accounts.accounts.find((item) => item.id === props.accountId));
const swatches = computed(() => ACCOUNT_COLORS.map((color) => ({ value: color.value, label: t(`accountAppearance.colors.${color.id}`) })));

function recolor(color: string) {
  emit('close');
  void accounts.setColor(props.accountId, color);
}

function openSettings() {
  emit('close');
  ui.closeMobileSidebar();
  void router.push({ name: 'settings', query: { section: 'accounts' } });
}
</script>

<template>
  <ContextMenu :open="Boolean(anchor && account)" :anchor="anchor" :label="account?.displayName ?? ''" :width="248" @close="emit('close')">
    <div v-if="account" class="account-menu__colors">
      <span class="account-menu__heading">{{ t('accountAppearance.color') }}</span>
      <ColorSwatchPicker
        :model-value="account.color"
        :options="swatches"
        :label="t('accountAppearance.colorFor', { account: account.displayName })"
        menu
        @update:model-value="recolor"
      />
    </div>
    <MenuItem :label="t('accountAppearance.settings')" @select="openSettings" />
  </ContextMenu>
</template>

<style scoped>
.account-menu__colors {
  --swatch-ring: var(--bg-raised);
  display: grid;
  gap: var(--space-xs);
  padding: var(--space-xs) var(--space-sm) var(--space-sm);
  border-bottom: 1px solid var(--border-color-muted);
  margin-bottom: var(--space-2xs);
}

.account-menu__heading {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
}
</style>
