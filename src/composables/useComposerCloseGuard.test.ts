// @vitest-environment jsdom

import { defineComponent, h, ref } from 'vue';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ComposerDraftState } from '@/utils/composerDraftState';

const nativeWindow = vi.hoisted(() => ({
  getCurrent: vi.fn(),
}));

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: nativeWindow.getCurrent,
}));

import { useComposerCloseGuard } from './useComposerCloseGuard';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function draftState(overrides: Partial<ComposerDraftState> = {}): ComposerDraftState {
  return {
    accountId: 'account-1',
    fromAddress: 'sender@example.com',
    to: [],
    cc: [],
    bcc: [],
    subject: '',
    body: '',
    bodyHtml: null,
    plainText: false,
    scheduledFor: null,
    signatureId: null,
    signatureVariant: 'auto',
    signatureOverrideText: null,
    signatureOverrideHtml: null,
    attachments: [],
    ...overrides,
  };
}

type CloseHandler = (event: { preventDefault: () => void }) => void | Promise<void>;

interface HarnessOptions {
  embedded?: boolean;
  standalone?: boolean;
  supportsBackend?: boolean;
  initialized?: boolean;
  state?: ComposerDraftState;
  persistDraft?: (options: { silent?: boolean }) => Promise<string | null>;
  closeDraftWithoutSaving?: () => Promise<void>;
  closeRegistration?: Promise<() => void>;
}

function createHarness(options: HarnessOptions = {}) {
  const initialized = ref(options.initialized ?? true);
  let embedded = options.embedded ?? false;
  let standalone = options.standalone ?? true;
  let state = options.state ?? draftState();
  let closeHandler: CloseHandler | null = null;
  const unlisten = vi.fn();
  const destroy = vi.fn(async () => undefined);
  const onCloseRequested = vi.fn(async (handler: CloseHandler) => {
    closeHandler = handler;
    return options.closeRegistration ?? unlisten;
  });
  nativeWindow.getCurrent.mockReturnValue({ onCloseRequested, destroy });
  const persistDraft = vi.fn(options.persistDraft ?? (async () => 'saved-draft'));
  const finishDraftPersistence = vi.fn();
  const closeDraftWithoutSaving = vi.fn(
    options.closeDraftWithoutSaving ?? (async () => undefined),
  );
  let closeGuard!: ReturnType<typeof useComposerCloseGuard>;
  const wrapper = mount(defineComponent({
    setup() {
      closeGuard = useComposerCloseGuard({
        embedded: () => embedded,
        standalone: () => standalone,
        supportsBackend: options.supportsBackend ?? true,
        initialized,
        currentState: () => state,
        persistDraft,
        finishDraftPersistence,
        closeDraftWithoutSaving,
      });
      return () => h('div');
    },
  }));

  return {
    wrapper,
    initialized,
    closeGuard,
    persistDraft,
    finishDraftPersistence,
    closeDraftWithoutSaving,
    onCloseRequested,
    destroy,
    unlisten,
    setEmbedded(value: boolean) {
      embedded = value;
    },
    setStandalone(value: boolean) {
      standalone = value;
    },
    setState(value: ComposerDraftState) {
      state = value;
    },
    requestClose(event = { preventDefault: vi.fn() }) {
      if (!closeHandler) throw new Error('close guard is not installed');
      closeHandler(event);
      return event;
    },
  };
}

const wrappers: VueWrapper[] = [];

function harness(options: HarnessOptions = {}) {
  const result = createHarness(options);
  wrappers.push(result.wrapper);
  return result;
}

