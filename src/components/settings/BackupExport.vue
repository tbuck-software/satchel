<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import {
  backendChooseDirectory,
  backendChooseSavePath,
  backendCancelImport,
  backendCancelPendingRestore,
  backendCreateBackup,
  backendCredentialsStatus,
  backendExportMailbox,
  backendExportPlatformData,
  backendImportMbox,
  backendImportPlatformData,
  backendListBackups,
  backendPruneBackups,
  backendRestoreBackup,
  backendSupportsNativeCommands,
  type BackendBackupEntry,
  type BackendCredentialsStatus,
  type BackendImportProgress,
} from '@/services/backend';
import { listenToBackend } from '@/services/appEvents';
import { pickDirectory, pickFile } from '@/services/fileDialog';
import { useToast } from '@/composables/useToast';
import { formatBytes } from '@/utils/formatBytes';
import AppButton from '@/components/ui/AppButton.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

const { locale, t } = useI18n();
const accounts = useAccountsStore();
const toast = useToast();
const accountId = ref<string>('');
const status = ref('');
const errorStatus = ref('');
const busy = ref(false);

/**
 * Progress of the running import, or `null`.
 *
 * A gigabyte-scale mbox used to run for minutes behind a greyed-out button.
 * The backend emits `import:progress`; without an event (mock mode, an older
 * build) `busy` and the status line stay exactly as they were.
 */
const importProgress = ref<BackendImportProgress | null>(null);
let unlistenImport: (() => void) | null = null;

onMounted(async () => {
  void accounts.load();
  void loadCredentialsStatus();
  if (!backendSupportsNativeCommands()) return;
  unlistenImport = await listenToBackend<BackendImportProgress>('import:progress', (payload) => {
    importProgress.value = payload.running ? payload : null;
  });
});

onBeforeUnmount(() => {
  unlistenImport?.();
});

const importPercent = computed(() => {
  const progress = importProgress.value;
  if (!progress) return 0;
  // Bytes for an mbox, tables for the platform import: the mbox message total
  // is an estimate and would make the bar jump.
  const [done, total] = progress.bytes_total > 0
    ? [progress.bytes_done, progress.bytes_total]
    : [progress.done, progress.total];
  if (total <= 0) return 0;
  return Math.min(100, Math.round((done / total) * 100));
});

const importLine = computed(() => {
  const progress = importProgress.value;
  if (!progress) return '';
  // Nothing measured yet: the bar alone says the run has started, which beats
  // a counter reading "0 of about 0".
  if (progress.total <= 0 && progress.done <= 0) return '';
  if (progress.kind === 'platform') {
    return t('settings.backup.importingTables', {
      done: formatCount(progress.done),
      total: formatCount(progress.total),
    });
  }
  return t('settings.backup.importingMessages', {
    done: formatCount(progress.done),
    total: formatCount(Math.max(progress.total, progress.done)),
  });
});

const importEtaLabel = computed(() => {
  const seconds = importProgress.value?.eta_seconds ?? 0;
  if (seconds <= 0) return '';
  return t('settings.backup.importEta', { duration: formatDuration(seconds) });
});

async function cancelImport() {
  try {
    await backendCancelImport();
  } catch (err) {
    reportError(err);
  }
}

function formatCount(value: number): string {
  return value.toLocaleString(locale.value);
}

function formatDuration(seconds: number): string {
  if (seconds < 90) return t('settings.backup.durationSeconds', Math.max(1, Math.round(seconds)));
  const minutes = Math.round(seconds / 60);
  if (minutes < 90) return t('settings.backup.durationMinutes', minutes);
  return t('settings.backup.durationHours', Math.round(minutes / 60));
}

/** An import or export failure is worth more than a grey line of text. */
function reportError(err: unknown) {
  const message = err instanceof Error ? err.message : String(err);
  errorStatus.value = message;
  status.value = '';
  toast.error(message);
}

/** Backup directory chosen in this session; the list follows it. */
const backupDir = ref('');
const backups = ref<BackendBackupEntry[]>([]);
const credentials = ref<BackendCredentialsStatus | null>(null);
const backupStatus = ref('');
const BACKUPS_TO_KEEP = 5;

