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
  /**
   * `body` is text the user wrote outside the composer, such as a notification
   * reply, and exists nowhere else until it is saved. The host asks and saves
   * it before a replacement, even while the form reports it untouched.
   */
  unsavedBody?: boolean;
}

/** The message key of the heading a composer opened in `mode` carries. */
export function composerModeTitleKey(mode: ComposerMode) {
  if (mode === 'forward') return 'common.forward';
  if (mode === 'resend') return 'compose.title.resend';
  if (mode === 'reply' || mode === 'reply-all') return 'common.reply';
  return 'compose.title.new';
}

/**
 * The host's answer before an open composer is replaced: `replaced` once it is
 * closed (saved, or pristine), `kept` when it stays by choice, `unsaved` when
 * it had to stay because its draft could not be saved, `interrupted` when
 * another question took the dialog before anyone answered. Only `replaced`
 * lets the new request in.
 */
export type ComposerReplaceOutcome = 'replaced' | 'kept' | 'unsaved' | 'interrupted';

export type ComposerReplaceGuard = (next: ComposerRequest) => Promise<ComposerReplaceOutcome>;

/**
 * What became of an open request. `focused` found its draft in another window.
 * `kept` is the user's refusal. Neither `unsaved` (accepted, but the open draft
 * could not be saved to make room) nor `interrupted` (nobody answered) is.
 */
export type ComposerOpenOutcome = 'opened' | 'focused' | 'kept' | 'unsaved' | 'interrupted';

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
  /** Settles once every request so far is decided; `null` while idle. */
  let openTail: Promise<void> | null = null;

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
    return (await openWithOutcome(next)) === 'opened';
  }

  /**
   * `open`, for a caller that has to tell a refusal from a failure: content
   * that exists nowhere else, such as a notification reply, may only be let go
   * when the user kept the open draft. Not when saving it failed, and not when
   * nobody answered.
   */
  function openWithOutcome(next: ComposerRequest = { mode: 'new' }): Promise<ComposerOpenOutcome> {
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
      ...(next.unsavedBody ? { unsavedBody: true } : {}),
    };
    // One request at a time, in arrival order. A second one waits for the
    // answer to the first instead of taking its question away, then meets the
    // composer that answer left open. An idle store starts at once.
    const turn = openTail ? openTail.then(() => openNow(normalized)) : openNow(normalized);
    const tail = turn.then(() => {}, () => {});
    openTail = tail;
    void tail.then(() => {
      if (openTail === tail) openTail = null;
    });
    return turn;
  }

  async function openNow(normalized: ComposerRequest): Promise<ComposerOpenOutcome> {
    // One window per draft: a draft open in a popout is brought forward there.
    if (normalized.draftId && backendCommandsAvailable()) {
      try {
        if (await backendFocusDraftEditor(normalized.draftId)) return 'focused';
      } catch (error) {
        console.warn('draft editor lookup failed', error);
      }
    }
    if (request.value && replaceGuard) {
      const replaced = await replaceGuard(normalized);
      if (replaced !== 'replaced') return replaced;
    }
    request.value = normalized;
    titleOverride.value = '';
    instance.value += 1;
    return 'opened';
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
    openWithOutcome,
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
