<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConfirm } from '@/composables/useConfirm';
import {
  backendErrorKey,
  backendDeletePgpKey,
  backendGeneratePgpKey,
  backendImportPgpKeys,
  backendListPluginAuditLog,
  backendListPluginMetrics,
  backendListPgpKeys,
  backendPgpEngineStatus,
  backendListPlugins,
  backendDeleteWebhook,
  backendListWebhooks,
  backendSetPluginEnabled,
  backendSetPluginPermission,
  backendUpsertWebhook,
  backendUnlockPgpKeys,
  backendValidatePgpBundlePassphrase,
  translateBackendError,
  type BackendGeneratePgpKeyResult,
  type BackendPluginAuditRecord,
  type BackendPluginMetrics,
  type BackendPgpEngineStatus,
  type BackendPgpKey,
  type BackendPlugin,
  type BackendWebhook,
} from '@/services/backend';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { useToast } from '@/composables/useToast';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import TimeAgo from '@/components/common/TimeAgo.vue';
import AppBanner from '@/components/ui/AppBanner.vue';
import { useMessagesStore } from '@/stores/messages';

const { t } = useI18n();
const { ask } = useConfirm();
const toast = useToast();
const messages = useMessagesStore();

interface PluginView {
  id: string;
  name: string;
  version: string;
  enabled: boolean;
  apiVersion: string;
  capabilities: string[];
  events: string[];
  uiSlots: string[];
  trust: string;
  signatureVerified: boolean;
  manifestErrors: string[];
  disabledBy: 'user' | 'timeouts' | null;
  grantedCapabilities: string[];
  enforcedCapabilities: string[];
}

const props = withDefaults(defineProps<{
  /**
   * `pgp` renders only the OpenPGP key management, so it can live under
   * Settings → Identität next to S/MIME where users actually look for it.
   * `plugins` renders everything else. Default shows both.
   */
  only?: 'pgp' | 'plugins' | 'all';
}>(), { only: 'all' });

const showPgp = computed(() => props.only === 'pgp' || props.only === 'all');
const showPlugins = computed(() => props.only === 'plugins' || props.only === 'all');

const pgpPluginId = 'satchel-plugin-pgp';

const loadingPlugins = ref(false);
const loadingKeys = ref(false);
const importingBundle = ref(false);
const generatingKey = ref(false);
const deletingKeyId = ref<string | null>(null);
const unlockingKeyId = ref<string | null>(null);
const dragActive = ref(false);
const dragDepth = ref(0);

const plugins = ref<PluginView[]>([]);
const keys = ref<BackendPgpKey[]>([]);
const engineStatus = ref<BackendPgpEngineStatus | null>(null);
const pluginMetrics = ref<BackendPluginMetrics[]>([]);
const pluginAudit = ref<BackendPluginAuditRecord[]>([]);
const webhooks = ref<BackendWebhook[]>([]);
const statusText = ref('');
const webhookUrl = ref('');
const webhookEvents = ref('message.received, thread.updated');
const fileInput = ref<HTMLInputElement | null>(null);

const importLabelPrefix = ref('');
const importPassphrase = ref('');
const importArmored = ref('');

const generateName = ref('');
const generateEmail = ref('');
const generateLabel = ref('');
const generatePassphrase = ref('');
const generatedPublicKey = ref('');
const generatedFingerprint = ref('');
const pendingImportFileName = ref('');
const importValidationText = ref('');
const importValidationTone = ref<'ok' | 'warn' | 'error' | ''>('');
const validatingImportPassphrase = ref(false);
const showImportArmored = ref(false);
const unlockEditorKeyId = ref<string | null>(null);
const unlockPassphrase = ref('');
let importValidationTimer: ReturnType<typeof setTimeout> | null = null;

