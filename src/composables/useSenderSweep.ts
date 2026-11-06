import { reactive } from 'vue';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';

export interface SenderSweepRequest {
  accountId: string;
  email: string;
  name?: string;
  /** Opens on the "from now on" part, as the cleanup view's rule key does. */
  focus?: 'now' | 'rule';
}

const state = reactive({ request: null as SenderSweepRequest | null });

/**
 * The sender sweep dialog, opened from anywhere: the reader, a row's sender
 * menu, the palette, the cleanup view. `SenderSweepDialog` is mounted once in
 * `App.vue` and renders whatever is requested here.
 */
export function useSenderSweep() {
  function open(request: SenderSweepRequest) {
    const email = request.email.trim().toLowerCase();
    if (!request.accountId || !email.includes('@')) return false;
    state.request = { ...request, email };
    return true;
  }

  /**
   * A sweep of a whole thread is about whoever started it — not the latest
   * sender, which in a conversation is as often the user. The backend knows
   * the first message even when the reader has not loaded it.
   */
  async function openForThread(threadId: string): Promise<boolean> {
    if (!threadId) return false;
    try {
      // Loaded here, not at startup: the store carries the whole sweep API.
      const { useSweepRulesStore } = await import('@/stores/sweepRules');
      const sender = await useSweepRulesStore().threadSender(threadId);
      if (sender) return open({ accountId: sender.account_id, email: sender.email, name: sender.name ?? undefined });
    } catch {
      // Falls through to the same notice as a thread only the user wrote.
    }
    useToast().info(t('sweep.noSender'));
    return false;
  }

  function close() {
    state.request = null;
  }

  return { state, open, openForThread, close };
}
