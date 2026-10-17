// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  unlockPgp: vi.fn(),
  unlockSmime: vi.fn(),
  loadByThread: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendUnlockPgpKeys: harness.unlockPgp,
  backendUnlockSmimeCerts: harness.unlockSmime,
}));
vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ loadByThread: harness.loadByThread }),
}));
vi.mock('@/i18n/runtime', () => ({
  t: (key: string) => key,
}));

import { useMessageSecurityActions } from './useMessageSecurityActions';

type Actions = ReturnType<typeof useMessageSecurityActions>;

/** The dialog is a component; here it is whatever the caller does with it. */
async function answerPrompt(actions: Actions, passphrase: string | null) {
  // One turn for the failing first attempt to reach the ask.
  await Promise.resolve();
  await Promise.resolve();
  expect(actions.passphraseRequest.value).not.toBeNull();
  if (passphrase === null) actions.cancelPassphrase();
  else actions.submitPassphrase(passphrase);
}

function backendError(key: string) {
  return Object.assign(new Error(key), { cause: key });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.unlockPgp.mockResolvedValue({ unlocked: 1, key_ids: ['key-1'] });
  harness.unlockSmime.mockResolvedValue({ unlocked: 1, cert_ids: ['cert-1'] });
  harness.loadByThread.mockResolvedValue(undefined);
});

describe('useMessageSecurityActions', () => {
  it('unlocks PGP without asking and reloads the thread', async () => {
    const actions = useMessageSecurityActions();

    await actions.unlockPgp('thread-1');

    expect(harness.unlockPgp).toHaveBeenCalledOnce();
    expect(harness.unlockPgp).toHaveBeenCalledWith();
    expect(actions.passphraseRequest.value).toBeNull();
    expect(harness.loadByThread).toHaveBeenCalledWith('thread-1');
    expect(actions.pgpStatus.value).toBe('mail.pgp.unlocked');
    expect(actions.pgpBusy.value).toBe(false);
  });

  it('retries PGP with a trimmed passphrase from the dialog', async () => {
    harness.unlockPgp.mockRejectedValueOnce(backendError('backend.errors.pgpStoredPassphraseRejected'));
    const actions = useMessageSecurityActions();

    const unlocking = actions.unlockPgp('thread-2');
    await answerPrompt(actions, '  secret  ');
    await unlocking;

    expect(harness.unlockPgp).toHaveBeenNthCalledWith(2, { passphrase: 'secret' });
    expect(actions.passphraseRequest.value).toBeNull();
    expect(harness.loadByThread).toHaveBeenCalledWith('thread-2');
    expect(actions.pgpStatus.value).toBe('mail.pgp.unlocked');
  });

  /**
   * The refusal that says "no key exists" cannot be answered with a
   * passphrase, so asking for one only makes the failure look like the user's
   * fault.
   */
  it('does not ask when there is no key to unlock', async () => {
    harness.unlockPgp.mockRejectedValueOnce(backendError('backend.errors.pgpNoPrivateKeys'));
    const actions = useMessageSecurityActions();

    await actions.unlockPgp('thread-3');

    expect(actions.passphraseRequest.value).toBeNull();
    expect(harness.unlockPgp).toHaveBeenCalledOnce();
    expect(harness.loadByThread).not.toHaveBeenCalled();
    expect(actions.pgpStatus.value).toBe('mail.pgp.noKeyDescription');
    expect(actions.pgpBusy.value).toBe(false);
  });

  it('tells a wrong passphrase apart from an unlock that failed for another reason', async () => {
    harness.unlockPgp
      .mockRejectedValueOnce(backendError('backend.errors.pgpStoredPassphraseRejected'))
      .mockRejectedValueOnce(backendError('backend.errors.pgpPassphraseMismatch'));
    const actions = useMessageSecurityActions();

    const unlocking = actions.unlockPgp('thread-4');
    await answerPrompt(actions, 'wrong');
    await unlocking;

    expect(harness.loadByThread).not.toHaveBeenCalled();
    expect(actions.pgpStatus.value).toBe('mail.pgp.passphraseWrong');
    expect(actions.pgpBusy.value).toBe(false);
  });

  it('stops cleanly when the dialog is cancelled', async () => {
    harness.unlockPgp.mockRejectedValueOnce(backendError('backend.errors.pgpStoredPassphraseRejected'));
    const actions = useMessageSecurityActions();

    const unlocking = actions.unlockPgp('thread-5');
    await answerPrompt(actions, null);
    await unlocking;

    expect(harness.unlockPgp).toHaveBeenCalledOnce();
    expect(harness.loadByThread).not.toHaveBeenCalled();
    expect(actions.pgpStatus.value).toBe('');
    expect(actions.pgpBusy.value).toBe(false);
  });

  it('passes undefined for a whitespace-only S/MIME passphrase', async () => {
    harness.unlockSmime.mockRejectedValueOnce(new Error('locked'));
    const actions = useMessageSecurityActions();

    const unlocking = actions.unlockSmime('thread-6');
    await answerPrompt(actions, '   ');
    await unlocking;

    expect(harness.unlockSmime).toHaveBeenNthCalledWith(2, { passphrase: undefined });
    expect(harness.loadByThread).toHaveBeenCalledWith('thread-6');
    expect(actions.smimeStatus.value).toBe('mail.smime.unlocked');
    expect(actions.smimeBusy.value).toBe(false);
  });

  it('reports S/MIME failure and always clears busy state', async () => {
    harness.unlockSmime
      .mockRejectedValueOnce(new Error('locked'))
      .mockRejectedValueOnce(new Error('wrong passphrase'));
    const actions = useMessageSecurityActions();

    const unlocking = actions.unlockSmime('thread-7');
    await answerPrompt(actions, 'wrong');
    await unlocking;

    expect(harness.loadByThread).not.toHaveBeenCalled();
    expect(actions.smimeStatus.value).toBe('mail.smime.unlockFailed');
    expect(actions.smimeBusy.value).toBe(false);
  });

  it('guards a second unlock while the first one is busy', async () => {
    let resolveUnlock!: (value: unknown) => void;
    harness.unlockPgp.mockReturnValueOnce(new Promise((resolve) => { resolveUnlock = resolve; }));
    const actions = useMessageSecurityActions();

    const first = actions.unlockPgp('thread-8');
    await Promise.resolve();
    const second = actions.unlockPgp('thread-9');
    resolveUnlock({ unlocked: 1, key_ids: [] });
    await Promise.all([first, second]);

    expect(harness.unlockPgp).toHaveBeenCalledOnce();
    expect(harness.loadByThread).toHaveBeenCalledWith('thread-8');
    expect(harness.loadByThread).not.toHaveBeenCalledWith('thread-9');
  });

  it('releases an unlock that is still waiting on a dialog when it is reset', async () => {
    harness.unlockPgp.mockRejectedValueOnce(backendError('backend.errors.pgpStoredPassphraseRejected'));
    const actions = useMessageSecurityActions();

    const unlocking = actions.unlockPgp('thread-10');
    await Promise.resolve();
    await Promise.resolve();
    actions.reset();
    await unlocking;

    expect(actions.passphraseRequest.value).toBeNull();
    expect(actions.pgpBusy.value).toBe(false);
  });
});