function prettifyPluginName(id: string): string {
  return id
    .replace(/^satchel-plugin-/, '')
    .split(/[-_]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function pluginDescription(id: string): string {
  if (id === pgpPluginId) {
    return t('settings.plugins.pgpPluginDescription');
  }
  return t('settings.plugins.installedPluginDescription');
}

function toViewModel(plugin: BackendPlugin): PluginView {
  return {
    id: plugin.id,
    name: prettifyPluginName(plugin.id),
    version: plugin.version,
    enabled: plugin.enabled,
    apiVersion: plugin.api_version,
    capabilities: plugin.capabilities,
    events: plugin.event_subscriptions,
    uiSlots: plugin.ui_slots,
    trust: plugin.trust,
    signatureVerified: plugin.signature_verified,
    manifestErrors: plugin.manifest_errors,
    disabledBy: plugin.disabled_by ?? null,
    grantedCapabilities: plugin.granted_capabilities,
    enforcedCapabilities: plugin.enforced_capabilities ?? [],
  };
}

function formatFingerprint(raw: string): string {
  const compact = raw.replace(/\s+/g, '').toUpperCase();
  return compact.replace(/(.{4})/g, '$1 ').trim();
}

const pgpPlugin = computed(() => plugins.value.find((plugin) => plugin.id === pgpPluginId) ?? null);
const pgpMessageAccessBlocked = computed(() => pgpPlugin.value?.capabilities.includes('read_threads')
  && !hasGrant(pgpPlugin.value, 'read_threads'));
const additionalPlugins = computed(() =>
  plugins.value.filter((plugin) => plugin.id !== pgpPluginId),
);
const unlockingInProgress = computed(() => unlockingKeyId.value !== null);
const metricsByPlugin = computed(() => new Map(pluginMetrics.value.map((row) => [row.plugin_id, row])));

function hasGrant(plugin: PluginView, capability: string): boolean {
  return plugin.grantedCapabilities.includes(capability);
}

// The runtime gates commands on only a subset of the declared capabilities;
// the rest persist a switch that changes nothing. Name them instead of showing
// eight toggles that look equally effective.
function unenforcedCapabilities(plugin: PluginView): string[] {
  return plugin.capabilities.filter(
    (capability) => !plugin.enforcedCapabilities.includes(capability),
  );
}

async function toggleCapability(plugin: PluginView, capability: string) {
  const granted = !hasGrant(plugin, capability);
  try {
    await backendSetPluginPermission({
      pluginId: plugin.id,
      capability,
      granted,
    });
    await loadPlugins();
    if (plugin.id === pgpPluginId) await messages.reloadPgpMessages();
  } catch (error) {
    statusText.value = t('settings.plugins.permissionUpdateFailed', {
      error: (error as Error).message ?? String(error),
    });
  }
}

async function setPluginEnabled(plugin: PluginView, enabled: boolean) {
  try {
    await backendSetPluginEnabled({ pluginId: plugin.id, enabled });
    await loadPlugins();
    if (plugin.id === pgpPluginId) await messages.reloadPgpMessages();
  } catch (error) {
    statusText.value = t('settings.plugins.enableFailed', {
      error: (error as Error).message ?? String(error),
    });
  }
}

async function loadPlugins() {
  loadingPlugins.value = true;
  try {
    const rows = await backendListPlugins();
    plugins.value = rows.map(toViewModel);
    pluginMetrics.value = showPlugins.value ? await backendListPluginMetrics() : [];
    pluginAudit.value = showPlugins.value ? (await backendListPluginAuditLog()).slice(-20).reverse() : [];
    webhooks.value = showPlugins.value ? await backendListWebhooks() : [];
  } catch {
    plugins.value = [];
    pluginMetrics.value = [];
    pluginAudit.value = [];
    webhooks.value = [];
  } finally {
    loadingPlugins.value = false;
  }
}

/**
 * The subscribed events, as a list.
 *
 * `events_json` is storage, not copy. A row whose JSON is unreadable still has
 * a URL worth showing and a delete button worth reaching — parsing must not be
 * able to take the section down with it.
 */
function webhookEventList(webhook: BackendWebhook): string[] {
  try {
    const parsed: unknown = JSON.parse(webhook.events_json);
    return Array.isArray(parsed) ? parsed.map((event) => String(event)) : [];
  } catch {
    return [];
  }
}

async function setWebhookEnabled(webhook: BackendWebhook, enabled: boolean) {
  const previous = webhook.enabled;
  // Optimistic: the switch has to move under the finger, not after a round trip.
  webhook.enabled = enabled ? 1 : 0;
  try {
    await backendUpsertWebhook({
      id: webhook.id,
      url: webhook.url,
      events: webhookEventList(webhook),
      enabled,
    });
  } catch (error) {
    webhook.enabled = previous;
    toast.error(t('settings.plugins.webhookToggleFailed', { error: backendErrorLabel(error) }));
  }
}

async function removeWebhook(webhook: BackendWebhook) {
  // A webhook forwards message metadata to a foreign host. Name the host in
  // the question, so nobody deletes the wrong one from a list of two.
  const confirmed = await ask({
    title: t('settings.plugins.webhookDeleteTitle'),
    body: t('settings.plugins.webhookDeleteConfirm', { url: webhook.url }),
    confirmLabel: t('settings.plugins.deleteWebhook'),
    tone: 'danger',
  });
  if (!confirmed) return;
  try {
    await backendDeleteWebhook(webhook.id);
    webhooks.value = webhooks.value.filter((entry) => entry.id !== webhook.id);
    statusText.value = t('settings.plugins.webhookDeleted', { url: webhook.url });
  } catch (error) {
    toast.error(t('settings.plugins.webhookDeleteFailed', { error: backendErrorLabel(error) }));
  }
}

async function addWebhook() {
  try {
    await backendUpsertWebhook({
      url: webhookUrl.value.trim(),
      events: webhookEvents.value.split(',').map((event) => event.trim()).filter(Boolean),
      enabled: true,
    });
    webhookUrl.value = '';
    await loadPlugins();
  } catch (error) {
    statusText.value = t('settings.plugins.webhookSaveFailed', {
      error: (error as Error).message ?? String(error),
    });
  }
}

async function loadKeys() {
  loadingKeys.value = true;
  try {
    keys.value = await backendListPgpKeys();
  } catch (error) {
    keys.value = [];
    statusText.value = t('settings.pgp.keysLoadFailed', {
      error: (error as Error).message ?? String(error),
    });
  } finally {
    loadingKeys.value = false;
  }
}

function resetImportFields() {
  importArmored.value = '';
  importLabelPrefix.value = '';
  pendingImportFileName.value = '';
  importPassphrase.value = '';
  importValidationText.value = '';
  importValidationTone.value = '';
  showImportArmored.value = false;
}

function setImportValidation(tone: 'ok' | 'warn' | 'error' | '', text: string) {
  importValidationTone.value = tone;
  importValidationText.value = text;
}

function inferLabelPrefix(fileName: string): string | undefined {
  const trimmed = fileName.trim();
  if (!trimmed) return undefined;
  const base = trimmed.replace(/\.[^.]+$/, '').trim();
  return base || undefined;
}

async function verifyImportPassphrase(auto = false): Promise<boolean> {
  const armoredBundle = importArmored.value.trim();
  if (!armoredBundle) {
    if (!auto) {
      setImportValidation('error', t('settings.pgp.addMaterialFirst'));
    }
    return false;
  }

  const passphrase = importPassphrase.value.trim();
  if (!passphrase) {
    if (!auto) {
      setImportValidation('', t('settings.pgp.noPassphraseSet'));
    }
    return true;
  }

  validatingImportPassphrase.value = true;
  try {
    const result = await backendValidatePgpBundlePassphrase({
      armoredBundle,
      passphrase,
    });

    if (result.private_keys === 0) {
      setImportValidation('warn', t('settings.pgp.noPrivateKeysInBundle'));
      return true;
    }

    if (result.matching_keys === 0) {
      setImportValidation(
        'error',
        t('settings.pgp.passphraseMismatch', { total: result.private_keys }),
      );
      return false;
    }

    if (result.failing_keys > 0) {
      setImportValidation(
        'warn',
        t('settings.pgp.passphrasePartial', {
          matching: result.matching_keys,
          total: result.private_keys,
        }),
      );
      return true;
    }

    setImportValidation('ok', t('settings.pgp.passphraseVerified', result.matching_keys));
    return true;
  } catch (error) {
    const message = (error as Error).message ?? String(error);
    setImportValidation('error', t('settings.pgp.passphraseCheckFailed', { error: message }));
    return false;
  } finally {
    validatingImportPassphrase.value = false;
  }
}

async function importBundle(rawBundle: string, labelPrefix?: string) {
  const armoredBundle = rawBundle.trim();
  if (!armoredBundle) {
    statusText.value = t('settings.pgp.noKeyMaterial');
    return;
  }

  importingBundle.value = true;
  statusText.value = '';
  try {
    const result = await backendImportPgpKeys({
      armoredBundle,
      labelPrefix: labelPrefix || importLabelPrefix.value || undefined,
      passphrase: importPassphrase.value || undefined,
    });
    await loadKeys();

    const summary = t('settings.pgp.importSummary', {
      imported: result.imported.length,
      detected: result.detected,
      skipped: result.skipped,
      failed: result.failed,
    });
    const failureDetails = result.failures
      .slice(0, 3)
      .map((failure) =>
        t('settings.pgp.importFailureDetail', {
          index: failure.index,
          reason: translateBackendError(failure.key) ?? t('backend.errors.unexpected'),
        }),
      )
      .join(' ');
    statusText.value = failureDetails ? `${summary} ${failureDetails}` : summary;

    if (result.imported.length > 0) {
      resetImportFields();
      await messages.reloadPgpMessages();
    }
  } catch (error) {
    statusText.value = t('settings.pgp.importFailed', {
      error: (error as Error).message ?? String(error),
    });
  } finally {
    importingBundle.value = false;
  }
}

async function importFromTextarea() {
  const canProceed = await verifyImportPassphrase(false);
  if (!canProceed) {
    statusText.value = t('settings.pgp.importStopped');
    return;
  }
  await importBundle(importArmored.value);
}

function openFilePicker() {
  fileInput.value?.click();
}

async function stageImportFromFile(file: File) {
  const text = await file.text();
  importArmored.value = text;
  showImportArmored.value = false;
  pendingImportFileName.value = file.name;
  if (!importLabelPrefix.value.trim()) {
    importLabelPrefix.value = inferLabelPrefix(file.name) ?? '';
  }
  statusText.value = t('settings.pgp.fileLoaded', { name: file.name });
  if (importPassphrase.value.trim()) {
    void verifyImportPassphrase(true);
  }
}

async function onFilePicked(event: Event) {
  const target = event.target as HTMLInputElement;
  const file = target.files?.[0];
  if (!file) return;
  await stageImportFromFile(file);
  target.value = '';
}

function onDragOver(event: DragEvent) {
  event.preventDefault();
  dragActive.value = true;
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'copy';
  }
}

function onDragEnter(event: DragEvent) {
  event.preventDefault();
  dragDepth.value += 1;
  dragActive.value = true;
}

function onDragLeave(event: DragEvent) {
  event.preventDefault();
  dragDepth.value = Math.max(0, dragDepth.value - 1);
  if (dragDepth.value === 0) {
    dragActive.value = false;
  }
}

async function onDrop(event: DragEvent) {
  event.preventDefault();
  dragActive.value = false;
  dragDepth.value = 0;

  const itemFile =
    event.dataTransfer?.items &&
    Array.from(event.dataTransfer.items)
      .find((item) => item.kind === 'file')
      ?.getAsFile();
  const file = itemFile ?? event.dataTransfer?.files?.[0];
  if (file) {
    await stageImportFromFile(file);
    return;
  }

  const rawText = event.dataTransfer?.getData('text/plain') ?? '';
  if (rawText.trim()) {
    await importBundle(rawText);
  }
}

function clearImportMaterial() {
  importArmored.value = '';
  pendingImportFileName.value = '';
  setImportValidation('', '');
}

async function generateKey() {
  if (!generateName.value.trim() || !generateEmail.value.trim()) {
    statusText.value = t('settings.pgp.nameAndEmailRequired');
    return;
  }

  generatingKey.value = true;
  statusText.value = '';
  try {
    const generated: BackendGeneratePgpKeyResult = await backendGeneratePgpKey({
      name: generateName.value.trim(),
      email: generateEmail.value.trim(),
      label: generateLabel.value || undefined,
      passphrase: generatePassphrase.value || undefined,
    });
    await loadKeys();
    generatedPublicKey.value = generated.public_key.trim();
    generatedFingerprint.value = generated.fingerprint;
    statusText.value = t('settings.pgp.keyGenerated', { fingerprint: generated.fingerprint });
  } catch (error) {
    statusText.value = t('settings.pgp.keyGenerationFailed', {
      error: (error as Error).message ?? String(error),
    });
  } finally {
    generatingKey.value = false;
  }
}

async function copyPublicKey() {
  if (!generatedPublicKey.value) return;
  try {
    await navigator.clipboard.writeText(generatedPublicKey.value);
    statusText.value = t('settings.pgp.publicKeyCopied');
  } catch {
    statusText.value = t('settings.pgp.publicKeyCopyFailed');
  }
}

async function removeKey(key: BackendPgpKey) {
  // Deleting a private key is final: every mail ever encrypted for it stays
  // shut. The fingerprint is in the question so it is clear which one goes.
  const fingerprint = formatFingerprint(key.fingerprint);
  const question = key.key_type === 'private'
    ? t('settings.pgp.keyDeleteConfirmPrivate', { fingerprint })
    : t('settings.pgp.keyDeleteConfirmPublic', { fingerprint });
  const confirmed = await ask({
    title: t('settings.pgp.keyDeleteTitle'),
    body: question,
    confirmLabel: t('common.delete'),
    tone: 'danger',
  });
  if (!confirmed) return;

  deletingKeyId.value = key.id;
  statusText.value = '';
  try {
    await backendDeletePgpKey(key.id);
    await loadKeys();
    statusText.value = t('settings.pgp.keyDeleted', { fingerprint: key.fingerprint });
  } catch (error) {
    const message = t('settings.pgp.keyDeleteFailed', { error: backendErrorLabel(error) });
    statusText.value = message;
    // The grey status line reads like "Draft saved"; a failure needs its own voice.
    toast.error(message);
  } finally {
    deletingKeyId.value = null;
  }
}

async function unlockSingleKey(key: BackendPgpKey, passphrase?: string): Promise<boolean> {
  if (unlockingKeyId.value) {
    return false;
  }

  unlockingKeyId.value = key.id;
  statusText.value = t('settings.pgp.unlockInProgress');
  try {
    const result = await backendUnlockPgpKeys({
      keyId: key.id,
      passphrase: passphrase?.trim() || undefined,
    });
    if (result.key_ids.length > 0) {
      const unlockedSet = new Set(result.key_ids);
      keys.value = keys.value.map((candidate) =>
        unlockedSet.has(candidate.id)
          ? { ...candidate, is_unlocked: true }
          : candidate,
      );
    }
    void loadKeys();
    await messages.reloadPgpMessages();
    statusText.value = t('settings.pgp.unlockSuccess', result.unlocked);
    unlockEditorKeyId.value = null;
    unlockPassphrase.value = '';
    return true;
  } catch (error) {
    const message = (error as Error).message ?? String(error);
    statusText.value = t('settings.pgp.unlockFailed', { error: message });
    if (
      !passphrase
      && key.has_passphrase
      && backendErrorKey(error) === 'backend.errors.pgpStoredPassphraseRejected'
    ) {
      unlockEditorKeyId.value = key.id;
      statusText.value = t('settings.pgp.storedPassphraseFailed');
    }
    return false;
  } finally {
    unlockingKeyId.value = null;
  }
}

async function onUnlockClick(key: BackendPgpKey) {
  if (unlockEditorKeyId.value === key.id) {
    await submitUnlockPassphrase(key);
    return;
  }

  if (key.key_type !== 'private') {
    statusText.value = t('settings.pgp.unlockPrivateOnly');
    return;
  }

  if (!key.has_passphrase) {
    await unlockSingleKey(key);
    return;
  }

  const unlocked = await unlockSingleKey(key);
  if (!unlocked && key.has_passphrase && !unlockEditorKeyId.value) {
    unlockEditorKeyId.value = key.id;
  }
}

async function submitUnlockPassphrase(key: BackendPgpKey) {
  const candidate = unlockPassphrase.value.trim();
  if (!candidate) {
    statusText.value = t('settings.pgp.enterPassphrase');
    return;
  }
  await unlockSingleKey(key, candidate);
}

function cancelUnlockEditor() {
  unlockEditorKeyId.value = null;
  unlockPassphrase.value = '';
}

watch([importArmored, importPassphrase], () => {
  if (importValidationTimer) {
    clearTimeout(importValidationTimer);
    importValidationTimer = null;
  }

  setImportValidation('', '');
  if (!importArmored.value.trim() || !importPassphrase.value.trim()) {
    return;
  }

  importValidationTimer = setTimeout(() => {
    void verifyImportPassphrase(true);
  }, 450);
});

onBeforeUnmount(() => {
  if (importValidationTimer) {
    clearTimeout(importValidationTimer);
    importValidationTimer = null;
  }
});

onMounted(() => {
  void Promise.all([
    loadPlugins(),
    loadKeys(),
    backendPgpEngineStatus()
      .then((status) => {
        engineStatus.value = status;
      })
      .catch(() => {
        engineStatus.value = null;
      }),
  ]);
});
</script>

<template>
  <div class="section">
    <header class="section-header">
      <h3>{{ props.only === 'pgp' ? $t('settings.pgp.title') : $t('settings.plugins.title') }}</h3>
      <p class="section-desc">
        {{ props.only === 'pgp'
          ? $t('settings.pgp.description')
          : $t('settings.plugins.description') }}
      </p>
    </header>

    <p v-if="loadingPlugins" class="empty">{{ $t('settings.plugins.loading') }}</p>
    <p v-else-if="plugins.length === 0" class="empty">{{ $t('settings.plugins.noneFound') }}</p>

    <p v-else-if="props.only === 'pgp' && !pgpPlugin" class="empty">{{ $t('settings.plugins.pgpPluginMissing') }}</p>

    <p v-if="showPgp && pgpPlugin && engineStatus && !engineStatus.available" class="pgp-engine-warning" role="alert">
      <strong>{{ $t('settings.pgp.engineUnavailableTitle') }}</strong>
      {{ $t('settings.pgp.engineUnavailableBody') }}
    </p>

    <p v-if="showPgp && showPlugins && pgpPlugin" class="pgp-scope-note">
      {{ $t('settings.pgp.scopeNote') }}
    </p>

    <section v-if="showPgp && pgpPlugin" class="pgp-integration">
      <header class="pgp-header">
        <div class="plugin-info">
          <span class="plugin-name">
            {{ $t('settings.pgp.keysTitle') }}
            <small class="plugin-version">v{{ pgpPlugin.version }}</small>
          </span>
          <span class="plugin-desc">
            {{ $t('settings.pgp.keysDescription') }}
          </span>
        </div>
        <ToggleSwitch
          class="plugin-enabled"
          :label="$t('settings.plugins.enabledSwitch')"
          :model-value="pgpPlugin.enabled"
          :disabled="pgpPlugin.manifestErrors.length > 0"
          @update:model-value="setPluginEnabled(pgpPlugin, $event)"
        />
      </header>
      <p v-if="pgpPlugin.disabledBy === 'timeouts'" class="plugin-error">{{ $t('settings.plugins.disabledByTimeouts') }}</p>
      <AppBanner
        v-if="pgpMessageAccessBlocked"
        :title="$t('settings.pgp.messageAccessBlocked')"
      >
        <p class="pgp-access-description">{{ $t('settings.pgp.messageAccessBlockedDescription') }}</p>
        <AppButton size="sm" @click="toggleCapability(pgpPlugin, 'read_threads')">
          {{ $t('settings.pgp.allowMessageAccess') }}
        </AppButton>
      </AppBanner>
      <div class="permission-grid">
        <ToggleSwitch
          v-for="capability in pgpPlugin.capabilities"
          :key="capability"
          class="permission-toggle"
          :model-value="hasGrant(pgpPlugin, capability)"
          @update:model-value="toggleCapability(pgpPlugin, capability)"
        >
          {{ capability === 'read_threads' ? $t('settings.pgp.messageAccess') : capability }}
        </ToggleSwitch>
      </div>
      <span v-if="unenforcedCapabilities(pgpPlugin).length" class="plugin-meta">
        {{ $t('settings.plugins.capabilitiesNotEnforced', { list: unenforcedCapabilities(pgpPlugin).join(', ') }) }}
      </span>

      <section class="keys">
        <header class="keys-header">
          <h4>{{ $t('settings.pgp.storedKeysTitle') }}</h4>
          <AppButton :disabled="loadingKeys || importingBundle" @click="loadKeys">
            {{ loadingKeys ? $t('common.loading') : $t('common.refresh') }}
          </AppButton>
        </header>

        <p v-if="loadingKeys" class="empty">{{ $t('settings.pgp.keysLoading') }}</p>
        <p v-else-if="keys.length === 0" class="empty">{{ $t('settings.pgp.keysEmpty') }}</p>

        <div v-else class="key-list">
          <article
            v-for="key in keys"
            :key="key.id"
            class="key-card"
            :class="{ expanded: unlockEditorKeyId === key.id }"
          >
            <div class="key-row">
              <div class="key-info">
                <span class="key-label">
                  {{ key.label }}
                  <small v-if="key.is_unlocked" class="unlocked-pill">{{ $t('settings.pgp.unlockedPill') }}</small>
                </span>
                <span class="key-meta">{{ key.key_type === 'private' ? $t('settings.pgp.keyTypePrivate') : $t('settings.pgp.keyTypePublic') }}</span>
                <small
                  v-if="key.key_type === 'private'"
                  class="security-pill"
                  :class="{ ok: key.has_passphrase, warn: !key.has_passphrase }"
                >
                  {{ key.has_passphrase ? $t('settings.pgp.passphraseStored') : $t('settings.pgp.noPassphraseStored') }}
                </small>
                <span class="key-fingerprint">{{ formatFingerprint(key.fingerprint) }}</span>
              </div>
              <div class="key-card-actions">
                <AppButton
                  v-if="key.key_type === 'private' && !key.is_unlocked"
                  size="sm"
                  :disabled="unlockingKeyId !== null"
                  @click="onUnlockClick(key)"
                >
                  <template v-if="unlockingKeyId === key.id" #icon>
                    <span class="btn-spinner" aria-hidden="true" />
                  </template>
                  {{ unlockingKeyId === key.id ? $t('settings.pgp.unlocking') : unlockEditorKeyId === key.id ? $t('settings.pgp.unlockNow') : $t('settings.pgp.unlock') }}
                </AppButton>
                <AppButton
                  variant="danger"
                  size="sm"
                  :disabled="deletingKeyId === key.id"
                  @click="removeKey(key)"
                >
                  {{ deletingKeyId === key.id ? $t('settings.pgp.deleting') : $t('common.delete') }}
                </AppButton>
              </div>
            </div>
            <div v-if="key.key_type === 'private' && unlockEditorKeyId === key.id" class="unlock-editor">
              <input
                v-model="unlockPassphrase"
                type="password"
                :placeholder="$t('settings.pgp.unlockPassphrasePlaceholder')"
                @keydown.enter.prevent="submitUnlockPassphrase(key)"
              />
              <AppButton
                :disabled="unlockingKeyId !== null || !unlockPassphrase.trim()"
                @click="submitUnlockPassphrase(key)"
              >
                <template v-if="unlockingKeyId === key.id" #icon>
                  <span class="btn-spinner" aria-hidden="true" />
                </template>
                {{ $t('settings.pgp.unlock') }}
              </AppButton>
              <AppButton variant="ghost" :disabled="unlockingKeyId === key.id" @click="cancelUnlockEditor">
                {{ $t('common.cancel') }}
              </AppButton>
            </div>
          </article>
        </div>
      </section>

      <details class="keys pgp-key-task" :open="keys.length === 0">
        <summary>{{ $t('settings.pgp.importKeys') }}</summary>
        <div class="pgp-key-task-body">

          <div
            class="dropzone"
            :class="{ active: dragActive, loaded: !!pendingImportFileName }"
            @dragenter="onDragEnter"
            @dragover="onDragOver"
            @dragleave="onDragLeave"
            @drop="onDrop"
          >
            <p v-if="dragActive" class="dropzone-title">
              {{ $t('settings.pgp.dropRelease') }}
            </p>
            <p v-else-if="pendingImportFileName" class="dropzone-title">
              {{ $t('settings.pgp.dropLoaded', { name: pendingImportFileName }) }}
            </p>
            <p v-else class="dropzone-title">
              {{ $t('settings.pgp.dropHint') }}
            </p>
            <AppButton @click="openFilePicker">{{ $t('settings.pgp.chooseFile') }}</AppButton>
            <input
              ref="fileInput"
              class="hidden-input"
              type="file"
              accept=".asc,.txt,text/plain,application/pgp-keys"
              @change="onFilePicked"
            />
          </div>

          <div class="field-grid">
            <label class="field">
              <span class="field-label">{{ $t('settings.pgp.labelPrefix') }}</span>
              <input v-model="importLabelPrefix" :placeholder="$t('settings.pgp.labelPrefixPlaceholder')" />
            </label>
            <label class="field">
              <span class="field-label">{{ $t('settings.pgp.passphraseOptional') }}</span>
              <input v-model="importPassphrase" type="password" :placeholder="$t('settings.pgp.importPassphrasePlaceholder')" />
            </label>
          </div>

          <label class="field">
            <span class="field-label">{{ $t('settings.pgp.keyMaterial') }}</span>
            <div class="secure-material">
              <p v-if="importArmored.trim()" class="secure-material-info">
                {{ $t('settings.pgp.keyMaterialLoaded') }}
              </p>
              <p v-else class="secure-material-info">
                {{ $t('settings.pgp.keyMaterialEmpty') }}
              </p>
              <div class="secure-material-actions">
                <AppButton @click="showImportArmored = !showImportArmored">
                  {{ showImportArmored ? $t('settings.pgp.hideManualInput') : $t('settings.pgp.pasteManually') }}
                </AppButton>
                <AppButton v-if="importArmored.trim()" variant="ghost" @click="clearImportMaterial">
                  {{ $t('settings.pgp.clearMaterial') }}
                </AppButton>
              </div>
            </div>
            <textarea
              v-if="showImportArmored"
              v-model="importArmored"
              rows="7"
              :placeholder="$t('settings.pgp.keyMaterialPlaceholder')"
              autocomplete="off"
              autocorrect="off"
              autocapitalize="off"
              spellcheck="false"
            />
          </label>

          <div class="key-actions">
            <AppButton
              :disabled="validatingImportPassphrase || importingBundle"
              @click="verifyImportPassphrase(false)"
            >
              {{ validatingImportPassphrase ? $t('settings.pgp.checking') : $t('settings.pgp.verifyPassphrase') }}
            </AppButton>
            <AppButton variant="primary" :disabled="importingBundle" @click="importFromTextarea">
              {{ importingBundle ? $t('settings.pgp.importing') : $t('settings.pgp.importKeys') }}
            </AppButton>
          </div>
          <p v-if="importValidationText" class="inline-status" :class="importValidationTone">
            {{ importValidationText }}
          </p>
        </div>
      </details>

      <details class="keys pgp-key-task">
        <summary>{{ $t('settings.pgp.generateTitle') }}</summary>
        <div class="pgp-key-task-body">
          <p class="keys-desc">{{ $t('settings.pgp.generateDescription') }}</p>

          <div class="field-grid">
            <label class="field">
              <span class="field-label">{{ $t('common.name') }}</span>
              <input v-model="generateName" :placeholder="$t('settings.pgp.generateNamePlaceholder')" />
            </label>
            <label class="field">
              <span class="field-label">{{ $t('common.email') }}</span>
              <input v-model="generateEmail" type="email" :placeholder="$t('settings.pgp.generateEmailPlaceholder')" />
            </label>
          </div>

          <div class="field-grid">
            <label class="field">
              <span class="field-label">{{ $t('settings.pgp.labelOptional') }}</span>
              <input v-model="generateLabel" :placeholder="$t('settings.pgp.generateLabelPlaceholder')" />
            </label>
            <label class="field">
              <span class="field-label">{{ $t('settings.pgp.passphraseOptional') }}</span>
              <input v-model="generatePassphrase" type="password" :placeholder="$t('settings.pgp.generatePassphrasePlaceholder')" />
            </label>
          </div>

          <div class="key-actions">
            <AppButton variant="primary" :disabled="generatingKey" @click="generateKey">
              {{ generatingKey ? $t('settings.pgp.generating') : $t('settings.pgp.generateTitle') }}
            </AppButton>
          </div>

          <label v-if="generatedPublicKey" class="field">
            <span class="field-label">{{ generatedFingerprint
              ? $t('settings.pgp.generatedPublicKeyWithFingerprint', { fingerprint: generatedFingerprint })
              : $t('settings.pgp.generatedPublicKey') }}</span>
            <textarea :value="generatedPublicKey" rows="6" readonly />
            <div class="key-actions">
              <AppButton @click="copyPublicKey">{{ $t('settings.pgp.copyPublicKey') }}</AppButton>
            </div>
          </label>
        </div>
      </details>
    </section>

    <div v-if="additionalPlugins.length > 0" class="plugin-list">
      <article v-for="plugin in (showPlugins ? additionalPlugins : [])" :key="plugin.id" class="plugin-card">
        <div class="plugin-info">
          <span class="plugin-name">{{ plugin.name }} <small class="plugin-version">v{{ plugin.version }}</small></span>
          <span class="plugin-desc">{{ pluginDescription(plugin.id) }}</span>
          <span class="plugin-meta">API {{ plugin.apiVersion }} · {{ plugin.trust.replace('_', ' ') }}</span>
          <span class="plugin-meta">{{ $t('settings.plugins.signature', { status: plugin.signatureVerified ? $t('settings.plugins.signatureVerified') : $t('settings.plugins.signatureUnverified') }) }}</span>
          <span class="plugin-meta">{{ $t('settings.plugins.capabilities', { list: plugin.capabilities.join(', ') || $t('settings.plugins.noCapabilities') }) }}</span>
          <div v-if="plugin.capabilities.length" class="permission-grid">
            <ToggleSwitch
              v-for="capability in plugin.capabilities"
              :key="capability"
              class="permission-toggle"
              :model-value="hasGrant(plugin, capability)"
              @update:model-value="toggleCapability(plugin, capability)"
            >
              {{ capability }}
            </ToggleSwitch>
          </div>
          <span v-if="unenforcedCapabilities(plugin).length" class="plugin-meta">
            {{ $t('settings.plugins.capabilitiesNotEnforced', { list: unenforcedCapabilities(plugin).join(', ') }) }}
          </span>
          <span v-if="plugin.events.length" class="plugin-meta">{{ $t('settings.plugins.events', { list: plugin.events.join(', ') }) }}</span>
          <span v-if="plugin.uiSlots.length" class="plugin-meta">{{ $t('settings.plugins.uiSlots', { list: plugin.uiSlots.join(', ') }) }}</span>
          <span v-if="metricsByPlugin.get(plugin.id)" class="plugin-meta">
            {{ $t('settings.plugins.metrics', {
              calls: metricsByPlugin.get(plugin.id)?.calls ?? 0,
              denied: metricsByPlugin.get(plugin.id)?.denied ?? 0,
              errors: metricsByPlugin.get(plugin.id)?.errors ?? 0,
            }) }}
          </span>
          <span v-if="plugin.manifestErrors.length" class="plugin-error">
            {{ plugin.manifestErrors.join('; ') }} {{ $t('settings.plugins.disabledByManifest') }}
          </span>
          <span v-else-if="plugin.disabledBy === 'timeouts'" class="plugin-error">{{ $t('settings.plugins.disabledByTimeouts') }}</span>
        </div>
        <ToggleSwitch
          class="plugin-enabled"
          :label="$t('settings.plugins.enabledSwitch')"
          :model-value="plugin.enabled"
          :disabled="plugin.manifestErrors.length > 0"
          @update:model-value="setPluginEnabled(plugin, $event)"
        />
      </article>
    </div>

    <section v-if="showPlugins && pluginAudit.length > 0" class="keys">
      <header class="keys-header">
        <h4>{{ $t('settings.plugins.auditTitle') }}</h4>
      </header>
      <div class="audit-list">
        <div v-for="record in pluginAudit" :key="`${record.created_at}-${record.plugin_id}-${record.command}`" class="audit-row">
          <span>{{ record.plugin_id }}</span>
          <span>{{ record.command }}</span>
          <span :class="record.decision === 'allowed' ? 'audit-ok' : 'audit-denied'">{{ record.decision }}</span>
          <small>{{ record.reason || record.capability || record.created_at }}</small>
        </div>
      </div>
    </section>

    <section v-if="showPlugins" class="keys">
      <header class="keys-header">
        <h4>{{ $t('settings.plugins.webhooksTitle') }}</h4>
      </header>
      <div class="field-grid">
        <label class="field">
          <span class="field-label">{{ $t('settings.plugins.webhookUrl') }}</span>
          <input v-model="webhookUrl" :placeholder="$t('settings.plugins.webhookUrlPlaceholder')" />
        </label>
        <label class="field">
          <span class="field-label">{{ $t('settings.plugins.webhookEvents') }}</span>
          <input v-model="webhookEvents" />
        </label>
      </div>
      <div class="key-actions">
        <AppButton :disabled="!webhookUrl.trim()" @click="addWebhook">
          {{ $t('settings.plugins.addWebhook') }}
        </AppButton>
      </div>
      <div v-if="webhooks.length" class="webhook-list">
        <div v-for="webhook in webhooks" :key="webhook.id" class="webhook-row">
          <span class="webhook-url">{{ webhook.url }}</span>
          <span class="webhook-events">
            <span v-for="event in webhookEventList(webhook)" :key="event" class="webhook-chip">
              {{ event }}
            </span>
            <small v-if="!webhookEventList(webhook).length">
              {{ $t('settings.plugins.webhookNoEvents') }}
            </small>
          </span>
          <TimeAgo :date="webhook.updated_at" />
          <ToggleSwitch
            :model-value="webhook.enabled !== 0"
            @update:model-value="setWebhookEnabled(webhook, $event)"
          >
            {{ webhook.enabled ? $t('common.enabled') : $t('common.disabled') }}
          </ToggleSwitch>
          <IconButton
            size="sm"
            tone="danger"
            :title="$t('settings.plugins.deleteWebhook')"
            @click="removeWebhook(webhook)"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
              <path d="M3 4h10M6.5 4V2.8h3V4M5 4l.6 9h4.8L11 4" />
            </svg>
          </IconButton>
        </div>
      </div>
    </section>

    <p v-if="unlockingInProgress" class="status status-progress">{{ $t('settings.pgp.unlockingStatus') }}</p>
    <p v-if="statusText" class="status">{{ statusText }}</p>
  </div>
</template>

<style scoped>
.pgp-engine-warning {
  margin: 0 0 12px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--accent-amber) 45%, transparent);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-amber) 10%, transparent);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.pgp-scope-note {
  margin: 0 0 14px;
  padding: 10px 12px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.section {
  display: grid;
  min-width: 0;
  gap: 20px;
}

.section-header h3 {
  margin: 0;
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
  color: var(--text-primary);
}

.section-desc {
  margin: 4px 0 0;
  font-size: var(--font-size-sm);
  color: var(--text-tertiary);
}

.pgp-integration {
  display: grid;
  min-width: 0;
  gap: 14px;
  padding: 14px;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
}

.pgp-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}

