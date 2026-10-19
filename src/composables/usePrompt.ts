import { reactive } from 'vue';

export interface PromptRequest {
  title: string;
  /** Optional sentence under the title — what the value is for. */
  body?: string;
  /** Visible label of the text field. */
  label: string;
  /** What the field holds when the dialog opens. */
  initialValue?: string;
  /** Label of the button that submits the value. */
  confirmLabel: string;
  /** `password` masks what is typed — for passphrases. */
  inputType?: 'text' | 'password';
}

interface PendingPrompt extends PromptRequest {
  resolve: (value: string | null) => void;
}

const state = reactive({ pending: null as PendingPrompt | null });

/**
 * Ask the user for one line of text from anywhere — the in-app replacement
 * for `window.prompt`.
 *
 * `PromptDialog` is mounted once in `App.vue` and renders whatever is pending
 * here. Callers `await ask(...)` and get the submitted text, or `null` for
 * every way out that is not the submit button — cancel, Escape, the system
 * back gesture. `window.prompt` is not guaranteed to exist in WKWebView, and
 * where it does it shows passphrases in clear text and cannot be translated.
 */
export function usePrompt() {
  function ask(request: PromptRequest): Promise<string | null> {
    // A second question would otherwise orphan the first one's promise.
    state.pending?.resolve(null);
    return new Promise<string | null>((resolve) => {
      state.pending = { ...request, resolve };
    });
  }

  function answer(value: string | null) {
    const pending = state.pending;
    state.pending = null;
    pending?.resolve(value);
  }

  return { state, ask, answer };
}
