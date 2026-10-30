// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';

const composeWindow = vi.hoisted(() => ({
  open: vi.fn(),
  openNative: vi.fn(),
  supportsNative: vi.fn(() => true),
}));

const composeView = vi.hoisted(() => ({
  persistDraft: vi.fn(async () => 'draft-1' as string | null),
  shouldConfirmClose: vi.fn(() => false),
  closeWithoutSaving: vi.fn(async () => undefined),
  currentDraftId: vi.fn(() => null as string | null),
}));

const confirm = vi.hoisted(() => ({
  decide: vi.fn(async (): Promise<'confirmed' | 'declined' | 'preempted'> => 'confirmed'),
}));

const toast = vi.hoisted(() => ({
  error: vi.fn(),
  info: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
}));

vi.mock('@/composables/useToast', () => ({ useToast: () => toast }));
vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => confirm }));

vi.mock('@/composables/useComposeWindow', () => ({
  useComposeWindow: () => composeWindow,
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn(), resolve: vi.fn(() => ({ href: '/compose' })) }),
}));

vi.mock('@/views/ComposeView.vue', () => ({
  default: {
    setup(_props: unknown, { expose }: { expose: (handle: unknown) => void }) {
      expose({
        persistDraft: composeView.persistDraft,
        shouldConfirmClose: composeView.shouldConfirmClose,
        closeWithoutSaving: composeView.closeWithoutSaving,
        currentDraftId: composeView.currentDraftId,
      });
      return () => undefined;
    },
  },
}));

import ComposerHost from './ComposerHost.vue';
import { useComposerStore } from '@/stores/composer';

async function mountOpenComposer() {
  const wrapper = mount(ComposerHost, { attachTo: document.body });
  useComposerStore().openNew();
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  composeWindow.supportsNative.mockReturnValue(true);
  composeView.persistDraft.mockResolvedValue('draft-1');
  composeView.shouldConfirmClose.mockReturnValue(false);
  composeView.currentDraftId.mockReturnValue(null);
  confirm.decide.mockResolvedValue('confirmed');
});

describe('composer popout', () => {
  it('opens an empty composer without creating a draft', async () => {
    composeView.persistDraft.mockResolvedValue(null);
    const wrapper = await mountOpenComposer();

    await wrapper.get('button[title="Open in its own window"]').trigger('click');
    await flushPromises();

    expect(composeWindow.open).toHaveBeenCalledWith(null);
    expect(useComposerStore().isOpen).toBe(false);
  });

  it('keeps a nonempty composer open when its draft cannot be saved', async () => {
    composeView.shouldConfirmClose.mockReturnValue(true);
    composeView.persistDraft.mockResolvedValue(null);
    const wrapper = await mountOpenComposer();

    await wrapper.get('button[title="Open in its own window"]').trigger('click');
    await flushPromises();

    expect(composeWindow.open).not.toHaveBeenCalled();
    expect(useComposerStore().isOpen).toBe(true);
    expect(wrapper.get('.composer-error').text()).toBe('The draft could not be saved. The composer stays open.');
  });

  it('keeps the composer open and reports the failure when the popout fails', async () => {
    composeWindow.open.mockRejectedValue(new Error('no window'));
    const wrapper = await mountOpenComposer();

    await wrapper.get('button[title="Open in its own window"]').trigger('click');
    await flushPromises();

    expect(useComposerStore().isOpen).toBe(true);
    expect(wrapper.get('.composer-error').text()).toContain('no window');
  });

  it('closes the composer once the popout succeeded', async () => {
    composeWindow.open.mockResolvedValue(undefined);
    const wrapper = await mountOpenComposer();

    await wrapper.get('button[title="Open in its own window"]').trigger('click');
    await flushPromises();

    expect(composeWindow.open).toHaveBeenCalledWith('draft-1');
    // The popout owns the draft now: no closing save may race its first edit.
    expect(composeView.closeWithoutSaving).toHaveBeenCalled();
    expect(composeWindow.open.mock.invocationCallOrder[0]!)
      .toBeLessThan(composeView.closeWithoutSaving.mock.invocationCallOrder[0]!);
    expect(useComposerStore().isOpen).toBe(false);
    expect(wrapper.find('.composer-error').exists()).toBe(false);
  });
});