.keys {
  display: grid;
  min-width: 0;
  gap: 10px;
  padding: 12px;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-base) 75%, transparent);
}

.pgp-key-task {
  display: block;
}

.pgp-access-description {
  flex-basis: 100%;
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.pgp-key-task summary {
  cursor: pointer;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
}

.pgp-key-task-body {
  display: grid;
  gap: var(--space-md);
  padding-top: var(--space-md);
}

.keys-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.keys-header h4 {
  margin: 0;
  font-size: var(--font-size-md);
  color: var(--text-primary);
}

.keys-desc {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.dropzone {
  border: 1px dashed var(--border-color);
  border-radius: var(--radius-sm);
  padding: 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  background: color-mix(in srgb, var(--bg-overlay) 45%, transparent);
  transition:
    border-color var(--duration-fast) var(--ease-out),
    background var(--duration-fast) var(--ease-out),
    box-shadow var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.dropzone.active {
  border-color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 20%, var(--bg-overlay));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-primary) 45%, transparent);
  transform: translateY(-1px);
}

.dropzone.loaded {
  border-color: color-mix(in srgb, var(--accent-green) 40%, var(--border-color));
}

.dropzone-title {
  margin: 0;
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
  font-size: var(--font-size-sm);
}

.hidden-input {
  display: none;
}

