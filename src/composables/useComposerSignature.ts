import { computed, ref, type ComputedRef, type Ref } from 'vue';
import { useSignaturesStore } from '@/stores/signatures';
import type { Account } from '@/types/mail';
import type { BackendResolvedSignature } from '@/types/composer';
import type { SignaturePreviewRequest } from '@/composables/useComposerTransport';
import { hydrateCidReferences, serializeCidReferences } from '@/utils/composerInlineCids';

export type ComposerSignatureVariant = 'auto' | 'long' | 'short' | 'none';

export interface ComposerSignatureTransport {
  resolvedSignature: Ref<BackendResolvedSignature | null>;
  signaturePreviewLoading: Ref<boolean>;
  refreshSignaturePreview: (request: SignaturePreviewRequest | null) => Promise<void>;
}

export interface ComposerSignatureOptions {
  activeAccount: ComputedRef<Account | undefined>;
  fromAddress: ComputedRef<string>;
  contextThreadId: Ref<string | null>;
  supportsBackend: boolean;
  transport: ComposerSignatureTransport;
}

/** Owns signature selection, preview rendering, and per-message overrides. */
export function useComposerSignature(options: ComposerSignatureOptions) {
  const signaturesStore = useSignaturesStore();
  const selectedSignatureId = ref<string | null>(null);
  const signatureVariant = ref<ComposerSignatureVariant>('auto');
  const signatureOverrideText = ref<string | null>(null);
  const signatureOverrideHtml = ref<string | null>(null);

  const availableSignatures = computed(() =>
    options.activeAccount.value
      ? signaturesStore.byAccount.get(options.activeAccount.value.id) ?? []
      : [],
  );

  const signaturePreviewHtml = computed(() => {
    const html = options.transport.resolvedSignature.value?.body_html ?? null;
    return html
      ? hydrateCidReferences(html, options.transport.resolvedSignature.value?.attachments ?? [])
      : null;
  });

  const signatureDisplayHtml = computed(() => {
    if (signatureOverrideHtml.value !== null) {
      return hydrateCidReferences(
        signatureOverrideHtml.value,
        options.transport.resolvedSignature.value?.attachments ?? [],
      );
    }
    if (signatureOverrideText.value === null && signaturePreviewHtml.value) return signaturePreviewHtml.value;
    const text = signatureOverrideText.value ?? options.transport.resolvedSignature.value?.body_text ?? '';
    if (!text.trim()) return null;
    return text
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/\n/g, '<br>');
  });

  const signatureDisplayText = computed(
    () => signatureOverrideText.value ?? options.transport.resolvedSignature.value?.body_text ?? '',
  );
  const signatureOverrideActive = computed(
    () => signatureOverrideText.value !== null || signatureOverrideHtml.value !== null,
  );

  function clearOverride() {
    signatureOverrideText.value = null;
    signatureOverrideHtml.value = null;
  }

  function syncSelection() {
    clearOverride();
    const account = options.activeAccount.value;
    if (!account || !options.fromAddress.value) {
      selectedSignatureId.value = null;
      signatureVariant.value = 'none';
      return;
    }
    selectedSignatureId.value =
      signaturesStore.selectedFor(account.id, options.fromAddress.value)?.id ?? null;
    signatureVariant.value = selectedSignatureId.value ? 'auto' : 'none';
  }

  async function refreshPreview() {
    const account = options.activeAccount.value;
    if (
      !options.supportsBackend
      || !account
      || !options.fromAddress.value
      || !selectedSignatureId.value
      || signatureVariant.value === 'none'
    ) {
      await options.transport.refreshSignaturePreview(null);
      return;
    }

    await options.transport.refreshSignaturePreview({
      accountId: account.id,
      identityEmail: options.fromAddress.value,
      threadId: options.contextThreadId.value,
      signatureId: selectedSignatureId.value,
      preference: signatureVariant.value,
    });
  }

  function select(id: string | null) {
    selectedSignatureId.value = id;
    signatureVariant.value = id ? 'auto' : 'none';
    clearOverride();
  }

  function selectVariant(variant: Exclude<ComposerSignatureVariant, 'none'>) {
    signatureVariant.value = variant;
    clearOverride();
  }

  function edit(payload: { html: string | null; text: string }) {
    const resolved = options.transport.resolvedSignature.value;
    if (signatureVariant.value === 'auto' && resolved) {
      signatureVariant.value = resolved.variant;
    }
    signatureOverrideText.value = payload.text;
    signatureOverrideHtml.value = payload.html === null
      ? null
      : serializeCidReferences(payload.html, resolved?.attachments ?? []);
  }

  return {
    load: signaturesStore.load,
    availableSignatures,
    selectedSignatureId,
    signatureVariant,
    signaturePreviewLoading: options.transport.signaturePreviewLoading,
    signatureOverrideText,
    signatureOverrideHtml,
    signatureDisplayHtml,
    signatureDisplayText,
    signatureOverrideActive,
    clearOverride,
    syncSelection,
    refreshPreview,
    select,
    selectVariant,
    edit,
  };
}