describe('a second composer request while one is open', () => {
  it('replaces an untouched composer without asking', async () => {
    await mountOpenComposer();

    await useComposerStore().openReply('thread-1');
    await flushPromises();

    expect(confirm.decide).not.toHaveBeenCalled();
    expect(composeView.closeWithoutSaving).toHaveBeenCalled();
    expect(useComposerStore().request?.mode).toBe('reply');
  });

  it('asks first and keeps the composer when the user declines', async () => {
    composeView.shouldConfirmClose.mockReturnValue(true);
    confirm.decide.mockResolvedValue('declined');
    await mountOpenComposer();

    await expect(useComposerStore().openWithOutcome({ mode: 'reply', threadId: 'thread-1' })).resolves.toBe('kept');

    expect(confirm.decide).toHaveBeenCalledTimes(1);
    expect(composeView.persistDraft).not.toHaveBeenCalled();
    expect(useComposerStore().request?.mode).toBe('new');
  });

  it('saves before replacing and keeps the composer when that save fails', async () => {
    composeView.shouldConfirmClose.mockReturnValue(true);
    composeView.persistDraft.mockResolvedValue(null);
    const wrapper = await mountOpenComposer();

    // Not a refusal: the user agreed, and the new request's content is not on disk.
    await expect(useComposerStore().openWithOutcome({ mode: 'new', to: ['mara@example.com'] })).resolves.toBe('unsaved');
    await flushPromises();

    expect(useComposerStore().request?.to).toEqual([]);
    expect(wrapper.get('.composer-error').text()).toBe('The draft could not be saved. The composer stays open.');

    composeView.persistDraft.mockResolvedValue('draft-1');
    await expect(useComposerStore().openEmail(['mara@example.com'])).resolves.toBe(true);
    expect(useComposerStore().request?.to).toEqual(['mara@example.com']);
  });

  it('reports a question another dialog took away as interrupted, not as kept', async () => {
    composeView.shouldConfirmClose.mockReturnValue(true);
    confirm.decide.mockResolvedValue('preempted');
    await mountOpenComposer();

    await expect(useComposerStore().openWithOutcome({ mode: 'reply', threadId: 'thread-1' })).resolves.toBe('interrupted');

    expect(composeView.persistDraft).not.toHaveBeenCalled();
    expect(useComposerStore().request?.mode).toBe('new');
  });

  it('asks before replacing a notification reply the form calls untouched, and saves it', async () => {
    const wrapper = mount(ComposerHost, { attachTo: document.body });
    await useComposerStore().open({ mode: 'reply', threadId: 'thread-1', body: 'typed in a notification', unsavedBody: true });
    await flushPromises();

    await expect(useComposerStore().openWithOutcome({ mode: 'forward', threadId: 'thread-2' })).resolves.toBe('opened');

    expect(confirm.decide).toHaveBeenCalledTimes(1);
    expect(composeView.persistDraft).toHaveBeenCalledTimes(1);
    expect(composeView.closeWithoutSaving).not.toHaveBeenCalled();
    expect(useComposerStore().request?.mode).toBe('forward');
    wrapper.unmount();
  });

  it('retries saving a notification reply instead of dropping it', async () => {
    composeView.persistDraft.mockResolvedValue(null);
    const wrapper = mount(ComposerHost, { attachTo: document.body });
    await useComposerStore().open({ mode: 'reply', threadId: 'thread-1', body: 'typed in a notification', unsavedBody: true });
    await flushPromises();

    await expect(useComposerStore().openWithOutcome({ mode: 'forward', threadId: 'thread-2' })).resolves.toBe('unsaved');
    composeView.persistDraft.mockResolvedValue('draft-4');
    await toast.error.mock.calls[0]?.[1]?.action?.handler();
    await flushPromises();

    expect(composeView.persistDraft).toHaveBeenCalledTimes(2);
    expect(composeView.closeWithoutSaving).not.toHaveBeenCalled();
    expect(useComposerStore().isOpen).toBe(false);
    wrapper.unmount();
  });

  it('replaces a notification reply without asking once its draft is on disk', async () => {
    composeView.currentDraftId.mockReturnValue('draft-3');
    const wrapper = mount(ComposerHost, { attachTo: document.body });
    await useComposerStore().open({ mode: 'reply', threadId: 'thread-1', body: 'typed in a notification', unsavedBody: true });
    await flushPromises();

    await expect(useComposerStore().openWithOutcome({ mode: 'forward', threadId: 'thread-2' })).resolves.toBe('opened');

    expect(confirm.decide).not.toHaveBeenCalled();
    expect(composeView.closeWithoutSaving).toHaveBeenCalled();
    wrapper.unmount();
  });

  it('keeps the composer when the same draft is opened again', async () => {
    composeView.currentDraftId.mockReturnValue('draft-7');
    await mountOpenComposer();
    const instance = useComposerStore().instance;

    await expect(useComposerStore().openDraft('draft-7')).resolves.toBe(false);

    expect(useComposerStore().instance).toBe(instance);
  });
});

/**
 * A draft that cannot be written is the one case where the window must stay
 * open — and it was also the case with the quietest sign: one thin line under
 * the title bar, which reads as a broken close button.
 */
describe('a close that cannot save the draft', () => {
  it('keeps the window, announces the failure and offers the retry', async () => {
    composeView.shouldConfirmClose.mockReturnValue(true);
    composeView.persistDraft.mockResolvedValue(null);
    const wrapper = await mountOpenComposer();

    await wrapper.get('button[title="Close composer"]').trigger('click');
    await flushPromises();

    expect(useComposerStore().isOpen).toBe(true);
    expect(wrapper.get('.composer-error').attributes('role')).toBe('alert');
    expect(toast.error).toHaveBeenCalledTimes(1);
    expect(toast.error.mock.calls[0]?.[1]?.action?.label).toBe('Try again');

    // The retry runs the same close, so a save that now works closes the window.
    composeView.persistDraft.mockResolvedValue('draft-2');
    await toast.error.mock.calls[0]?.[1]?.action?.handler();
    await flushPromises();
    expect(useComposerStore().isOpen).toBe(false);
  });
});

describe('composer placement', () => {
  it('places the first composer when App mounts the host after opening', async () => {
    await useComposerStore().openNew();
    const wrapper = mount(ComposerHost, { attachTo: document.body });
    await flushPromises();
    const section = wrapper.get('.composer-window');
    expect(section.attributes('style')).toContain(`translate3d(${window.innerWidth - 620 - 18}px, ${window.innerHeight - 540 - 18}px, 0)`);
    wrapper.unmount();
  });

  it('redocks an untouched composer after the window changes size', async () => {
    const wrapper = await mountOpenComposer();
    const oldWidth = window.innerWidth;
    Object.defineProperty(window, 'innerWidth', { value: 1200, configurable: true });
    window.dispatchEvent(new Event('resize'));
    await flushPromises();
    expect(wrapper.get('.composer-window').attributes('style')).toContain('translate3d(562px,');
    Object.defineProperty(window, 'innerWidth', { value: oldWidth, configurable: true });
    wrapper.unmount();
  });
});