.field-grid {
  display: grid;
  gap: 10px;
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.field-label {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.field input,
.field textarea {
  display: block;
  width: 100%;
  min-width: 0;
  appearance: none;
  -webkit-appearance: none;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  padding: 8px 10px;
  font-family: var(--font-sans);
}

.field textarea {
  resize: vertical;
  min-height: 120px;
  font-family: var(--font-mono);
}

.secure-material {
  display: grid;
  gap: 8px;
  padding: 10px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-overlay) 40%, transparent);
}

.secure-material-info {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
}

.secure-material-actions {
  display: flex;
  gap: 8px;
}

.key-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.inline-status {
  margin: 0;
  font-size: var(--font-size-xs);
  font-family: var(--font-mono);
  color: var(--text-secondary);
}

.inline-status.ok {
  color: var(--accent-green);
}

.inline-status.warn {
  color: var(--accent-amber-soft);
}

.inline-status.error {
  color: var(--accent-red-soft);
}

.plugin-list,
.key-list {
  display: grid;
  gap: 0;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  overflow: hidden;
}

.plugin-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 16px;
}

.key-card {
  display: grid;
  gap: 10px;
  padding: 14px 16px;
}

.key-card.expanded {
  background: color-mix(in srgb, var(--bg-overlay) 45%, transparent);
}

