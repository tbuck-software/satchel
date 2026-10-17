import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import { t } from '@/i18n/runtime';
import { backendCommandsAvailable, backendFocusDraftEditor } from '@/services/backend';

export type ComposerMode = 'new' | 'reply' | 'reply-all' | 'forward' | 'resend';

export interface ComposerRequest {
  mode: ComposerMode;
  threadId?: string | null;
  draftId?: string | null;
  to?: string[];
  cc?: string[];
  bcc?: string[];
  subject?: string;
  body?: string;
  bodyHtml?: string | null;
}

/**
 * Asked before an open composer is replaced. `true` once it is closed (saved,
 * or pristine); `false` keeps it, and the new request is dropped.
 */
export type ComposerReplaceGuard = (next: ComposerRequest) => Promise<boolean>;

export const useComposerStore = defineStore('composer', () => {
  const request = ref<ComposerRequest | null>(null);
  const instance = ref(0);
  /**
   * A message key while the title is derived from the mode, a literal once the
   * composer reports its subject. Storing the key keeps the derived title
   * following a language change instead of freezing the one it opened in.
   */
  const titleKey = ref('compose.title.new');
  const titleOverride = ref('');
  const title = computed(() => titleOverride.value || t(titleKey.value));

  const isOpen = computed(() => request.value !== null);

  let replaceGuard: ComposerReplaceGuard | null = null;
  let openTicket = 0;

  function setReplaceGuard(guard: ComposerReplaceGuard): () => void {
    replaceGuard = guard;
    return () => {
      if (replaceGuard === guard) replaceGuard = null;
    };
  }

  /**
   * Opens a composer for `next`. The app behind the composer stays clickable,
   * so a reply or a mailto link can arrive while one is open: it goes through
   * the host's close path instead of replacing the window under the user.
   * Resolves `false` when nothing opened.
   */
  async function open(next: ComposerRequest = { mode: 'new' }): Promise<boolean> {
    const ticket = ++openTicket;
    const normalized: ComposerRequest = {
      mode: next.mode,
      threadId: next.threadId ?? null,
      draftId: next.draftId ?? null,
      to: [...(next.to ?? [])],
      cc: [...(next.cc ?? [])],
      bcc: [...(next.bcc ?? [])],
      subject: next.subject ?? '',
      body: next.body ?? '',
      bodyHtml: next.bodyHtml ?? null,
    };
    // One window per draft: a draft open in a popout is brought forward there.
    if (normalized.draftId && backendCommandsAvailable()) {
      try {
        if (await backendFocusDraftEditor(normalized.draftId)) return false;
      } catch (error) {
        console.warn('draft editor lookup failed', error);
      }
    }
    if (request.value && replaceGuard && !(await replaceGuard(normalized))) return false;
    // A newer request arrived while this one waited.
    if (ticket !== openTicket) return false;
    request.value = normalized;
    titleOverride.value = '';
    titleKey.value = next.mode === 'forward'
      ? 'common.forward'
      : next.mode === 'resend'
        ? 'compose.title.resend'
      : next.mode === 'reply' || next.mode === 'reply-all'
        ? 'common.reply'
        : 'compose.title.new';
    instance.value += 1;
    return true;
  }

  function openNew() {
    return open({ mode: 'new' });
  }

  function openEmail(to: string[], options: { subject?: string; body?: string } = {}) {
    return open({
      mode: 'new',
      to,
      subject: options.subject,
      body: options.body,
    });
  }

  function openDraft(draftId: string) {
    return open({ mode: 'new', draftId });
  }

  function openReply(threadId: string, replyAll = false) {
    return open({ mode: replyAll ? 'reply-all' : 'reply', threadId });
  }

  function openForward(threadId: string) {
    return open({ mode: 'forward', threadId });
  }

  function openResend(threadId: string) {
    return open({ mode: 'resend', threadId });
  }

  function setTitle(value: string) {
    const next = value.trim();
    if (next) titleOverride.value = next;
  }

  function close() {
    request.value = null;
  }

  return {
    request,
    instance,
    title,
    isOpen,
    open,
    openNew,
    openEmail,
    openDraft,
    openReply,
    openForward,
    openResend,
    setTitle,
    close,
    setReplaceGuard,
  };
});
