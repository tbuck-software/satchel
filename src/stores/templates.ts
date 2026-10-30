import { useConnectionStore } from './connection';
import { orderedLoad } from './internal/orderedLoad';
import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendDeleteTemplate,
  backendListTemplates,
  backendRenderTemplate,
  backendCommandsAvailable,
  backendUpsertTemplate,
  type BackendRenderedTemplate,
  type BackendTemplate,
} from '@/services/backend';

export const useTemplatesStore = defineStore('templates', () => {
  const items = ref<BackendTemplate[]>([]);
  const connection = useConnectionStore();
  const loader = orderedLoad();
  const { loading, loadFailed } = loader;

  async function load() {
    if (!backendCommandsAvailable()) return;
    await loader.load(backendListTemplates, (rows) => { items.value = rows; },
      (error) => connection.reportBackendError('templates', error),
      () => connection.clearBackendError('templates'));
  }

  async function upsert(input: Parameters<typeof backendUpsertTemplate>[0]) {
    return loader.mutate(async () => {
      const tpl = await backendUpsertTemplate(input);
      const idx = items.value.findIndex((t) => t.id === tpl.id);
      if (idx >= 0) items.value.splice(idx, 1, tpl);
      else items.value.push(tpl);
      return tpl;
    });
  }

  /** Resolves with the deleted template so the caller can offer Undo. */
  async function remove(id: string): Promise<BackendTemplate | null> {
    return loader.mutate(async () => {
      const template = items.value.find((t) => t.id === id) ?? null;
      await backendDeleteTemplate(id);
      items.value = items.value.filter((t) => t.id !== id);
      return template;
    });
  }

  /** Undo for a delete: the same id, so it survives a reload as itself. */
  async function restore(template: BackendTemplate) {
    await upsert({
      id: template.id,
      accountId: template.account_id ?? null,
      name: template.name,
      subject: template.subject ?? null,
      bodyText: template.body_text,
      bodyHtml: template.body_html ?? null,
      shortcut: template.shortcut ?? null,
      cc: parseRecipients(template.cc_json),
      bcc: parseRecipients(template.bcc_json),
      attachmentsJson: template.attachments_json ?? null,
    });
  }

  /** Recipients travel as a JSON array; the upsert wants them as a list. */
  function parseRecipients(value?: string | null): string[] {
    if (!value) return [];
    try {
      const parsed: unknown = JSON.parse(value);
      return Array.isArray(parsed) ? parsed.filter((item) => typeof item === 'string') : [];
    } catch {
      return [];
    }
  }

  async function render(
    templateId: string,
    variables: Record<string, string> = {},
  ): Promise<BackendRenderedTemplate> {
    return backendRenderTemplate({ templateId, variables });
  }

  function findByShortcut(shortcut: string): BackendTemplate | null {
    const lower = shortcut.toLowerCase();
    return items.value.find((t) => t.shortcut?.toLowerCase() === lower) ?? null;
  }

  return { items, loading, loadFailed, load, upsert, remove, restore, render, findByShortcut };
});
