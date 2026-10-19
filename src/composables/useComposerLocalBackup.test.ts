// @vitest-environment jsdom
import { defineComponent, h, nextTick, ref } from 'vue';
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { composerBackupPersistence, type ComposerBackupContent } from '@/persistence/composerBackup';
import { useComposerLocalBackup } from './useComposerLocalBackup';

beforeEach(() => { vi.useFakeTimers(); localStorage.clear(); });
afterEach(() => { vi.useRealTimers(); localStorage.clear(); });

function editor() {
  const content = ref<ComposerBackupContent>({
    accountId: 'account', fromAddress: 'me@example.com', fromName: null,
    threadId: null, replyTo: null, sendOptions: {},
    to: [], cc: [], bcc: [], subject: '', body: '', bodyHtml: null,
    plainText: true, scheduledFor: null, signatureId: null, signatureVariant: 'none',
    signatureOverrideText: null, signatureOverrideHtml: null,
  });
  const read = vi.fn(() => content.value);
  const wrapper = mount(defineComponent({ setup() {
    useComposerLocalBackup({ enabled: ref(true), draftId: ref(null), content: read });
    return () => h('div');
  } }));
  return { wrapper, content, read };
}

it('reads the composer once typing pauses, not on every keystroke', async () => {
  const { wrapper, content, read } = editor();
  try {
    read.mockClear();
    for (const body of ['H', 'He', 'Hel', 'Hell', 'Hello']) {
      content.value.body = body;
      await nextTick();
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(read).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(300);
    expect(composerBackupPersistence.findOrphan({}, Date.now() + 20_000)?.content.body).toBe('Hello');
  } finally { wrapper.unmount(); }
});

it('writes the latest keystroke when the page goes away', async () => {
  const { wrapper, content } = editor();
  try {
    content.value.body = 'Last words';
    await nextTick();
    window.dispatchEvent(new Event('pagehide'));
    expect(composerBackupPersistence.findOrphan({}, Date.now() + 20_000)?.content.body).toBe('Last words');
  } finally { wrapper.unmount(); }
});

it.each(['cc', 'bcc'] as const)('recovers an edit containing only %s', async field => {
  const { wrapper, content } = editor();
  try {
    content.value[field] = ['copy@example.com'];
    await nextTick();
    await vi.advanceTimersByTimeAsync(350);
    expect(composerBackupPersistence.findOrphan({}, Date.now() + 20_000)?.content[field])
      .toEqual(['copy@example.com']);
  } finally { wrapper.unmount(); }
});

it('retains encryption, reply policy and scheduling across recovery', async () => {
  const { wrapper, content } = editor();
  try {
    content.value.body = 'Private message';
    await nextTick();
    await vi.advanceTimersByTimeAsync(350);
    content.value.sendOptions = { pgp_encrypt_to_recipients: true, priority: 'high' };
    content.value.replyTo = 'reply@example.com';
    content.value.scheduledFor = '2026-11-01T12:00:00Z';
    await nextTick();
    await vi.advanceTimersByTimeAsync(350);
    expect(composerBackupPersistence.findOrphan({}, Date.now() + 20_000)?.content).toMatchObject({
      body: 'Private message', replyTo: 'reply@example.com', scheduledFor: '2026-11-01T12:00:00Z',
      sendOptions: { pgp_encrypt_to_recipients: true, priority: 'high' },
    });
  } finally { wrapper.unmount(); }
});
