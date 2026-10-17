import { computed, ref } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useSignaturesStore } from '@/stores/signatures';
import type { Account } from '@/types/mail';
import type { BackendResolvedSignature, BackendSignature } from '@/types/composer';
import {
  useComposerSignature,
  type ComposerSignatureTransport,
} from './useComposerSignature';

const account: Account = {
  id: 'account-1',
  displayName: 'Ada',
  email: 'ada@example.com',
  provider: 'imap',
  kind: 'email',
  color: 'blue',
};

function signature(overrides: Partial<BackendSignature> = {}): BackendSignature {
  return {
    id: 'signature-1',
    account_id: account.id,
    name: 'Default',
    body_text: 'Regards',
    body_html: '<p>Regards</p>',
    short_body_text: 'Thanks',
    short_body_html: '<p>Thanks</p>',
    assets: [],
    is_default: 1,
    created_at: '',
    updated_at: '',
    ...overrides,
  };
}

function createHarness(options: { supportsBackend?: boolean } = {}) {
  const activeAccount = ref<Account | undefined>(account);
  const fromAddress = ref(account.email);
  const contextThreadId = ref<string | null>('thread-1');
  const resolvedSignature = ref<BackendResolvedSignature | null>(null);
  const signaturePreviewLoading = ref(false);
  const refreshSignaturePreview = vi.fn<(request: unknown) => Promise<void>>()
    .mockResolvedValue(undefined);
  const transport: ComposerSignatureTransport = {
    resolvedSignature,
    signaturePreviewLoading,
    refreshSignaturePreview,
  };
  const policy = useComposerSignature({
    activeAccount: computed(() => activeAccount.value),
    fromAddress: computed(() => fromAddress.value),
    contextThreadId,
    supportsBackend: options.supportsBackend ?? true,
    transport,
  });
  return {
    activeAccount,
    fromAddress,
    contextThreadId,
    resolvedSignature,
    refreshSignaturePreview,
    policy,
  };
}

describe('useComposerSignature', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('selects the assigned/default signature and clears per-message edits when identity changes', () => {
    const store = useSignaturesStore();
    store.items = [signature()];
    const { activeAccount, policy } = createHarness();

    policy.syncSelection();
    expect(policy.availableSignatures.value).toEqual(store.items);
    expect(policy.selectedSignatureId.value).toBe('signature-1');
    expect(policy.signatureVariant.value).toBe('auto');

    policy.edit({ html: '<p>Custom</p>', text: 'Custom' });
    activeAccount.value = undefined;
    policy.syncSelection();

    expect(policy.selectedSignatureId.value).toBeNull();
    expect(policy.signatureVariant.value).toBe('none');
    expect(policy.signatureOverrideActive.value).toBe(false);
  });

  it('builds a complete preview request and clears it when selection is disabled', async () => {
    const store = useSignaturesStore();
    store.items = [signature()];
    const { policy, refreshSignaturePreview } = createHarness();
    policy.syncSelection();
    policy.signatureVariant.value = 'short';

    await policy.refreshPreview();
    expect(refreshSignaturePreview).toHaveBeenLastCalledWith({
      accountId: 'account-1',
      identityEmail: 'ada@example.com',
      threadId: 'thread-1',
      signatureId: 'signature-1',
      preference: 'short',
    });

    policy.select(null);
    await policy.refreshPreview();
    expect(refreshSignaturePreview).toHaveBeenLastCalledWith(null);
  });

  it('hydrates CID images for editing and serializes them back while pinning auto', () => {
    const { policy, resolvedSignature } = createHarness();
    resolvedSignature.value = {
      signature_id: 'signature-1',
      variant: 'long',
      body_text: 'Regards',
      body_html: '<p>Regards</p><img src="cid:logo">',
      attachments: [{
        filename: 'logo.png',
        mime_type: 'image/png',
        content_base64: 'aW1hZ2U=',
        content_id: 'logo',
        inline: true,
      }],
    };

    expect(policy.signatureDisplayHtml.value).toContain('data:image/png;base64,aW1hZ2U=');
    policy.edit({
      html: '<p>Custom</p><img src="data:image/png;base64,aW1hZ2U=">',
      text: 'Custom',
    });

    expect(policy.signatureVariant.value).toBe('long');
    expect(policy.signatureOverrideText.value).toBe('Custom');
    expect(policy.signatureOverrideHtml.value).toContain('cid:logo');
    expect(policy.signatureDisplayHtml.value).toContain('data:image/png;base64,aW1hZ2U=');
  });

  it('escapes text-only previews and clears overrides when the variant changes', () => {
    const { policy, resolvedSignature } = createHarness();
    resolvedSignature.value = {
      signature_id: 'signature-1',
      variant: 'short',
      body_text: '<Ada & Co>\nThanks',
      body_html: null,
      attachments: [],
    };

    expect(policy.signatureDisplayHtml.value).toBe('&lt;Ada &amp; Co&gt;<br>Thanks');
    expect(policy.signatureDisplayText.value).toBe('<Ada & Co>\nThanks');

    policy.edit({ html: null, text: 'Custom' });
    policy.selectVariant('short');
    expect(policy.signatureOverrideActive.value).toBe(false);
    expect(policy.signatureDisplayText.value).toBe('<Ada & Co>\nThanks');
  });
});
