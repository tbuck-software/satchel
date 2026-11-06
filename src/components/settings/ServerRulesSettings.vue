<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConfirm } from '@/composables/useConfirm';
import {
  backendDetectServerRules,
  backendDisableServerRules,
  backendEnableServerRules,
  backendGetServerRulesOverview,
  backendSetServerRulesEndpoint,
  translateBackendError,
  type BackendServerRulesAccount,
  type BackendServerRulesChoice,
  type BackendServerRulesOverview,
} from '@/services/backend';
import AppBanner from '@/components/ui/AppBanner.vue';
import AppButton from '@/components/ui/AppButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';

/**
 * Server rules per IMAP account: whether the server speaks ManageSieve,
 * whether Satchel's script runs there, and the question to ask when the
 * person already has a script of their own active.
 */

const { t, d } = useI18n();
const { ask } = useConfirm();
const overview = ref<BackendServerRulesOverview | null>(null);
const busy = ref<string | null>(null);
const errors = reactive<Record<string, string>>({});
const editing = ref<string | null>(null);
const draft = reactive({ host: '', port: '', tls: 'starttls' });

const rows = computed(() => overview.value?.accounts ?? []);

function errorText(error: unknown): string {
  return translateBackendError(error) ?? (error instanceof Error ? error.message : String(error));
}

async function load() {
  try {
    overview.value = await backendGetServerRulesOverview();
  } catch (error) {
    errors.__all = errorText(error);
  }
}

async function run(accountId: string, action: () => Promise<BackendServerRulesOverview>) {
  busy.value = accountId;
  delete errors[accountId];
  try {
    overview.value = await action();
  } catch (error) {
    errors[accountId] = errorText(error);
    await load();
  } finally {
    busy.value = null;
  }
}

type StatusTone = 'active' | 'available' | 'attention' | 'muted';

function status(state: BackendServerRulesAccount): { label: string; tone: StatusTone } {
  if (state.server_active) return { label: t('settings.serverRules.status.active'), tone: 'active' };
  if (state.consent_script) return { label: t('settings.serverRules.status.waiting'), tone: 'attention' };
  if (state.availability === 'available') return { label: t('settings.serverRules.status.available'), tone: 'available' };
  if (state.availability === 'unavailable') return { label: t('settings.serverRules.status.unavailable'), tone: 'muted' };
  return { label: t('settings.serverRules.status.unknown'), tone: 'muted' };
}

function storedError(state: BackendServerRulesAccount): string | null {
  return state.last_error ? errorText({ key: state.last_error }) : null;
}

function endpointText(state: BackendServerRulesAccount): string | null {
  if (!state.host) return null;
  const tls = state.tls === 'tls' ? t('settings.serverRules.tlsImplicit') : t('settings.serverRules.tlsStarttls');
  return t('settings.serverRules.endpoint', { host: state.host, port: state.port ?? 4190, tls });
}

/** Wrapper and Replace answer for the script shown; the backend asks again if it changed. */
function enable(state: BackendServerRulesAccount, choice: BackendServerRulesChoice) {
  const expected = choice === 'ask' ? null : state.consent_script;
  void run(state.account_id, () => backendEnableServerRules(state.account_id, choice, expected));
}

/** Autoconfig named a server on another domain: only a yes sends the password there. */
function acceptSuggested(state: BackendServerRulesAccount) {
  if (!state.suggested_host) return;
  const host = state.suggested_host;
  const port = state.suggested_port;
  const tls = state.suggested_tls === 'tls' ? 'tls' : 'starttls';
  void run(state.account_id, async () => {
    await backendSetServerRulesEndpoint({ accountId: state.account_id, host, port, tls });
    return backendDetectServerRules(state.account_id);
  });
}

async function useWrapper(state: BackendServerRulesAccount) {
  const decision = await ask({
    title: t('settings.serverRules.wrapperConfirmTitle'),
    body: t('settings.serverRules.wrapperConfirmBody', { script: state.consent_script ?? '' }),
    confirmLabel: t('settings.serverRules.runBoth'),
  });
  if (decision) enable(state, 'wrapper');
}

async function replaceScript(state: BackendServerRulesAccount) {
  const decision = await ask({
    title: t('settings.serverRules.replaceConfirmTitle', { script: state.consent_script ?? '' }),
    body: t('settings.serverRules.replaceConfirmBody', { script: state.consent_script ?? '' }),
    confirmLabel: t('settings.serverRules.replace'),
    tone: 'danger',
  });
  if (decision) enable(state, 'replace');
}

