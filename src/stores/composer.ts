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

/** The message key of the heading a composer opened in `mode` carries. */
export function composerModeTitleKey(mode: ComposerMode) {
  if (mode === 'forward') return 'common.forward';
  if (mode === 'resend') return 'compose.title.resend';
  if (mode === 'reply' || mode === 'reply-all') return 'common.reply';
  return 'compose.title.new';
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
   * Derived from the mode until the composer reports its subject. Translating
   * on read keeps the derived title following a language change instead of
   * freezing the one it opened in.
   */
  const titleOverride = ref('');
  const title = computed(() => titleOverride.value || t(composerModeTitleKey(request.value?.mode ?? 'new')));

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
    instance.value += 1;
    return true;
  }

  /**
   * Every way back into a draft opens it as a new message, because only the
   * loaded draft knows what it answers. Once it has loaded, a reply draft
   * says so in the heading instead of posing as a new message.
   */
  function adoptDraftMode(mode: ComposerMode) {
    if (request.value?.draftId && request.value.mode === 'new') request.value.mode = mode;
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
    adoptDraftMode,
    close,
    setReplaceGuard,
  };
});