/**
 * A backup folder holds the key to every stored credential. Either it is
 * wrapped under a passphrase, or the user says out loud that this one is not.
 */
const backupPassphrase = ref('');
const acknowledgeUnprotectedKey = ref(false);
const restorePassphrase = ref('');
const backupPassphraseTooShort = computed(
  () => backupPassphrase.value.length > 0 && [...backupPassphrase.value].length < 12,
);
const canCreateBackup = computed(
  () => backupPassphrase.value.length > 0
    ? !backupPassphraseTooShort.value
    : acknowledgeUnprotectedKey.value,
);
const anyBackupWrapped = computed(() => backups.value.some((entry) => entry.master_key_wrapped));

async function loadCredentialsStatus() {
  if (!backendSupportsNativeCommands()) return;
  try {
    credentials.value = await backendCredentialsStatus();
  } catch (err) {
    backupStatus.value = err instanceof Error ? err.message : String(err);
  }
}

async function pickBackupDir(): Promise<string | null> {
  const selected = await pickDirectory(backupDir.value);
  if (!selected) return null;
  backupDir.value = selected;
  return selected;
}

async function refreshBackups() {
  if (!backupDir.value) return;
  backups.value = await backendListBackups(backupDir.value);
}

/** A folder the backend will let the backup be written into. */
async function chooseBackupTarget(): Promise<string | null> {
  const selected = await backendChooseDirectory(backupDir.value);
  if (!selected) return null;
  backupDir.value = selected;
  return selected;
}