function disable(state: BackendServerRulesAccount) {
  void run(state.account_id, () => backendDisableServerRules(state.account_id));
}

function detect(state: BackendServerRulesAccount) {
  void run(state.account_id, () => backendDetectServerRules(state.account_id));
}

function startEditing(state: BackendServerRulesAccount) {
  editing.value = state.account_id;
  draft.host = state.host_override ?? '';
  draft.port = state.port_override ? String(state.port_override) : '';
  draft.tls = state.tls_override ?? 'starttls';
}

function saveEndpoint(state: BackendServerRulesAccount) {
  const host = draft.host.trim();
  const port = Number.parseInt(draft.port, 10);
  editing.value = null;
  void run(state.account_id, () => backendSetServerRulesEndpoint({
    accountId: state.account_id,
    host: host || null,
    port: host && Number.isFinite(port) ? port : null,
    tls: host ? (draft.tls === 'tls' ? 'tls' : 'starttls') : null,
  }));
}

function clearEndpoint(state: BackendServerRulesAccount) {
  editing.value = null;
  void run(state.account_id, () => backendSetServerRulesEndpoint({
    accountId: state.account_id, host: null, port: null, tls: null,
  }));
}

const tlsOptions = computed(() => [
  { value: 'starttls', label: t('settings.serverRules.tlsStarttls') },
  { value: 'tls', label: t('settings.serverRules.tlsImplicit') },
]);

onMounted(load);

defineExpose({ load });
</script>

