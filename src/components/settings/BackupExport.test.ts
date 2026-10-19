// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { answerConfirms, type DialogStub } from '@/test/dialogs';
import type { ConfirmRequest } from '@/composables/useConfirm';

const backend = vi.hoisted(() => ({
  backendCancelPendingRestore: vi.fn(),
  backendChooseDirectory: vi.fn(),
  backendChooseSavePath: vi.fn(),
  backendCreateBackup: vi.fn(),
  backendCredentialsStatus: vi.fn(),
  backendExportMailbox: vi.fn(),
  backendExportPlatformData: vi.fn(),
  backendImportMbox: vi.fn(),
  backendImportPlatformData: vi.fn(),
  backendListBackups: vi.fn(),
  backendPruneBackups: vi.fn(),
  backendRestoreBackup: vi.fn(),
  backendCancelImport: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
const events = vi.hoisted(() => ({
  listenToBackend: vi.fn(),
  handlers: new Map<string, (payload: unknown) => void>(),
}));
vi.mock('@/services/backend', () => backend);
vi.mock('@tauri-apps/plugin-dialog', () => dialog);
vi.mock('@/services/appEvents', () => ({
  listenToBackend: (name: string, handler: (payload: unknown) => void) => {
    events.handlers.set(name, handler);
    return Promise.resolve(() => events.handlers.delete(name));
  },
}));
const accountsState = vi.hoisted(() => ({
  load: vi.fn(),
  emailAccounts: [] as { id: string; email: string; displayName: string }[],
  activeAccountId: null as string | null,
}));
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => accountsState }));

import BackupExport from './BackupExport.vue';

let confirms: DialogStub<ConfirmRequest, boolean>;

afterEach(() => confirms.stop());

const entry = {
  path: '/backups/satchel-20260828-101500',
  name: 'satchel-20260828-101500',
  created_at: '2026-08-28T08:15:00Z',
  bytes: 2048,
  account_count: 2,
  message_count: 1500,
  master_key_files: 1,
  master_key_wrapped: true,
};

async function typeBackupPassphrase(wrapper: ReturnType<typeof mount>, value: string) {
  const field = wrapper
    .findAll('input[type="password"]')
    .find((input) => input.attributes('autocomplete') === 'new-password');
  if (!field) throw new Error('no backup passphrase field');
  await field.setValue(value);
}

function button(wrapper: ReturnType<typeof mount>, label: string) {
  const found = wrapper.findAll('button').find((b) => b.text().includes(label));
  if (!found) throw new Error(`no button ${label}`);
  return found;
}

beforeEach(() => {
  vi.clearAllMocks();
  backend.backendSupportsNativeCommands.mockReturnValue(true);
  backend.backendCredentialsStatus.mockResolvedValue({
    total_accounts: 2,
    encrypted_accounts: 2,
    plaintext_accounts: 0,
    master_key_present: true,
    database_bytes: 4096,
    restore_pending: false,
  });
  backend.backendListBackups.mockResolvedValue([entry]);
  backend.backendCreateBackup.mockResolvedValue(entry);
  backend.backendRestoreBackup.mockResolvedValue({
    source: entry.path,
    pending_path: '/profile/satchel.db.restore-pending',
    account_count: 2,
    message_count: 1500,
    master_key_files: 1,
  });
  dialog.open.mockResolvedValue('/backups');
  backend.backendChooseDirectory.mockResolvedValue('/backups');
  backend.backendChooseSavePath.mockResolvedValue('/export.mbox');
  events.handlers.clear();
  accountsState.emailAccounts = [];
  accountsState.activeAccountId = null;
  confirms = answerConfirms(true);
});

/** Play a backend `import:progress` event into the mounted component. */
async function emitProgress(payload: Record<string, unknown>) {
  const handler = events.handlers.get('import:progress');
  if (!handler) throw new Error('no import:progress listener');
  handler({
    kind: 'mbox',
    done: 0,
    skipped: 0,
    total: 0,
    bytes_done: 0,
    bytes_total: 0,
    running: true,
    eta_seconds: null,
    finished_reason: null,
    last_error: null,
    ...payload,
  });
  await Promise.resolve();
}

