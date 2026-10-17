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

export const useDraftsStore = defineStore('drafts', () => {
  const items = ref<BackendDraft[]>([]);
  const loading = ref(false);
  const lastError = ref<string | null>(null);

  const sorted = computed(() =>
    [...items.value].sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
  );

  const scheduled = computed(() =>
    items.value.filter((draft) => draft.scheduled_for && new Date(draft.scheduled_for) > new Date()),
  );

  async function load() {
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      items.value = await backendListDrafts();
      lastError.value = null;
    } catch (err) {
      lastError.value = err instanceof Error ? err.message : String(err);
    } finally {
      loading.value = false;
    }
  }

  async function save(input: BackendDraftInput): Promise<BackendDraft | null> {
    if (!backendCommandsAvailable()) return null;
    try {
      const draft = await backendSaveDraft(input);
      const idx = items.value.findIndex((item) => item.id === draft.id);
      if (idx >= 0) items.value.splice(idx, 1, draft);
      else items.value.unshift(draft);
      return draft;
    } catch (err) {
      lastError.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  async function fetchOne(id: string): Promise<BackendDraft | null> {
    if (!backendCommandsAvailable()) return null;
    return backendLoadDraft(id);
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
