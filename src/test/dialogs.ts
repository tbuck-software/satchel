import { watch } from 'vue';
import { vi } from 'vitest';
import { useConfirm, type ConfirmRequest } from '@/composables/useConfirm';
import { usePrompt, type PromptRequest } from '@/composables/usePrompt';

/*
 * Answer the app's in-app dialogs from a test, the way a spy on
 * `window.confirm` used to. Both dialogs are module-level state, so no
 * component has to be mounted: the watcher answers the moment a question is
 * pending, and `asked` records what was asked.
 */

export interface DialogStub<Request, Reply> {
  asked: ReturnType<typeof vi.fn<(request: Request) => void>>;
  /** Change the answer for the questions still to come. */
  reply: (next: Reply) => void;
  stop: () => void;
}

export function answerConfirms(initial: boolean): DialogStub<ConfirmRequest, boolean> {
  const { state, answer } = useConfirm();
  answer(false);
  let reply = initial;
  const asked = vi.fn<(request: ConfirmRequest) => void>();
  const stop = watch(
    () => state.pending,
    (pending) => {
      if (!pending) return;
      asked({ ...pending });
      answer(reply);
    },
    { flush: 'sync' },
  );
  return { asked, reply: (next) => { reply = next; }, stop };
}

export function answerPrompts(initial: string | null): DialogStub<PromptRequest, string | null> {
  const { state, answer } = usePrompt();
  answer(null);
  let reply = initial;
  const asked = vi.fn<(request: PromptRequest) => void>();
  const stop = watch(
    () => state.pending,
    (pending) => {
      if (!pending) return;
      asked({ ...pending });
      answer(reply);
    },
    { flush: 'sync' },
  );
  return { asked, reply: (next) => { reply = next; }, stop };
}
