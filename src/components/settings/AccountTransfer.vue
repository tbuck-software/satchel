<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import { backendAccountTransfer, translateBackendError } from '@/services/backend';
import { useAccountsStore } from '@/stores/accounts';

const props = defineProps<{ receive: boolean }>();
const emit = defineEmits<{ close: []; imported: [] }>();
const { t } = useI18n();
const accounts = useAccountsStore();
const session = Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('');
const status = ref<Awaited<ReturnType<typeof backendAccountTransfer>>>();
const selected = ref<string[]>([]);
const code = ref('');
const digits = computed(() => code.value.replace(/[\s-]/g, ''));
const peer = ref('');
const manual = ref('');
const manualMode = ref(false);
const busy = ref(false);
const error = ref('');
const eligible = computed(() => accounts.emailAccounts.filter(a => ['imap', 'pop3', 'gmail', 'outlook'].includes(a.provider)));
const destination = computed(() => manualMode.value ? manual.value.trim() : peer.value);
const pending = computed(() => ['waiting', 'connecting', 'searching', 'preview'].includes(status.value?.phase ?? ''));
const finished = computed(() => status.value?.phase === 'imported' || status.value?.phase === 'sent');
let timer: ReturnType<typeof setTimeout>;
let alive = true;
let generation = 0;

async function update(action: Parameters<typeof backendAccountTransfer>[0]) {
  clearTimeout(timer);
  const current = ++generation;
  busy.value = action !== 'status';
  error.value = '';
  try {
    const next = await backendAccountTransfer(action, session, action === 'send'
      ? { address: destination.value, code: digits.value, accounts: selected.value } : undefined);
    if (!alive || generation !== current) return;
    status.value = next;
    if (next.phase === 'searching' && !next.nearby.some(device => device.address === peer.value)) {
      peer.value = next.nearby.length === 1 ? next.nearby[0]?.address ?? '' : '';
    }
    if (next.phase === 'imported') await accounts.load();
  } catch (cause) {
    if (alive && current === generation) error.value = translateBackendError(cause) ?? t('backend.errors.accountTransfer');
  } finally {
    if (!alive && (action === 'receive' || action === 'discover')) void backendAccountTransfer('cancel', session).catch(() => {});
    if (alive && current === generation) {
      busy.value = false;
      if (pending.value && !error.value) timer = setTimeout(() => void update('status'), 1000);
    }
  }
}
function close() {
  if (status.value?.phase === 'imported') emit('imported');
  else emit('close');
}
async function signIn(email: string) {
  busy.value = true;
  error.value = '';
  try {
    await accounts.load();
    const account = accounts.emailAccounts.find(a => a.email === email);
    if (!account) throw new Error();
    await accounts.startReauth(account.id);
    if (status.value) status.value.sign_in = status.value.sign_in.filter(e => e !== email);
  } catch (cause) {
    error.value = translateBackendError(cause) ?? t('backend.errors.transferSignIn');
  } finally { busy.value = false; }
}
function start() {
  peer.value = '';
  code.value = '';
  void update(props.receive ? 'receive' : 'discover');
}
onMounted(start);
onBeforeUnmount(() => {
  alive = false;
  clearTimeout(timer);
  void backendAccountTransfer('cancel', session).catch(() => {});
});
</script>

