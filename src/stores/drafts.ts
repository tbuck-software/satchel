import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendCommandsAvailable,
  backendDeleteDraft,
  backendListDrafts,
  backendLoadDraft,
  backendSaveDraft,
  type BackendDraft,
  type BackendDraftInput,
} from '@/services/backend';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import { useConnectionStore } from '@/stores/connection';
import { useMessagesStore } from '@/stores/messages';

export const useDraftsStore = defineStore('drafts', () => {
  const items = ref<BackendDraft[]>([]);
  const loading = ref(false);
  const lastError = ref<string | null>(null);
  const connectionStore = useConnectionStore();

  const sorted = computed(() =>
    [...items.value].sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
  );

  const scheduled = computed(() =>
    items.value.filter((draft) => !draft.content_error && draft.scheduled_for && new Date(draft.scheduled_for) > new Date()),
  );

  async function load() {
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      items.value = await backendListDrafts();
      lastError.value = null;
      connectionStore.clearBackendError('drafts');
    } catch (err) {
      // `items` stays as it was: a failed list says nothing about the drafts.
      // The view reads the connection channel to tell this from "no drafts".
      lastError.value = err instanceof Error ? err.message : String(err);
      connectionStore.reportBackendError('drafts', err);
    } finally {
      loading.value = false;
    }
  }

  async function save(input: BackendDraftInput): Promise<BackendDraft | null> {
    if (!backendCommandsAvailable()) return null;
    try {
      const known = items.value.find((item) => item.id === input.id);
      if (known?.content_error) throw new Error(backendErrorLabel(known.content_error), { cause: known.content_error });
      const draft = await backendSaveDraft(input);
      const idx = items.value.findIndex((item) => item.id === draft.id);
      if (idx >= 0) items.value.splice(idx, 1, draft);
      else items.value.unshift(draft);
      useMessagesStore().applySavedDraft(draft);
      return draft;
    } catch (err) {
      lastError.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  async function fetchOne(id: string): Promise<BackendDraft | null> {
    if (!backendCommandsAvailable()) return null;
    const draft = await backendLoadDraft(id);
    if (draft?.content_error) throw new Error(backendErrorLabel(draft.content_error), { cause: draft.content_error });
    const idx = items.value.findIndex((item) => item.id === id);
    if (draft && idx >= 0) items.value.splice(idx, 1, draft);
    return draft;
  }

  async function remove(id: string) {
    if (!backendCommandsAvailable()) return;
    await backendDeleteDraft(id);
    items.value = items.value.filter((item) => item.id !== id);
  }

  /**
   * Puts a deleted draft back. save_draft upserts on the id, so the draft
   * returns under its original one rather than as a copy — which is what makes
   * an undo honest.
   */
  async function restore(draft: BackendDraft): Promise<BackendDraft | null> {
    if (draft.content_error) throw new Error(backendErrorLabel(draft.content_error), { cause: draft.content_error });
    return save({
      id: draft.id,
      sendOptions: draft.send_options,
      accountId: draft.account_id,
      threadId: draft.thread_id ?? null,
      to: draft.to,
      cc: draft.cc,
      bcc: draft.bcc,
      subject: draft.subject,
      body: draft.body,
      bodyHtml: draft.body_html ?? null,
      fromAddress: draft.from_address ?? null,
      fromName: draft.from_name ?? null,
      replyTo: draft.reply_to ?? null,
      scheduledFor: draft.scheduled_for ?? null,
      attachments: draft.attachments,
      signatureId: draft.signature_id ?? null,
      signatureVariant: draft.signature_variant ?? null,
      signatureOverrideText: draft.signature_override_text ?? null,
      signatureOverrideHtml: draft.signature_override_html ?? null,
    });
  }

  return { items, sorted, scheduled, loading, lastError, load, save, fetchOne, remove, restore };
});
