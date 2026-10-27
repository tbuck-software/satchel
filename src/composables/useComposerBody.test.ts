// @vitest-environment jsdom
import { computed, ref } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useComposerBody } from './useComposerBody';
import { useComposerSignature } from './useComposerSignature';
import { splitComposerHtmlHistory, splitComposerTextHistory } from '@/utils/composerHistory';
import { composerDraftInput, type ComposerDraftState } from '@/utils/composerDraftState';
import type { Account } from '@/types/mail';
import type { BackendResolvedSignature } from '@/types/composer';

const reply = 'On Monday, Ada wrote:';
const forward = '---------- Forwarded message ----------';

beforeEach(() => setActivePinia(createPinia()));

describe('composer body sections', () => {
  it.each([reply, forward])('keeps writing separate from %s through edits and draft reload', (intro) => {
    const body = ref(`My answer\n\n${intro}\n> Older mail`);
    const bodyHtml = ref<string | null>(`<p>My answer</p><blockquote><p>${intro}</p><p>Older mail</p></blockquote>`);
    const sections = useComposerBody(body, bodyHtml);
    expect(sections.writingText.value).toBe('My answer');
    expect(sections.writingHtml.value).toBe('<p>My answer</p>');
    expect(sections.historyHtml.value).toContain('Older mail');

    sections.writingText.value = 'Edited answer';
    sections.writingHtml.value = '<p><strong>Edited answer</strong></p>';
    sections.historyText.value = `${intro}\nEdited older mail`;
    sections.historyHtml.value = `<blockquote><p>${intro}</p><p>Edited older mail</p></blockquote>`;
    const saved = composerDraftInput({
      accountId: 'account', fromAddress: 'me@example.test', fromName: null,
      threadId: 'thread', replyTo: null, sendOptions: {}, to: ['ada@example.test'], cc: [], bcc: [],
      subject: 'Reply', body: body.value, bodyHtml: bodyHtml.value, plainText: false,
      scheduledFor: null, attachments: [], signatureId: 'sig', signatureVariant: 'long',
      signatureOverrideText: 'Kind regards', signatureOverrideHtml: '<p>Kind regards</p>',
    } satisfies ComposerDraftState);
    const reopened = useComposerBody(ref(saved.body!), ref(saved.bodyHtml ?? null));
    expect(reopened.writingHtml.value).toBe('<p><strong>Edited answer</strong></p>');
    expect(reopened.historyHtml.value).toContain('Edited older mail');
    expect(saved.signatureOverrideText).toBe('Kind regards');
    expect(saved.bodyHtml).not.toContain('Kind regards');
    reopened.removeHistory();
    expect(reopened.historyHtml.value).toBe('');
    expect(reopened.writingText.value).toBe('Edited answer');
  });

  it('restores structural history from plain text and keeps normal blockquotes in the writing', () => {
    const sections = useComposerBody(ref(`My answer\n\n${reply}\n> Older mail`), ref(null));
    expect(sections.writingHtml.value).toBe('<p>My answer</p>');
    expect(sections.historyHtml.value).toContain('<blockquote>');
    const normalQuote = '<p>My answer</p><blockquote><p>A quotation I wrote</p></blockquote>';
    expect(splitComposerHtmlHistory(normalQuote)).toEqual({ writing: normalQuote, history: '' });
    expect(splitComposerTextHistory('New message')).toEqual({ writing: 'New message', history: '' });
  });

  it('replaces signature selection and identity without modifying writing or history', () => {
    const body = ref(`My answer\n\n${reply}\n> Older mail`);
    const bodyHtml = ref<string | null>(`<p>My answer</p><blockquote><p>${reply}</p><p>Older mail</p></blockquote>`);
    const sections = useComposerBody(body, bodyHtml);
    const account = ref<Account | undefined>({ id: 'account', email: 'me@example.test', displayName: 'Me', provider: 'imap', kind: 'email', color: 'blue' });
    const fromAddress = ref('me@example.test');
    const resolvedSignature = ref<BackendResolvedSignature | null>({ signature_id: 'sig', variant: 'long', body_text: 'Kind regards', body_html: '<p>Kind regards</p>', attachments: [] });
    const signature = useComposerSignature({
      activeAccount: computed(() => account.value), fromAddress: computed(() => fromAddress.value),
      contextThreadId: ref('thread'), supportsBackend: true,
      transport: { resolvedSignature, signaturePreviewLoading: ref(false), refreshSignaturePreview: vi.fn() },
    });
    signature.edit({ text: 'Edited regards', html: null });
    expect(signature.signatureDisplayHtml.value).toBe('Edited regards');
    signature.select('other-signature');
    resolvedSignature.value = { signature_id: 'other-signature', variant: 'short', body_text: 'Thanks', body_html: '<p>Thanks</p>', attachments: [] };
    expect(signature.signatureDisplayHtml.value).toBe('<p>Thanks</p>');
    fromAddress.value = 'alias@example.test';
    signature.syncSelection();
    expect(signature.signatureOverrideActive.value).toBe(false);
    expect(sections.writingText.value).toBe('My answer');
    expect(sections.historyText.value).toBe(`${reply}\n> Older mail`);
    expect(bodyHtml.value).not.toContain('regards');
    expect(bodyHtml.value).not.toContain('Thanks');
  });
});