<template>
  <section class="transfer" :aria-label="receive ? t('transfer.receive') : t('transfer.send')">
    <h2>{{ receive ? t('transfer.receive') : t('transfer.sendTitle') }}</h2>
    <template v-if="status?.phase === 'waiting'">
      <p>{{ t('transfer.receiverHelp') }}</p>
      <strong class="code" dir="ltr" :aria-label="t('transfer.code')">{{ status.code?.replace(/^(\d{3})(\d{3})$/, '$1 $2') }}</strong>
      <p>{{ t('transfer.expiry') }}</p>
      <details v-if="status.address"><summary>{{ t('transfer.manual') }}</summary><code dir="ltr">{{ status.address }}</code></details>
    </template>
    <template v-if="status?.phase === 'searching'">
      <p>{{ t('transfer.senderHelp') }}</p>
      <label v-if="status.nearby.length && !manualMode">{{ t('transfer.device') }}
        <select v-model="peer"><option disabled value="">{{ t('transfer.chooseDevice') }}</option>
          <option v-for="device in status.nearby" :key="device.address" :value="device.address">{{ device.name }}</option>
        </select>
      </label>
      <p v-if="!status.nearby.length && !manualMode" role="status">{{ t('transfer.searching') }}</p>
      <details @toggle="manualMode = ($event.currentTarget as HTMLDetailsElement).open">
        <summary>{{ t('transfer.manual') }}</summary>
        <label>{{ t('transfer.address') }}<input v-model="manual" autocomplete="off" dir="ltr" spellcheck="false"></label>
      </details>
      <template v-if="destination">
        <label>{{ t('transfer.code') }}<input v-model="code" inputmode="numeric" maxlength="8" autocomplete="off" dir="ltr"></label>
        <fieldset><legend>{{ t('transfer.accounts') }}</legend>
          <label v-for="a in eligible" :key="a.id" class="choice">
            <input v-model="selected" type="checkbox" :value="a.id"><span>{{ a.email }}</span>
          </label>
        </fieldset>
        <p>{{ t('transfer.passwordHelp') }}</p>
      </template>
    </template>
    <p v-if="status?.phase === 'connecting'" role="status">{{ t('transfer.connecting') }}</p>
    <template v-if="status?.phase === 'preview'">
      <p>{{ t('transfer.review') }}</p>
      <ul><li v-for="(address, i) in status.emails" :key="i">{{ address }}</li></ul>
    </template>
    <p v-if="status?.phase === 'sent'" role="status">{{ t('transfer.sent') }}</p>
    <template v-if="status?.phase === 'imported'">
      <p role="status">{{ t('transfer.done', { count: status.created }, status.created) }}</p>
      <p v-if="status.emails.length > status.created">{{ t('transfer.skipped', { count: status.emails.length - status.created }, status.emails.length - status.created) }}</p>
      <AppButton v-for="email in status.sign_in" :key="email" :disabled="busy" @click="signIn(email)">{{ t('transfer.signIn', { email }) }}</AppButton>
    </template>
    <p v-if="status?.phase === 'expired'" role="alert">{{ t('transfer.expired') }}</p>
    <p v-if="status?.phase === 'failed' || error" role="alert">{{ error || t('backend.errors.accountTransfer') }}</p>
    <div class="actions">
      <AppButton v-if="!finished" variant="secondary" :disabled="busy" @click="close">{{ t('common.cancel') }}</AppButton>
      <AppButton v-if="status?.phase === 'preview'" variant="primary" :disabled="busy" @click="update('import')">{{ t('transfer.accept') }}</AppButton>
      <AppButton v-else-if="status?.phase === 'searching' && !error" variant="primary" :disabled="busy || !destination || !/^\d{6}$/.test(digits) || !selected.length" @click="update('send')">{{ t('transfer.submit') }}</AppButton>
      <AppButton v-else-if="finished" variant="primary" :disabled="busy" @click="close">{{ t('transfer.finish') }}</AppButton>
      <AppButton v-else-if="error || status?.phase === 'expired' || status?.phase === 'failed'" variant="primary" :disabled="busy" @click="start">{{ t('transfer.retry') }}</AppButton>
    </div>
  </section>
</template>
<style scoped>
.transfer, .transfer fieldset, .transfer label { display: grid; gap: var(--space-sm); min-width: 0; }
.transfer { gap: var(--space-lg); max-width: 36rem; overflow-wrap: anywhere; }
.transfer h2, .transfer p { margin: 0; }
.transfer h2 { font-size: var(--font-size-2xl); font-weight: var(--weight-semibold); line-height: 1.2; }
.transfer label, .transfer legend { font-size: var(--font-size-md); font-weight: var(--weight-medium); }
.transfer input:not([type="checkbox"]), .transfer select {
  min-height: calc(2 * var(--space-xl));
  width: 100%;
  padding-inline: var(--space-md);
  border: var(--border-default);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
}
.transfer input[inputmode="numeric"] { font-family: var(--font-mono); font-variant-numeric: tabular-nums; letter-spacing: .12em; }
.transfer :deep(.app-button) { max-width: 100%; white-space: normal; }
.transfer p, .transfer summary { color: var(--text-secondary); }
.transfer summary { cursor: pointer; }
.transfer fieldset { border: 0; padding: 0; margin: 0; }
.transfer legend { margin-bottom: var(--space-sm); }
.transfer .choice { display: flex; align-items: center; gap: var(--space-sm); min-height: var(--space-2xl); }
.transfer details label, .transfer details code { margin-top: var(--space-sm); }
.transfer details code { display: block; }
.code { font-size: calc(2 * var(--font-size-xl)); font-family: var(--font-mono); letter-spacing: .06em; line-height: 1.2; }
.actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--space-sm); }
</style>
