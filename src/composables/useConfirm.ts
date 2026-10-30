import { reactive } from 'vue';

export interface ConfirmRequest {
  title: string;
  /** Sentence under the title. A title that already is the question needs none. */
  body?: string;
  /** Label of the button that goes through with it. */
  confirmLabel: string;
  /** Extra line under the body — what exactly is at stake. */
  detail?: string;
  /** Styles the confirming button as destructive. */
  tone?: 'default' | 'danger';
}

/**
 * How a question ended. `preempted` means a newer question took the dialog
 * before anyone answered this one.
 */
export type ConfirmDecision = 'confirmed' | 'declined' | 'preempted';

interface PendingConfirm extends ConfirmRequest {
  resolve: (decision: ConfirmDecision) => void;
}

const state = reactive({ pending: null as PendingConfirm | null });

/**
 * Ask the user a yes/no question from anywhere, including a composable that
 * cannot render.
 *
 * `ConfirmDialog` is mounted once in `App.vue` and renders whatever is pending
 * here. Callers `await ask(...)` and get `false` for every way out — cancel,
 * Escape, backdrop, the system back gesture — so a refusal never reads as a
 * yes. There is no `window.confirm` fallback on purpose: it is not guaranteed
 * to exist in WKWebView and cannot be styled or translated.
 */
export function useConfirm() {
  /**
   * `ask`, for a caller that must not read a question nobody answered as a
   * no: content that exists nowhere else waits for a real answer.
   */
  function decide(request: ConfirmRequest): Promise<ConfirmDecision> {
    // A second question would otherwise orphan the first one's promise.
    state.pending?.resolve('preempted');
    return new Promise<ConfirmDecision>((resolve) => {
      state.pending = { ...request, resolve };
    });
  }

  async function ask(request: ConfirmRequest): Promise<boolean> {
    return (await decide(request)) === 'confirmed';
  }

  function answer(confirmed: boolean) {
    const pending = state.pending;
    state.pending = null;
    pending?.resolve(confirmed ? 'confirmed' : 'declined');
  }

  return { state, ask, decide, answer };
}
