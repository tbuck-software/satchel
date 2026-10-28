<script setup lang="ts">
/**
 * Right-click on an account in the sidebar: its colour, showing a resting
 * account until midnight (or hiding it again), and the way to the rest of
 * its settings. Loaded on the first right-click, not with the sidebar.
 */
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { useToast } from '@/composables/useToast';
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
const toast = useToast();
const account = computed(() => accounts.accounts.find((item) => item.id === props.accountId));
const swatches = computed(() => ACCOUNT_COLORS.map((color) => ({ value: color.value, label: t(`accountAppearance.colors.${color.id}`) })));

function recolor(color: string) {
  emit('close');
  void accounts.setColor(props.accountId, color);
}

const visible = computed(() => accounts.isVisible(props.accountId));
// "Hide" only where it means something: a schedule, or a peek for today.
const canHide = computed(() => {
  const visibility = accounts.visibilityFor(props.accountId);
  return visible.value && (visibility?.policy.mode !== 'always' || visibility?.policy.override_date === visibility?.local_date);
});

const stateLabel = computed(() => {
  if (!visible.value) return t('accountVisibility.stateResting');
  if (accounts.isPeeked(props.accountId)) return t('accountVisibility.statePeek');
  return t('accountVisibility.visible');
});

async function setShown(shown: boolean) {
  emit('close');
  const name = account.value?.displayName ?? '';
  const saved = await accounts.showToday(props.accountId, shown);
  // A peek ends on its own at midnight; say so once, with the way back.
  if (saved && shown) {
    toast.success(t('accountVisibility.shownUntilMidnight', { account: name }), {
      action: { label: t('accountVisibility.hide'), handler: () => void accounts.showToday(props.accountId, false) },
    });
  }
}

function openSettings() {
  emit('close');
  ui.closeMobileSidebar();
  void router.push({ name: 'settings', query: { section: 'accounts' } });
}
</script>

<template>
  <ContextMenu :open="Boolean(anchor && account)" :anchor="anchor" :label="account?.displayName ?? ''" :width="248" @close="emit('close')">
    <!-- Whose menu, and in which state: the row it opened from may be under it. -->
    <div v-if="account" class="account-menu__head">
      <span class="account-menu__name">{{ account.displayName }}</span>
      <span class="account-menu__state">{{ stateLabel }}</span>
    </div>
    <MenuItem v-if="!visible" :label="t('accountVisibility.showToday')" icon="clock" :disabled="accounts.visibilityBusy" @select="setShown(true)" />
    <MenuItem v-else-if="canHide" :label="t('accountVisibility.hide')" :disabled="accounts.visibilityBusy" @select="setShown(false)">
      <template #icon>
        <AppIcon :grid="24"><path d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401" /></AppIcon>
      </template>
    </MenuItem>
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
.account-menu__head {
  display: grid;
  gap: var(--space-2xs);
  padding: var(--space-xs) var(--space-sm) var(--space-sm);
  border-bottom: 1px solid var(--border-color-muted);
  margin-bottom: var(--space-2xs);
}

.account-menu__name {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.account-menu__state {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.account-menu__colors {
  --swatch-ring: var(--bg-raised);
  display: grid;
  gap: var(--space-xs);
  padding: var(--space-sm);
  border-block: 1px solid var(--border-color-muted);
  margin-block: var(--space-2xs);
}

.account-menu__heading {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-semibold);
}
</style>