describe('useComposerCloseGuard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    while (wrappers.length > 0) wrappers.pop()?.unmount();
    vi.restoreAllMocks();
  });

  it('compares initialized, meaningful content with the captured baseline', () => {
    const initial = draftState({ body: 'Loaded draft' });
    const { closeGuard, initialized, setState } = harness({
      initialized: false,
      state: initial,
    });

    closeGuard.captureBaseline();
    setState(draftState({ body: 'Changed draft' }));
    expect(closeGuard.hasMeaningfulContent()).toBe(true);
    expect(closeGuard.shouldConfirmClose()).toBe(false);

    initialized.value = true;
    expect(closeGuard.shouldConfirmClose()).toBe(true);

    setState(initial);
    expect(closeGuard.shouldConfirmClose()).toBe(false);

    setState(draftState({ body: '   ' }));
    expect(closeGuard.hasMeaningfulContent()).toBe(false);
    expect(closeGuard.shouldConfirmClose()).toBe(false);
  });

  it.each([
    { embedded: true, standalone: true, supportsBackend: true },
    { embedded: false, standalone: false, supportsBackend: true },
    { embedded: false, standalone: true, supportsBackend: false },
  ])('does not install outside a native standalone composer', async (guardOptions) => {
    const { closeGuard, onCloseRequested } = harness(guardOptions);

    await closeGuard.installStandaloneCloseGuard();

    expect(nativeWindow.getCurrent).not.toHaveBeenCalled();
    expect(onCloseRequested).not.toHaveBeenCalled();
  });

  it('reads the standalone guards when installation runs', async () => {
    const { closeGuard, onCloseRequested, setEmbedded, setStandalone } = harness({
      embedded: true,
      standalone: false,
    });
    setEmbedded(false);
    setStandalone(true);

    await closeGuard.installStandaloneCloseGuard();

    expect(nativeWindow.getCurrent).toHaveBeenCalledTimes(1);
    expect(onCloseRequested).toHaveBeenCalledTimes(1);
  });

  it('prevents native close synchronously, then closes an unchanged composer', async () => {
    const pendingClose = deferred<void>();
    const { closeGuard, requestClose, closeDraftWithoutSaving, persistDraft, destroy } = harness({
      closeDraftWithoutSaving: async () => pendingClose.promise,
    });
    closeGuard.captureBaseline();
    await closeGuard.installStandaloneCloseGuard();

    const event = requestClose();

    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(closeDraftWithoutSaving).toHaveBeenCalledTimes(1);
    expect(persistDraft).not.toHaveBeenCalled();
    expect(destroy).not.toHaveBeenCalled();

    pendingClose.resolve();
    await flushPromises();
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  it('ignores duplicate close work while a changed draft save is pending', async () => {
    const pendingSave = deferred<string | null>();
    const { closeGuard, setState, requestClose, persistDraft, destroy } = harness({
      persistDraft: async () => pendingSave.promise,
    });
    closeGuard.captureBaseline();
    setState(draftState({ body: 'Changed' }));
    await closeGuard.installStandaloneCloseGuard();

    const firstEvent = requestClose();
    const secondEvent = requestClose();

    expect(firstEvent.preventDefault).toHaveBeenCalledTimes(1);
    expect(secondEvent.preventDefault).toHaveBeenCalledTimes(1);
    expect(persistDraft).toHaveBeenCalledTimes(1);
    expect(persistDraft).toHaveBeenCalledWith({ silent: false });
    expect(destroy).not.toHaveBeenCalled();

    pendingSave.resolve('saved-draft');
    await flushPromises();
  });

  it('keeps the window open after a failed save and allows another close attempt', async () => {
    const persistDraft = vi.fn()
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce('saved-draft');
    const {
      closeGuard,
      setState,
      requestClose,
      finishDraftPersistence,
      destroy,
    } = harness({ persistDraft });
    closeGuard.captureBaseline();
    setState(draftState({ subject: 'Changed' }));
    await closeGuard.installStandaloneCloseGuard();

    requestClose();
    await flushPromises();
    expect(destroy).not.toHaveBeenCalled();
    expect(finishDraftPersistence).not.toHaveBeenCalled();

    requestClose();
    await flushPromises();
    expect(persistDraft).toHaveBeenCalledTimes(2);
    expect(finishDraftPersistence).toHaveBeenCalledTimes(1);
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(finishDraftPersistence.mock.invocationCallOrder[0]).toBeLessThan(
      destroy.mock.invocationCallOrder[0]!,
    );
  });

  it('forwards explicit close-without-save requests', async () => {
    const { closeGuard, closeDraftWithoutSaving } = harness();

    await closeGuard.closeWithoutSaving();

    expect(closeDraftWithoutSaving).toHaveBeenCalledTimes(1);
  });

  it('removes the native listener when its owner unmounts', async () => {
    const { closeGuard, wrapper, unlisten } = harness();
    await closeGuard.installStandaloneCloseGuard();

    wrapper.unmount();

    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it('removes a listener whose registration resolves after unmount', async () => {
    const registration = deferred<() => void>();
    const result = harness({ closeRegistration: registration.promise });
    const installing = result.closeGuard.installStandaloneCloseGuard();

    result.wrapper.unmount();
    wrappers.splice(wrappers.indexOf(result.wrapper), 1);
    registration.resolve(result.unlisten);
    await installing;

    expect(result.unlisten).toHaveBeenCalledTimes(1);
    const event = result.requestClose();
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(result.persistDraft).not.toHaveBeenCalled();
  });
});
