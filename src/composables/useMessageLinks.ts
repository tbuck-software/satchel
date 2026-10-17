import { useRouter } from 'vue-router';
import { backendGetThread, backendMessageLinkTarget, backendResolveMessageLink } from '@/services/backend';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';
import { buildMessageLink, messageLinkMarkdown, type MessageLink } from '@/utils/messageLinks';

export function useMessageLinks() {
  const router = useRouter();
  const toast = useToast();

  async function resolve(link: MessageLink): Promise<string | null> {
    if (link.kind === 'message') return backendResolveMessageLink(link.messageId);
    try {
      return (await backendGetThread(link.threadId)).id;
    } catch {
      return null;
    }
  }

  async function open(link: MessageLink): Promise<void> {
    let threadId: string | null;
    try {
      threadId = await resolve(link);
    } catch (error) {
      console.warn('satchel link could not be resolved', error);
      threadId = null;
    }
    if (!threadId) {
      toast.warning(t('thread.link.notFound'));
      return;
    }
    await router.push({ name: 'thread', params: { id: threadId } });
  }

  /** Copies `[Subject](satchel://message/…)`; Markdown because agents and notes read it. */
  async function copy(threadId: string): Promise<void> {
    try {
      const target = await backendMessageLinkTarget(threadId);
      const url = buildMessageLink(target.message_id
        ? { kind: 'message', messageId: target.message_id }
        : { kind: 'thread', threadId: target.thread_id });
      await navigator.clipboard.writeText(messageLinkMarkdown(target.subject, url));
      toast.success(t('thread.link.copied'));
    } catch (error) {
      toast.error(t('thread.link.copyFailed', { error: error instanceof Error ? error.message : String(error) }));
    }
  }

  return { open, copy };
}
