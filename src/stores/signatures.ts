import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendDeleteSignature,
  backendDiscoverSignatureSuggestions,
  backendListSignatures,
  backendListSignatureAssignments,
  backendListSignatureSuggestions,
  backendSetSignatureSuggestionStatus,
  backendCommandsAvailable,
  backendUpsertSignatureAssignment,
  backendUpsertSignature,
  type BackendSignatureAssignment,
  type BackendSignatureSuggestion,
  type BackendSignature,
} from '@/services/backend';

/** What a deleted signature takes with it, and what Undo has to put back. */
export interface RemovedSignature {
  signature: BackendSignature;
  assignments: BackendSignatureAssignment[];
}

export const useSignaturesStore = defineStore('signatures', () => {
  const items = ref<BackendSignature[]>([]);
  const loading = ref(false);
  const assignments = ref<BackendSignatureAssignment[]>([]);
  const suggestions = ref<BackendSignatureSuggestion[]>([]);

  const byAccount = computed(() => {
    const map = new Map<string, BackendSignature[]>();
    for (const sig of items.value) {
      const list = map.get(sig.account_id) ?? [];
      list.push(sig);
      map.set(sig.account_id, list);
    }
    return map;
  });

  function defaultFor(accountId: string): BackendSignature | null {
    const list = byAccount.value.get(accountId) ?? [];
    return list.find((s) => s.is_default === 1) ?? list[0] ?? null;
  }

  function assignmentFor(accountId: string, identityEmail: string) {
    const normalized = identityEmail.trim().toLowerCase();
    return (
      assignments.value.find(
        (assignment) =>
          assignment.account_id === accountId && assignment.identity_email === normalized,
      ) ?? null
    );
  }

  function selectedFor(accountId: string, identityEmail: string): BackendSignature | null {
    const assigned = assignmentFor(accountId, identityEmail);
    if (assigned?.signature_id) {
      return items.value.find((signature) => signature.id === assigned.signature_id) ?? null;
    }
    return defaultFor(accountId);
  }

  async function load() {
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      [items.value, assignments.value, suggestions.value] = await Promise.all([
        backendListSignatures(),
        backendListSignatureAssignments(),
        backendListSignatureSuggestions(),
      ]);
    } finally {
      loading.value = false;
    }
  }

  async function upsert(input: Parameters<typeof backendUpsertSignature>[0]) {
    const sig = await backendUpsertSignature(input);
    const idx = items.value.findIndex((s) => s.id === sig.id);
    if (idx >= 0) items.value.splice(idx, 1, sig);
    else items.value.push(sig);
    if (sig.is_default === 1) {
      // Reset other defaults locally
      items.value = items.value.map((s) =>
        s.account_id === sig.account_id && s.id !== sig.id ? { ...s, is_default: 0 } : s,
      );
    }
    return sig;
  }

  /**
   * Resolves with everything Undo needs: the signature and the identities that
   * pointed at it. Deleting the row sets `signature_id` to NULL on those
   * assignments in the database, so putting the signature back on its own would
   * return a signature nobody uses.
   */
  async function remove(id: string): Promise<RemovedSignature | null> {
    const signature = items.value.find((s) => s.id === id) ?? null;
    const bound = assignments.value.filter((assignment) => assignment.signature_id === id);
    await backendDeleteSignature(id);
    items.value = items.value.filter((s) => s.id !== id);
    if (bound.length > 0) {
      assignments.value = assignments.value.map((assignment) =>
        assignment.signature_id === id ? { ...assignment, signature_id: null } : assignment,
      );
    }
    return signature ? { signature, assignments: bound } : null;
  }

  /** Undo for a delete: the same id, then the identities that used it. */
  async function restore(removed: RemovedSignature) {
    const signature = removed.signature;
    await upsert({
      id: signature.id,
      accountId: signature.account_id,
      name: signature.name,
      bodyText: signature.body_text,
      bodyHtml: signature.body_html ?? null,
      shortBodyText: signature.short_body_text,
      shortBodyHtml: signature.short_body_html ?? null,
      assets: (signature.assets ?? []).map((asset) => ({
        filename: asset.filename,
        mimeType: asset.mime_type,
        contentId: asset.content_id,
        contentBase64: asset.content_base64,
      })),
      isDefault: signature.is_default === 1,
    });
    for (const assignment of removed.assignments) {
      await assign({
        accountId: assignment.account_id,
        identityEmail: assignment.identity_email,
        signatureId: signature.id,
        firstContactMode: assignment.first_contact_mode,
      });
    }
  }

  async function assign(input: Parameters<typeof backendUpsertSignatureAssignment>[0]) {
    const assignment = await backendUpsertSignatureAssignment(input);
    const index = assignments.value.findIndex(
      (item) =>
        item.account_id === assignment.account_id &&
        item.identity_email === assignment.identity_email,
    );
    if (index >= 0) assignments.value.splice(index, 1, assignment);
    else assignments.value.push(assignment);
    return assignment;
  }

  async function discover(accountId: string, identityEmail: string) {
    const found = await backendDiscoverSignatureSuggestions({ accountId, identityEmail });
    const normalizedIdentity = identityEmail.trim().toLowerCase();
    const merged = [
      ...suggestions.value.filter(
        (item) =>
          item.account_id !== accountId || item.identity_email !== normalizedIdentity,
      ),
      ...found,
    ];
    suggestions.value = [...new Map(merged.map((item) => [item.id, item])).values()];
    return found;
  }

  async function setSuggestionStatus(id: string, status: 'accepted' | 'dismissed') {
    await backendSetSignatureSuggestionStatus(id, status);
    suggestions.value = suggestions.value.filter((item) => item.id !== id);
  }

  /** Undo for a dismissal: back to pending and back into the list. */
  async function restoreSuggestion(suggestion: BackendSignatureSuggestion) {
    await backendSetSignatureSuggestionStatus(suggestion.id, 'pending');
    if (!suggestions.value.some((item) => item.id === suggestion.id)) {
      suggestions.value = [...suggestions.value, suggestion];
    }
  }

  return {
    items,
    assignments,
    suggestions,
    byAccount,
    loading,
    defaultFor,
    assignmentFor,
    selectedFor,
    load,
    upsert,
    remove,
    restore,
    assign,
    discover,
    setSuggestionStatus,
    restoreSuggestion,
  };
});
