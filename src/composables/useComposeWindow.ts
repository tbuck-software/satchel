import {
  backendOpenComposeWindow,
  backendSupportsNativeCommands,
} from '@/services/backend';
import { isMobileRuntime } from '@/utils/runtimeShell';
import { BackendError } from '@/utils/backendErrors';

interface ComposeWindowLocation {
  name: string;
  query: Record<string, string>;
}

interface ComposeWindowRouter {
  resolve(location: ComposeWindowLocation): { href: string };
  push(location: ComposeWindowLocation): Promise<unknown>;
}

/**
 * Owns the three ways of opening a compose window: a native desktop window,
 * the standalone compose route in a new browser tab, or — on a phone or tablet,
 * which have one window — that same route in place.
 *
 * Callers that want an in-window fallback can use `openNative` first. The
 * complete `open` operation is for actions that should always leave the
 * current composer in its own window.
 */
export function useComposeWindow(router: ComposeWindowRouter) {
  function composeLocation(draftId?: string | null): ComposeWindowLocation {
    return {
      name: 'compose',
      query: { standalone: '1', ...(draftId ? { draftId } : {}) },
    };
  }

  function supportsNative(): boolean {
    // A second window is a desktop concept. Asking only whether native
    // commands exist would report success on mobile and open nothing.
    return backendSupportsNativeCommands() && !isMobileRuntime();
  }

  async function openNative(draftId?: string | null): Promise<boolean> {
    if (!supportsNative()) return false;
    await backendOpenComposeWindow(draftId);
    return true;
  }

  async function open(draftId?: string | null): Promise<void> {
    if (await openNative(draftId)) return;

    if (isMobileRuntime()) {
      await router.push(composeLocation(draftId));
      return;
    }

    const href = router.resolve(composeLocation(draftId)).href;
    const opened = window.open(href, '_blank', 'noopener,noreferrer');
    if (!opened) {
      throw new BackendError('backend.errors.browserWindowBlocked');
    }
    opened.opener = null;
  }

  return { open, openNative, supportsNative };
}
