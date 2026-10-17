import { ref } from 'vue';
import {
  backendResolveSignature,
  backendSendMessage,
  backendCommandsAvailable,
  backendUndoOutboxSend,
} from '@/services/backend';
import type {
  BackendDraft,
  BackendResolvedSignature,
  BackendSendMessagePayload,
  BackendSendMessageResult,
} from '@/types/composer';

export interface SignaturePreviewRequest {
  accountId: string;
  identityEmail: string;
  threadId?: string | null;
  signatureId?: string | null;
  preference: 'auto' | 'long' | 'short' | 'none';
}

/**
 * Owns the command transport used by the composer. Signature previews are
 * deliberately kept here with their request sequencing so an older IPC
 * response cannot replace a newer identity or variant selection.
 */
export function useComposerTransport() {
  const resolvedSignature = ref<BackendResolvedSignature | null>(null);
  const signaturePreviewLoading = ref(false);
  let signaturePreviewRequest = 0;

  function commandsAvailable(): boolean {
    return backendCommandsAvailable();
  }

  async function refreshSignaturePreview(request: SignaturePreviewRequest | null): Promise<void> {
    const requestId = ++signaturePreviewRequest;
    if (
      !request
      || !commandsAvailable()
      || !request.accountId
      || !request.identityEmail
      || !request.signatureId
      || request.preference === 'none'
    ) {
      resolvedSignature.value = null;
      signaturePreviewLoading.value = false;
      return;
    }

    signaturePreviewLoading.value = true;
    try {
      const resolved = await backendResolveSignature(request);
      if (requestId === signaturePreviewRequest) resolvedSignature.value = resolved;
    } catch (error) {
      if (requestId === signaturePreviewRequest) {
        resolvedSignature.value = null;
        console.warn('signature preview failed', error);
      }
    } finally {
      if (requestId === signaturePreviewRequest) signaturePreviewLoading.value = false;
    }
  }

  function sendMessage(payload: BackendSendMessagePayload): Promise<BackendSendMessageResult> {
    return backendSendMessage(payload);
  }

  /** Takes a send back inside its Undo window; the message returns as a draft. */
  function undoSend(queuedId: string): Promise<BackendDraft> {
    return backendUndoOutboxSend(queuedId);
  }

  return {
    resolvedSignature,
    signaturePreviewLoading,
    commandsAvailable,
    refreshSignaturePreview,
    sendMessage,
    undoSend,
  };
}