describe('database backup section', () => {
  it('shows the credential status on mount', async () => {
    const wrapper = mount(BackupExport);
    await vi.waitFor(() =>
      expect(wrapper.get('[data-testid="credentials-status"]').text()).toContain('2 accounts, 2 encrypted'),
    );
    expect(wrapper.text()).toContain('Master key present');
  });

  it('rejects a short passphrase even after acknowledging an unprotected export', async () => {
    const wrapper = mount(BackupExport);
    await wrapper.get('input[type="checkbox"]').setValue(true);
    await typeBackupPassphrase(wrapper, 'x');
    expect(button(wrapper, 'Create backup').attributes('disabled')).toBeDefined();
    expect(wrapper.text()).toContain('Use at least 12 characters');
    await button(wrapper, 'Create backup').trigger('click');
    expect(backend.backendCreateBackup).not.toHaveBeenCalled();
  });

  it('counts Unicode characters consistently with backend validation', async () => {
    const wrapper = mount(BackupExport);
    await typeBackupPassphrase(wrapper, '🔒'.repeat(6));
    expect(button(wrapper, 'Create backup').attributes('disabled')).toBeDefined();
    await typeBackupPassphrase(wrapper, '🔒'.repeat(12));
    expect(button(wrapper, 'Create backup').attributes('disabled')).toBeUndefined();
  });

  it('creates a backup in the chosen directory and lists it', async () => {
    const wrapper = mount(BackupExport);
    await typeBackupPassphrase(wrapper, 'backup passphrase');
    await button(wrapper, 'Create backup').trigger('click');

    await vi.waitFor(() =>
      expect(backend.backendCreateBackup).toHaveBeenCalledWith('/backups', {
        passphrase: 'backup passphrase',
        acknowledgeUnprotectedKey: false,
      }),
    );
    // The backend opens the folder dialog, so the write target is one it issued.
    expect(backend.backendChooseDirectory).toHaveBeenCalled();
    await vi.waitFor(() => expect(wrapper.text()).toContain('satchel-20260828-101500'));
    expect(wrapper.text()).toContain('Backed up: 1500 messages');
  });

  it('stages a restore for the next start after confirmation', async () => {
    const wrapper = mount(BackupExport);
    await button(wrapper, 'Show backups').trigger('click');
    await vi.waitFor(() => expect(wrapper.text()).toContain('satchel-20260828-101500'));

    await button(wrapper, 'Restore').trigger('click');

    await vi.waitFor(() =>
      expect(backend.backendRestoreBackup).toHaveBeenCalledWith(entry.path, ''),
    );
    expect(wrapper.text()).toContain('next time Satchel starts');
    // Asked in the app, not through window.confirm, and styled as destructive.
    expect(confirms.asked).toHaveBeenCalledOnce();
    expect(confirms.asked.mock.calls[0]![0]).toMatchObject({
      body: expect.stringContaining('satchel-20260828-101500'),
      tone: 'danger',
    });
  });

  it('does nothing when the restore is not confirmed', async () => {
    confirms.reply(false);
    const wrapper = mount(BackupExport);
    await button(wrapper, 'Show backups').trigger('click');
    await vi.waitFor(() => expect(wrapper.text()).toContain('satchel-20260828-101500'));

    await button(wrapper, 'Restore').trigger('click');

    await vi.waitFor(() => expect(confirms.asked).toHaveBeenCalledOnce());
    expect(backend.backendRestoreBackup).not.toHaveBeenCalled();
  });

  it('will not back up without a passphrase or an explicit acknowledgement', async () => {
    const wrapper = mount(BackupExport);

    expect(button(wrapper, 'Create backup').attributes('disabled')).toBeDefined();
    await button(wrapper, 'Create backup').trigger('click');
    expect(backend.backendCreateBackup).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain('like a password file');
  });

  it('backs up without a passphrase once the folder is acknowledged', async () => {
    const wrapper = mount(BackupExport);
    const acknowledgement = wrapper.find('input[type="checkbox"]');
    await acknowledgement.setValue(true);

    await button(wrapper, 'Create backup').trigger('click');

    await vi.waitFor(() =>
      expect(backend.backendCreateBackup).toHaveBeenCalledWith('/backups', {
        passphrase: '',
        acknowledgeUnprotectedKey: true,
      }),
    );
    // The opt-out does not carry over to the next backup.
    await vi.waitFor(() =>
      expect(button(wrapper, 'Create backup').attributes('disabled')).toBeDefined(),
    );
  });

  it('sends the restore passphrase for a wrapped backup', async () => {
    const wrapper = mount(BackupExport);
    await button(wrapper, 'Show backups').trigger('click');
    await vi.waitFor(() => expect(wrapper.text()).toContain('key wrapped'));

    const field = wrapper
      .findAll('input[type="password"]')
      .find((input) => input.attributes('autocomplete') === 'off');
    if (!field) throw new Error('no restore passphrase field');
    await field.setValue('backup passphrase');
    await button(wrapper, 'Restore').trigger('click');

    await vi.waitFor(() =>
      expect(backend.backendRestoreBackup).toHaveBeenCalledWith(entry.path, 'backup passphrase'),
    );
  });

  it('offers to cancel a pending restore', async () => {
    backend.backendCredentialsStatus.mockResolvedValue({
      total_accounts: 1,
      encrypted_accounts: 1,
      plaintext_accounts: 0,
      master_key_present: true,
      database_bytes: 1,
      restore_pending: true,
    });
    backend.backendCancelPendingRestore.mockResolvedValue(undefined);
    const wrapper = mount(BackupExport);
    await vi.waitFor(() => expect(wrapper.text()).toContain('Discard'));

    await button(wrapper, 'Discard').trigger('click');

    await vi.waitFor(() => expect(backend.backendCancelPendingRestore).toHaveBeenCalledOnce());
  });
});

