<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import NasPushSetup from '@/components/settings/NasPushSetup.vue';
import ModalShell from '@/components/ui/ModalShell.vue';
import { useDismissableOnTop } from '@/composables/useDismissable';
import AppIcon from '@/components/ui/AppIcon.vue';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import AppButton from '@/components/ui/AppButton.vue';
import { backendRelayPush } from '@/services/backend';
import { backendErrorKey } from '@/utils/backendErrors';

defineProps<{ desktop: boolean }>();
const emit = defineEmits<{ availability: [available: boolean] }>();
const { t } = useI18n();
const state = ref<Awaited<ReturnType<typeof backendRelayPush>>>();
const busy = ref(false);
const error = ref('');
const pending = computed(() => ['visible', 'searching', 'requested', 'verifying'].includes(state.value?.phase ?? ''));
const code = computed(() => state.value?.code?.replace(/^(\d{3})(\d{3})$/, '$1 $2'));
const dismissed = ref(false);
const dialogOpen = computed(() => pending.value && !dismissed.value);
function closeDialog() {
  if (busy.value) return;
  if (state.value?.phase === 'verifying') dismissed.value = true;
  else void update('cancel');
}
useDismissableOnTop(() => dialogOpen.value, closeDialog);
let timer: ReturnType<typeof setTimeout>;
let mounted = true;
let sequence = 0;

async function update(action: Parameters<typeof backendRelayPush>[0], id?: string) {
  clearTimeout(timer);
  if (action === 'start') dismissed.value = false;
  const current = ++sequence;
  busy.value = action !== 'status';
  try {
    const next = await backendRelayPush(action, id);
    if (mounted && sequence === current) {
      state.value = next;
      emit('availability', next.phase !== 'unavailable');
      error.value = '';
    }
  } catch (cause) {
    if (mounted && sequence === current) error.value = backendErrorKey(cause) === 'settings.relay.unavailable'
      ? t('settings.relay.unavailable') : t('settings.relay.failed');
  } finally {
    if (mounted && current === sequence) {
      busy.value = false;
      if (pending.value) timer = setTimeout(() => void update('status'), 2000);
    }
  }
}
onMounted(() => void update('status'));
onBeforeUnmount(() => {
  mounted = false;
  clearTimeout(timer);
  if (state.value?.phase === 'visible' || state.value?.phase === 'searching')
    void backendRelayPush('cancel').catch(() => {});
});
</script>

<template>
  <section v-if="state && state.phase !== 'unavailable'" class="relay">
    <Teleport to="body">
      <ModalShell v-if="dialogOpen" compact width="440px" height="auto"
        :label="desktop ? t('settings.relay.desktopTitle') : t('settings.relay.phoneTitle')" @dismiss="closeDialog">
        <div class="pair-panel">
          <div class="pair-heading">
            <h3 v-if="state.phase === 'requested' || state.phase === 'verifying'">{{ t('settings.relay.pairWith', { name: state.name }) }}</h3>
            <h3 v-else>{{ desktop ? t('settings.relay.visibleTitle') : t('settings.relay.scanTitle') }}</h3>
            <span class="pair-icon">
              <AppIcon v-if="desktop" :grid="24" size="lg"><rect x="6.5" y="2" width="11" height="20" rx="2.5"/><path d="M11 18h2"/></AppIcon>
              <AppIcon v-else :grid="24" size="lg"><rect x="2" y="3.5" width="20" height="14" rx="2"/><path d="M12 17.5v3M8 21h8"/></AppIcon>
            </span>
          </div>
          <template v-if="state.phase === 'requested'">
            <strong class="relay-code" dir="ltr">{{ code }}</strong>
            <p>{{ desktop ? t('settings.relay.compareDesktop') : t('settings.relay.comparePhone') }}</p>
          </template>
          <p v-if="state.phase === 'visible'" role="status">{{ t('settings.relay.visible') }}</p>
          <template v-if="state.phase === 'searching'">
            <p role="status">{{ state.nearby.length ? t('settings.relay.choose') : t('settings.relay.searching') }}</p>
            <div v-if="state.nearby.length" class="pair-list">
              <ListRowButton v-for="device in state.nearby" :key="device.id" class="pair-choice" :disabled="busy" @click="update('select', device.id)">
                <span>{{ device.name }}</span><AppIcon><path d="m6 3 5 5-5 5"/></AppIcon>
              </ListRowButton>
            </div>
          </template>
          <NasPushSetup v-if="state.phase === 'searching' && !state.nearby.length" />
          <p v-if="state.phase === 'verifying'" role="status">{{ t('settings.relay.verifying') }}</p>
          <div v-if="state.phase !== 'verifying'" class="pair-actions">
            <AppButton variant="secondary" :disabled="busy" @click="update('cancel')">{{ t('common.cancel') }}</AppButton>
            <AppButton v-if="desktop && state.phase === 'requested' && state.request" variant="primary" :disabled="busy" @click="update('approve', state.request)">{{ t('settings.relay.approve') }}</AppButton>
          </div>
          <AppButton v-else variant="secondary" class="pair-close" @click="closeDialog">{{ t('common.close') }}</AppButton>
          <p v-if="error" class="pair-error" role="alert">{{ error }}</p>
        </div>
      </ModalShell>
    </Teleport>
    <div class="relay-intro">
      <h3 class="settings-page-title">{{ desktop ? t('settings.relay.desktopTitle') : t('settings.relay.phoneTitle') }}</h3>
      <p>{{ desktop ? t('settings.relay.desktopHelp') : t('settings.relay.phoneHelp') }}</p>
    </div>
    <div v-if="state.devices.length" class="pair-list">
      <div v-for="device in state.devices" :key="device.id" class="pair-device">
        <div><strong>{{ device.name }}</strong><small>{{ t('settings.relay.paired') }}</small></div>
        <AppButton variant="ghost" :disabled="busy" @click="update('remove', device.id)">{{ t('settings.pushDevices.remove') }}</AppButton>
      </div>
    </div>
    <p v-if="state.phase === 'project'" role="status">{{ t('settings.relay.project') }}</p>
    <p v-if="state.phase === 'expired'" role="status">{{ t('settings.relay.expired') }}</p>
    <p v-if="pending && dismissed" role="status">{{ t('settings.relay.verifying') }}</p>
    <AppButton v-else class="pair-start" variant="secondary" :disabled="busy || pending" @click="update('start')">{{ desktop ? t('settings.relay.show') : t('settings.relay.scan') }}</AppButton>
    <NasPushSetup v-if="!pending" />
  </section>
  <p v-if="!dialogOpen && (error || state?.phase === 'failed')" class="pair-error" role="alert">{{ error || t('settings.relay.failed') }}</p>