<template>
  <section class="server-rules">
    <h3 class="settings-page-title">{{ $t('settings.serverRules.title') }}</h3>
    <p class="hint">{{ $t('settings.serverRules.hint') }}</p>
    <p v-if="errors.__all" role="alert" class="error">{{ errors.__all }}</p>

    <div v-for="state in rows" :key="state.account_id" class="setting-group" data-testid="server-rules-account">
      <div class="setting-row setting-row--stack-phone">
        <div class="setting-info">
          <span class="setting-label">{{ state.display_name }}</span>
          <span>{{ state.email }}</span>
          <span>
            <span :class="['badge', `badge--${status(state).tone}`]">{{ status(state).label }}</span>
            <template v-if="endpointText(state)"> {{ endpointText(state) }}</template>
          </span>
          <span v-if="state.server_active">
            {{ $t('settings.serverRules.pushed', { count: state.pushed_rule_count }) }}
            <template v-if="state.pushed_vacation"> · {{ $t('settings.serverRules.vacationOnServer') }}</template>
            <template v-if="state.activation_mode === 'wrapper' && state.previous_active_script">
              · {{ $t('settings.serverRules.alongside', { script: state.previous_active_script }) }}
            </template>
          </span>
          <span v-if="state.last_push_at">
            {{ $t('settings.serverRules.lastPush', { time: d(new Date(state.last_push_at), 'dateTime') }) }}
          </span>
          <span v-if="storedError(state)" class="error">{{ storedError(state) }}</span>
          <span v-if="errors[state.account_id]" role="alert" class="error">{{ errors[state.account_id] }}</span>
        </div>
        <div class="actions">
          <AppButton
            v-if="!state.enabled"
            size="sm"
            :disabled="busy === state.account_id"
            @click="detect(state)"
          >
            {{ $t('settings.serverRules.check') }}
          </AppButton>
          <AppButton
            v-if="!state.enabled && state.availability === 'available'"
            size="sm"
            variant="primary"
            :disabled="busy === state.account_id"
            @click="enable(state, 'ask')"
          >
            {{ $t('settings.serverRules.turnOn') }}
          </AppButton>
          <AppButton
            v-if="state.enabled && !state.consent_script"
            size="sm"
            :disabled="busy === state.account_id"
            @click="enable(state, 'ask')"
          >
            {{ $t('settings.serverRules.pushNow') }}
          </AppButton>
          <AppButton
            v-if="state.enabled"
            size="sm"
            :disabled="busy === state.account_id"
            @click="disable(state)"
          >
            {{ $t('settings.serverRules.turnOff') }}
          </AppButton>
        </div>
      </div>

      <!-- Autoconfig pointed somewhere else than the mail domain: ask first. -->
      <div v-if="state.suggested_host && !state.enabled" class="setting-row setting-row--single">
        <AppBanner
          tone="info"
          :title="$t('settings.serverRules.suggestedTitle', { host: state.suggested_host, port: state.suggested_port ?? 4190 })"
          :detail="$t('settings.serverRules.suggestedDetail')"
        >
          <template #actions>
            <AppButton size="sm" :disabled="busy === state.account_id" @click="acceptSuggested(state)">
              {{ $t('settings.serverRules.useSuggested') }}
            </AppButton>
          </template>
        </AppBanner>
      </div>

      <!-- The person's own script is active: nothing was written; they pick. -->
      <div v-if="state.consent_script" class="setting-row setting-row--single">
        <AppBanner
          tone="warning"
          :title="$t('settings.serverRules.consentTitle', { script: state.consent_script })"
          :detail="state.include_available
            ? $t('settings.serverRules.consentDetailInclude')
            : $t('settings.serverRules.consentDetailNoInclude')"
        >
          <template #actions>
            <AppButton v-if="state.include_available" size="sm" variant="primary" :disabled="busy === state.account_id" @click="void useWrapper(state)">
              {{ $t('settings.serverRules.runBoth') }}
            </AppButton>
            <AppButton size="sm" variant="danger" :disabled="busy === state.account_id" @click="void replaceScript(state)">
              {{ $t('settings.serverRules.replace') }}
            </AppButton>
            <AppButton size="sm" :disabled="busy === state.account_id" @click="disable(state)">
              {{ $t('settings.serverRules.keepLocal') }}
            </AppButton>
          </template>
        </AppBanner>
      </div>

      <!-- The server cannot change under a running script: turn off first. -->
      <div v-if="!state.enabled" class="setting-row setting-row--single">
        <form v-if="editing === state.account_id" class="endpoint" @submit.prevent="saveEndpoint(state)">
          <label class="field">
            <span>{{ $t('settings.serverRules.host') }}</span>
            <input v-model="draft.host" :placeholder="$t('settings.serverRules.hostPlaceholder')" autocomplete="off" spellcheck="false" />
          </label>
          <label class="field field--port">
            <span>{{ $t('settings.serverRules.port') }}</span>
            <input v-model="draft.port" inputmode="numeric" :placeholder="$t('settings.serverRules.portPlaceholder')" />
          </label>
          <SegmentedControl v-model="draft.tls" :options="tlsOptions" :aria-label="$t('settings.serverRules.security')" />
          <div class="endpoint__actions">
            <AppButton size="sm" @click="editing = null">{{ $t('common.cancel') }}</AppButton>
            <AppButton size="sm" variant="primary" type="submit">{{ $t('common.save') }}</AppButton>
          </div>
        </form>
        <div v-else class="endpoint-summary">
          <AppButton size="sm" variant="link" @click="startEditing(state)">
            {{ $t('settings.serverRules.editServer') }}
          </AppButton>
          <AppButton v-if="state.host_override" size="sm" variant="link" @click="clearEndpoint(state)">
            {{ $t('settings.serverRules.useDetected') }}
          </AppButton>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.server-rules { display: grid; gap: var(--space-md); }
.hint { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
.error { color: var(--accent-red); }
.actions, .endpoint__actions, .endpoint-summary { display: flex; flex-wrap: wrap; gap: var(--space-sm); }
.badge {
  padding: 2px 7px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  text-transform: uppercase;
}
.badge--active { color: var(--accent-green); background: color-mix(in srgb, var(--accent-green) 14%, transparent); }
.badge--available { color: var(--accent-primary); background: color-mix(in srgb, var(--accent-primary) 14%, transparent); }
.badge--attention { color: var(--accent-amber); background: color-mix(in srgb, var(--accent-amber) 14%, transparent); }
.endpoint { display: grid; grid-template-columns: minmax(0, 1fr) 7rem auto; align-items: end; gap: var(--space-sm); }
.endpoint__actions { grid-column: 1 / -1; justify-content: flex-end; }
.field { display: grid; gap: var(--space-xs); font-size: var(--font-size-sm); color: var(--text-secondary); }
.field input {
  border: 1px solid var(--border-color);
  background: var(--bg-raised);
  border-radius: var(--radius-sm);
  padding: 7px 10px;
  color: var(--text-primary);
  font-family: inherit;
}
@media (max-width: 520px) {
  .endpoint { grid-template-columns: minmax(0, 1fr); }
}
</style>

<style scoped src="./settingsRows.css"></style>