// UX-16: a multi-gigabyte mbox used to run for minutes behind nothing but a
// greyed-out button.
describe('mbox import progress', () => {
  it('shows the bar and the counter while the import runs', async () => {
    const wrapper = mount(BackupExport);
    await vi.waitFor(() => expect(events.handlers.has('import:progress')).toBe(true));

    await emitProgress({ done: 400, total: 1600, bytes_done: 250, bytes_total: 1000, eta_seconds: 120 });
    await wrapper.vm.$nextTick();

    const bar = wrapper.get('[role="progressbar"]');
    expect(bar.attributes('aria-valuenow')).toBe('25');
    expect(wrapper.get('[data-testid="import-progress"]').text()).toContain('400');
    expect(wrapper.text()).toContain('2 minutes');
    expect(button(wrapper, 'Cancel').exists()).toBe(true);
  });

  /** The first event carries no measurement yet; the bar still has to appear. */
  it('shows the bar before there is a count to show', async () => {
    const wrapper = mount(BackupExport);
    await vi.waitFor(() => expect(events.handlers.has('import:progress')).toBe(true));

    await emitProgress({ bytes_total: 4_000_000_000 });
    await wrapper.vm.$nextTick();

    expect(wrapper.find('[role="progressbar"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="import-progress"]').exists()).toBe(false);
  });

  it('takes the bar away when the run ends', async () => {
    const wrapper = mount(BackupExport);
    await vi.waitFor(() => expect(events.handlers.has('import:progress')).toBe(true));
    await emitProgress({ done: 400, total: 1600, bytes_done: 250, bytes_total: 1000 });
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[role="progressbar"]').exists()).toBe(true);

    await emitProgress({
      done: 1000,
      bytes_done: 1000,
      bytes_total: 1000,
      running: false,
      finished_reason: 'completed',
    });
    await wrapper.vm.$nextTick();

    expect(wrapper.find('[role="progressbar"]').exists()).toBe(false);
  });

  it('counts tables rather than bytes for the platform import', async () => {
    const wrapper = mount(BackupExport);
    await vi.waitFor(() => expect(events.handlers.has('import:progress')).toBe(true));

    await emitProgress({ kind: 'platform', done: 3, total: 12 });
    await wrapper.vm.$nextTick();

    expect(wrapper.get('[role="progressbar"]').attributes('aria-valuenow')).toBe('25');
    expect(wrapper.get('[data-testid="import-progress"]').text()).toContain('3 of 12 tables');
  });

  it('asks the backend to stop and says how far it got', async () => {
    backend.backendImportMbox.mockResolvedValue({
      path: '/archive.mbox',
      message_count: 812,
      skipped: 3,
      cancelled: true,
    });
    dialog.open.mockResolvedValue('/archive.mbox');
    accountsState.emailAccounts = [{ id: 'a1', email: 'a@example.test', displayName: 'A' }];
    const wrapper = mount(BackupExport);
    await vi.waitFor(() => expect(events.handlers.has('import:progress')).toBe(true));

    await button(wrapper, 'Import mbox').trigger('click');
    await emitProgress({ done: 400, total: 1600, bytes_done: 250, bytes_total: 1000 });
    await wrapper.vm.$nextTick();
    await button(wrapper, 'Cancel').trigger('click');

    await vi.waitFor(() => expect(backend.backendCancelImport).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(wrapper.text()).toContain('Stopped: 812 messages imported'));
  });

  /** No event at all — mock mode, or an older backend. */
  it('returns to rest without a single progress event', async () => {
    backend.backendImportMbox.mockResolvedValue({
      path: '/archive.mbox',
      message_count: 5,
      skipped: 0,
      cancelled: false,
    });
    dialog.open.mockResolvedValue('/archive.mbox');
    accountsState.emailAccounts = [{ id: 'a1', email: 'a@example.test', displayName: 'A' }];
    const wrapper = mount(BackupExport);

    await button(wrapper, 'Import mbox').trigger('click');

    await vi.waitFor(() => expect(wrapper.text()).toContain('Imported: 5 messages'));
    expect(wrapper.find('[role="progressbar"]').exists()).toBe(false);
    expect(button(wrapper, 'Import mbox').attributes('disabled')).toBeUndefined();
  });

  it('reports a failed import as an alert, not a grey line', async () => {
    backend.backendImportMbox.mockRejectedValue(new Error('mbox unreadable'));
    dialog.open.mockResolvedValue('/archive.mbox');
    accountsState.emailAccounts = [{ id: 'a1', email: 'a@example.test', displayName: 'A' }];
    const wrapper = mount(BackupExport);

    await button(wrapper, 'Import mbox').trigger('click');

    await vi.waitFor(() =>
      expect(wrapper.get('[role="alert"]').text()).toContain('mbox unreadable'),
    );
  });
});