</template>

<style scoped>
.relay { display: grid; gap: var(--space-md); min-width: 0; }
.relay p, .pair-panel p, .pair-error {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-md);
  line-height: 1.5;
  max-inline-size: 58ch;
}
.relay-intro { display: grid; gap: var(--space-xs); }
.pair-start { justify-self: start; }
.pair-close { justify-self: end; }
.pair-panel {
  display: grid;
  gap: var(--space-md);
  padding: var(--space-xl);
  overflow-wrap: anywhere;
}
.pair-heading, .pair-choice, .pair-device {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  min-width: 0;
}
.pair-heading h3 { margin: 0; overflow-wrap: anywhere; color: var(--text-primary); font-size: var(--font-size-xl); font-weight: var(--weight-semibold); line-height: 1.25; }
.pair-icon { display: grid; place-items: center; width: var(--space-2xl); height: var(--space-2xl); flex: none; background: var(--bg-active); color: var(--text-accent); border-radius: var(--radius-sm); }
.relay-code {
  color: var(--text-primary);
  font-family: var(--font-mono);
  font-size: clamp(var(--font-size-2xl), 2em + 1vw, calc(2 * var(--font-size-2xl)));
  font-weight: var(--weight-medium);
  font-variant-numeric: tabular-nums;
  letter-spacing: .035em;
  line-height: 1.2;
  white-space: nowrap;
}
.pair-actions { display: flex; justify-content: flex-end; align-items: center; flex-wrap: wrap; gap: var(--space-sm); margin-top: var(--space-xs); }
.pair-list { border-block: 1px solid var(--border-color-muted); }
.pair-device, .pair-choice { min-height: calc(2 * var(--space-xl)); padding-block: var(--space-sm); }
.pair-device + .pair-device, .pair-choice + .pair-choice { border-top: 1px solid var(--border-color-muted); }
.pair-device strong, .pair-choice span { overflow-wrap: anywhere; font-size: var(--font-size-md); font-weight: var(--weight-semibold); }
.pair-device small { display: block; color: var(--text-secondary); font-size: var(--font-size-xs); }
.pair-choice { padding-inline: 0 var(--space-sm); }
.pair-choice :deep(svg) { color: var(--text-secondary); }
.pair-error { color: var(--accent-red-soft); margin-top: var(--space-sm); }
@media (max-width: 767px) {
  .pair-panel { padding: var(--space-xl); }
  .relay :deep(.app-button), .pair-panel :deep(.app-button) { min-height: calc(2 * var(--space-xl)); max-width: 100%; white-space: normal; }
}
</style>