.plugin-card + .plugin-card,
.key-card + .key-card {
  border-top: 1px solid var(--border-color-muted);
}

.key-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.plugin-info,
.key-info {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.plugin-name {
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.plugin-version {
  font-weight: var(--weight-medium);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.plugin-desc {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  line-height: 1.4;
}

.plugin-meta,
.plugin-error,
.plugin-contract {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  line-height: 1.4;
}

.plugin-error {
  color: var(--accent-red-soft);
}

.plugin-contract {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding-top: 8px;
}

.permission-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding-top: 6px;
}


.plugin-enabled {
  flex-shrink: 0;
}

.audit-list {
  display: grid;
  gap: 6px;
}

.audit-row {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr) auto minmax(0, 1.6fr);
  gap: 8px;
  align-items: center;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.audit-row span,
.audit-row small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.webhook-list {
  display: grid;
  gap: var(--space-sm);
}

.webhook-row {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(0, 1.4fr) auto auto auto;
  gap: var(--space-sm);
  align-items: center;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}

.webhook-url {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.webhook-events {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-xs);
  min-width: 0;
}

.webhook-chip {
  padding: 1px var(--space-sm);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-xs);
  font-size: var(--font-size-2xs);
}

.audit-ok {
  color: var(--accent-green);
}

.audit-denied {
  color: var(--accent-red-soft);
}

.key-label {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
}

.key-meta {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  text-transform: capitalize;
}

.key-card-actions {
  display: inline-flex;
  gap: 8px;
}

.unlock-editor {
  display: grid;
  gap: 8px;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
}

.unlock-editor input {
  display: block;
  width: 100%;
  min-width: 0;
  appearance: none;
  -webkit-appearance: none;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  padding: 8px 10px;
  font-family: var(--font-sans);
}

.unlocked-pill {
  margin-left: 6px;
  font-size: var(--font-size-2xs);
  color: var(--accent-green);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.security-pill {
  display: inline-flex;
  align-items: center;
  width: fit-content;
  margin-top: 2px;
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.security-pill.ok {
  color: var(--accent-green);
}

.security-pill.warn {
  color: var(--accent-amber-soft);
}

.key-fingerprint {
  font-family: var(--font-mono);
  font-size: var(--font-size-2xs);
  color: var(--text-tertiary);
}

.btn-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid color-mix(in srgb, currentColor 30%, transparent);
  border-top-color: currentColor;
  border-radius: var(--radius-full);
  animation: spin 0.7s linear infinite;
  margin-right: 6px;
}

.status {
  margin: 0;
  color: var(--accent-primary);
  font-size: var(--font-size-xs);
  font-family: var(--font-mono);
}

.status-progress {
  color: var(--text-secondary);
}

.empty {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

@media (max-width: 900px) {
  .pgp-header {
    flex-direction: column;
    align-items: stretch;
  }

  .dropzone {
    flex-direction: column;
    align-items: flex-start;
  }

  .field-grid {
    grid-template-columns: 1fr;
  }

  .secure-material-actions {
    flex-wrap: wrap;
  }

  .unlock-editor {
    grid-template-columns: 1fr;
  }
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