async function createBackupNow() {
  if (!canCreateBackup.value) return;
  if (!backendSupportsNativeCommands()) {
    backupStatus.value = t('settings.backup.databaseDesktopOnly');
    return;
  }
  busy.value = true;
  backupStatus.value = '';
  try {
    const dir = await chooseBackupTarget();
    if (!dir) return;
    const entry = await backendCreateBackup(dir, {
      passphrase: backupPassphrase.value,
      acknowledgeUnprotectedKey: acknowledgeUnprotectedKey.value,
    });
    const keyNote = entry.master_key_files === 0
      ? t('settings.backup.missingMasterKeyWarning')
      : '';
    backupStatus.value = t('settings.backup.databaseCreated', {
      messages: entry.message_count,
      accounts: entry.account_count,
      size: formatBytes(entry.bytes),
      path: entry.path,
      keyNote,
    });
    // The opt-out is per backup: the next one asks again.
    acknowledgeUnprotectedKey.value = false;
    await refreshBackups();
  } catch (err) {
    backupStatus.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}

async function showBackups() {
  if (!backendSupportsNativeCommands()) {
    backupStatus.value = t('settings.backup.databaseDesktopOnly');
    return;
  }
  busy.value = true;
  backupStatus.value = '';
  try {
    const dir = await pickBackupDir();
    if (!dir) return;
    await refreshBackups();
    if (backups.value.length === 0) backupStatus.value = t('settings.backup.noBackups');
  } catch (err) {
    backupStatus.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}

async function restoreBackup(entry: BackendBackupEntry) {
  const ok = window.confirm(
    t('settings.backup.restoreConfirm', { name: entry.name }),
  );
  if (!ok) return;
  busy.value = true;
  backupStatus.value = '';
  try {
    const staged = await backendRestoreBackup(entry.path, restorePassphrase.value);
    backupStatus.value = t('settings.backup.restoreStaged', {
      messages: staged.message_count,
      accounts: staged.account_count,
    });
    await loadCredentialsStatus();
  } catch (err) {
    backupStatus.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}

async function cancelRestore() {
  busy.value = true;
  try {
    await backendCancelPendingRestore();
    backupStatus.value = t('settings.backup.restoreCancelled');
    await loadCredentialsStatus();
  } catch (err) {
    backupStatus.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}

async function pruneOldBackups() {
  if (!backupDir.value) return;
  const ok = window.confirm(t('settings.backup.pruneConfirm', { count: BACKUPS_TO_KEEP }));
  if (!ok) return;
  busy.value = true;
  try {
    const removed = await backendPruneBackups(backupDir.value, BACKUPS_TO_KEEP);
    backupStatus.value = removed.length === 0
      ? t('settings.backup.nothingToPrune')
      : t('settings.backup.pruned', { count: removed.length });
    await refreshBackups();
  } catch (err) {
    backupStatus.value = err instanceof Error ? err.message : String(err);
  } finally {
    busy.value = false;
  }
}

function formatDate(iso: string) {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleString(locale.value, { dateStyle: 'medium', timeStyle: 'short' });
}

const accountChoice = computed(() => {
  if (!accountId.value) return null;
  return accounts.emailAccounts.find((a) => a.id === accountId.value) ?? null;
});

async function exportNow() {
  if (!backendSupportsNativeCommands()) {
    status.value = t('settings.backup.desktopOnlyBackup');
    return;
  }
  busy.value = true;
  status.value = '';
  errorStatus.value = '';
  try {
    const path = await backendChooseSavePath({
      suggestedName: `satchel-${accountChoice.value?.email ?? 'all'}-${new Date()
        .toISOString()
        .slice(0, 10)}.mbox`,
      filterName: 'Mbox',
      extensions: ['mbox'],
    });
    if (!path) {
      busy.value = false;
      return;
    }
    const result = await backendExportMailbox({
      path,
      accountId: accountId.value || null,
    });
    status.value = t('settings.backup.exported', {
      count: result.message_count,
      size: formatBytes(result.bytes_written),
      path: result.path,
    });
  } catch (err) {
    reportError(err);
  } finally {
    busy.value = false;
  }
}

async function importNow() {
  if (!backendSupportsNativeCommands()) {
    status.value = t('settings.backup.desktopOnlyImport');
    return;
  }
  const targetAccountId = accountId.value
    || accounts.emailAccounts.find((account) => account.id === accounts.activeAccountId)?.id
    || accounts.emailAccounts[0]?.id;
  if (!targetAccountId) {
    status.value = t('settings.backup.selectTargetAccount');
    return;
  }
  busy.value = true;
  status.value = '';
  errorStatus.value = '';
  try {
    const selected = await pickFile([{ name: 'Mailbox', extensions: ['mbox', 'eml', 'txt'] }]);
    if (!selected) {
      busy.value = false;
      return;
    }
    const result = await backendImportMbox({ path: selected, accountId: targetAccountId });
    status.value = result.cancelled
      ? t('settings.backup.importCancelled', {
        count: result.message_count,
        skipped: result.skipped,
      })
      : t('settings.backup.imported', {
        count: result.message_count,
        skipped: result.skipped,
      });
  } catch (err) {
    reportError(err);
  } finally {
    busy.value = false;
  }
}

async function exportPlatformNow() {
  if (!backendSupportsNativeCommands()) {
    status.value = t('settings.backup.desktopOnlyExport');
    return;
  }
  busy.value = true;
  status.value = '';
  errorStatus.value = '';
  try {
    const path = await backendChooseSavePath({
      suggestedName: `satchel-platform-${new Date().toISOString().slice(0, 10)}.json`,
      filterName: 'Satchel Platform JSON',
      extensions: ['json'],
    });
    if (!path) {
      busy.value = false;
      return;
    }
    const result = await backendExportPlatformData({ path, redactSensitive: true });
    status.value = t('settings.backup.platformExported', {
      rows: result.row_count,
      tables: result.table_count,
      path: result.path,
    });
  } catch (err) {
    reportError(err);
  } finally {
    busy.value = false;
  }
}

async function importPlatformNow() {
  if (!backendSupportsNativeCommands()) {
    status.value = t('settings.backup.desktopOnlyImport');
    return;
  }
  busy.value = true;
  status.value = '';
  errorStatus.value = '';
  try {
    const selected = await pickFile([{ name: 'Satchel Platform JSON', extensions: ['json'] }]);
    if (!selected) {
      busy.value = false;
      return;
    }
    const result = await backendImportPlatformData({ path: selected });
    status.value = t('settings.backup.platformImported', {
      rows: result.row_count,
      tables: result.table_count,
    });
  } catch (err) {
    reportError(err);
  } finally {
    busy.value = false;
  }
}

</script>

<template>
  <section>
    <h3>{{ $t('settings.sections.backup.label') }}</h3>
    <p class="hint">
      {{ $t('settings.backup.hintBefore') }}
      <code>{{ $t('settings.backup.mboxExtension') }}</code>{{ $t('settings.backup.hintAfter') }}
    </p>

    <label class="field">
      <span>{{ $t('settings.backup.accountLabel') }}</span>
      <select v-model="accountId">
        <option value="">{{ $t('settings.backup.allAccounts') }}</option>
        <option v-for="a in accounts.emailAccounts" :key="a.id" :value="a.id">
          {{ a.displayName }} &lt;{{ a.email }}&gt;
        </option>
      </select>
    </label>

    <div class="actions">
      <AppButton variant="primary" :disabled="busy" @click="exportNow">
        {{ busy ? $t('settings.backup.exporting') : $t('settings.backup.exportMailbox') }}
      </AppButton>
      <AppButton variant="secondary" :disabled="busy" @click="importNow">
        {{ $t('settings.backup.importMbox') }}
      </AppButton>
    </div>
    <div class="actions">
      <AppButton variant="secondary" :disabled="busy" @click="exportPlatformNow">
        {{ $t('settings.backup.exportPlatform') }}
      </AppButton>
      <AppButton variant="secondary" :disabled="busy" @click="importPlatformNow">
        {{ $t('settings.backup.importPlatform') }}
      </AppButton>
    </div>
    <div v-if="importProgress" class="run">
      <div
        class="bar"
        role="progressbar"
        :aria-valuenow="importPercent"
        aria-valuemin="0"
        aria-valuemax="100"
        :aria-label="$t('settings.backup.importProgressLabel')"
      >
        <span :style="{ width: `${importPercent}%` }" />
      </div>
      <p class="run-line">
        <span v-if="importLine" data-testid="import-progress">{{ importLine }}</span>
        <span v-if="importEtaLabel">{{ importEtaLabel }}</span>
        <AppButton size="sm" variant="soft" @click="cancelImport">
          {{ $t('settings.backup.cancelImport') }}
        </AppButton>
      </p>
    </div>

    <p v-if="status" class="status">{{ status }}</p>
    <p v-if="errorStatus" class="status error" role="alert">{{ errorStatus }}</p>

    <h3 class="backup-title">{{ $t('settings.backup.databaseTitle') }}</h3>
    <p class="hint">{{ $t('settings.backup.databaseDescription') }}</p>
    <p v-if="credentials" class="hint" data-testid="credentials-status">
      {{
        $t('settings.backup.credentialsSummary', {
          total: credentials.total_accounts,
          encrypted: credentials.encrypted_accounts,
          masterKey: credentials.master_key_present
            ? $t('settings.backup.masterKeyPresent')
            : $t('settings.backup.masterKeyMissing'),
          database: formatBytes(credentials.database_bytes),
        })
      }}
    </p>
    <p v-if="credentials?.restore_pending" class="pending" role="status">
      {{ $t('settings.backup.restorePending') }}
      <AppButton size="sm" variant="ghost" :disabled="busy" @click="cancelRestore">
        {{ $t('settings.backup.discardRestore') }}
      </AppButton>
    </p>

    <label class="field">
      <span>{{ $t('settings.backup.keyPassphrase') }}</span>
      <input
        v-model="backupPassphrase"
        type="password"
        autocomplete="new-password"
        :aria-invalid="backupPassphraseTooShort"
        aria-describedby="backup-passphrase-hint"
        :placeholder="$t('settings.backup.keyPassphrasePlaceholder')"
      />
    </label>
    <p id="backup-passphrase-hint" class="hint" :class="{ warn: backupPassphraseTooShort }">
      {{ backupPassphraseTooShort
        ? $t('backend.errors.backupPassphraseTooShort')
        : $t('settings.backup.keyPassphraseHint') }}
    </p>
    <template v-if="!backupPassphrase">
      <ToggleSwitch v-model="acknowledgeUnprotectedKey">
        {{ $t('settings.backup.acknowledgeUnprotectedKey') }}
      </ToggleSwitch>
      <p class="hint warn">{{ $t('settings.backup.acknowledgeUnprotectedKeyHint') }}</p>
    </template>

    <div class="actions">
      <AppButton variant="primary" :disabled="busy || !canCreateBackup" @click="createBackupNow">
        {{ $t('settings.backup.createDatabaseBackup') }}
      </AppButton>
      <AppButton :disabled="busy" @click="showBackups">
        {{ $t('settings.backup.showBackups') }}
      </AppButton>
      <AppButton v-if="backups.length > BACKUPS_TO_KEEP" variant="danger" :disabled="busy" @click="pruneOldBackups">
        {{ $t('settings.backup.pruneBackups') }}
      </AppButton>
    </div>

    <label v-if="anyBackupWrapped" class="field">
      <span>{{ $t('settings.backup.restorePassphrase') }}</span>
      <input v-model="restorePassphrase" type="password" autocomplete="off" />
    </label>
    <p v-if="anyBackupWrapped" class="hint">{{ $t('settings.backup.restorePassphraseHint') }}</p>

    <ul v-if="backups.length" class="backups">
      <li v-for="entry in backups" :key="entry.path">
        <div class="backup-meta">
          <strong>{{ entry.name }}</strong>
          <small>
            {{ formatDate(entry.created_at) }} ·
            {{ $t('settings.backup.messageCount', { count: entry.message_count }) }} ·
            {{ $t('settings.backup.accountCount', { count: entry.account_count }) }} ·
            {{ formatBytes(entry.bytes) }}
            <span v-if="entry.master_key_files === 0" class="warn">
              · {{ $t('settings.backup.withoutMasterKey') }}
            </span>
            <span v-else-if="entry.master_key_wrapped">
              · {{ $t('settings.backup.wrappedMasterKey') }}
            </span>
          </small>
        </div>
        <AppButton size="sm" :disabled="busy" @click="restoreBackup(entry)">
          {{ $t('settings.backup.restore') }}
        </AppButton>
      </li>
    </ul>
    <p v-if="backupStatus" class="status" role="status">{{ backupStatus }}</p>
  </section>
</template>

<style scoped>
section { display: grid; gap: 12px; }
h3 { margin: 0; color: var(--text-primary); }
.hint { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-sm); }
code { background: var(--bg-raised); padding: 1px 6px; border-radius: var(--radius-xs); font-size: 0.9em; }
.field { display: grid; gap: 4px; font-size: var(--font-size-sm); color: var(--text-secondary); }
.field select, .field input { border: 1px solid var(--border-color); background: var(--bg-raised); border-radius: var(--radius-sm); padding: 8px 10px; color: var(--text-primary); font-family: inherit; }
.actions { display: flex; justify-content: flex-end; gap: 8px; }
.status { margin: 0; color: var(--text-tertiary); font-size: var(--font-size-xs); }
.status.error { color: var(--accent-red); }

.run { display: grid; gap: 7px; }

.bar {
  overflow: hidden;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
}

.bar span {
  display: block;
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
  transition: width var(--duration-fast) var(--ease-out);
}

.run-line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}
.backup-title { margin-top: 12px; }
.pending { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin: 0; padding: 8px 10px; border-radius: var(--radius-sm); background: color-mix(in srgb, var(--accent-amber) 12%, transparent); color: var(--accent-amber); font-size: var(--font-size-sm); }
.backups { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
.backups li { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 10px; border: 1px solid var(--border-color-muted); border-radius: var(--radius-sm); background: var(--bg-surface); }
.backup-meta { display: grid; gap: 2px; min-width: 0; }
.backup-meta strong { color: var(--text-primary); font-size: var(--font-size-sm); }
.backup-meta small { color: var(--text-tertiary); font-size: var(--font-size-xs); }
.warn { color: var(--accent-amber); }
</style>
