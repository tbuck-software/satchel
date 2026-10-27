import { computed, type ComputedRef } from 'vue';
import { useRouter } from 'vue-router';
import { useThreadsStore } from '@/stores/threads';
import { useAccountsStore } from '@/stores/accounts';
import { useComposerStore } from '@/stores/composer';
import { useThreadReply } from '@/composables/useThreadReply';
import { useSettingsStore } from '@/stores/settings';
import { useTopicsStore } from '@/stores/topics';
import { useThreadActions } from '@/composables/useThreadActions';
import { useMessageLinks } from '@/composables/useMessageLinks';
import { useShortcutHints, type ShortcutHint } from '@/composables/useShortcutHints';
import { activeThreadReaderCommands } from '@/composables/useThreadReaderCommands';
import { useUndoFab } from '@/composables/useUndoFab';
import { useToast } from '@/composables/useToast';
import { useMailSync } from '@/composables/useMailSync';
import { useUiStore } from '@/stores/ui';
import { backendStartOauthFlow } from '@/services/backend';
import { transportErrorCause } from '@/utils/transportLabels';
import { t } from '@/i18n/runtime';
import { availableSettingsSections } from '@/components/settings/settingsSections';
import type { CommandIconName } from '@/components/navigation/commandIcons';

export interface OmniCommand {
  id: string;
  title: string;
  context: 'mail' | 'navigation' | 'app';
  /** The glyph the same action wears in the toolbar or sidebar; none rather than a stand-in. */
  icon?: CommandIconName;
  shortcut?: ShortcutHint | null;
  /** Extra match terms (English aliases etc.) beyond the visible title. */
  keywords?: string;
  /** Listed only once the user types: one entry per folder would bury the rest. */
  searchOnly?: boolean;
  run: () => unknown;
}

/**
 * The command list for the omni panel. `keywords` keeps the muscle-memory
 * aliases searchable alongside the visible title.
 */
export function useCommands(): { commands: ComputedRef<OmniCommand[]> } {
  const router = useRouter();
  const threads = useThreadsStore();
  const accounts = useAccountsStore();
  const composer = useComposerStore();
  const threadReply = useThreadReply();
  const settings = useSettingsStore();
  const topics = useTopicsStore();
  const threadActions = useThreadActions();
  const messageLinks = useMessageLinks();
  const toast = useToast();
  const { syncMail } = useMailSync();
  const ui = useUiStore();
  const { hint } = useShortcutHints();
  const undoFab = useUndoFab();

  function targetIds(): string[] {
    return threads.selectedThreadIds.length > 0
      ? [...threads.selectedThreadIds]
      : threads.activeThreadId
        ? [threads.activeThreadId]
        : [];
  }

  /** Reader-only actions, offered while the reader shows the thread the palette acts on. */
  function readerCommands(): OmniCommand[] {
    const reader = activeThreadReaderCommands();
    if (!reader?.threadId || reader.threadId !== threads.activeThreadId) return [];
    return [
      {
        id: 'thread:block-address',
        icon: 'block',
        title: t('commands.blockAddress'),
        context: 'mail',
        keywords: t('commands.keywords.block'),
        run: () => reader.blockSender('address'),
      },
      {
        id: 'thread:block-domain',
        icon: 'block',
        title: t('commands.blockDomain'),
        context: 'mail',
        keywords: t('commands.keywords.block'),
        run: () => reader.blockSender('domain'),
      },
      {
        id: 'thread:print-conversation',
        icon: 'print',
        title: t('commands.printConversation'),
        context: 'mail',
        shortcut: hint('print'),
        keywords: t('commands.keywords.print'),
        run: () => reader.print('conversation'),
      },
      {
        id: 'thread:print-latest',
        icon: 'print',
        title: t('commands.printLatest'),
        context: 'mail',
        keywords: t('commands.keywords.print'),
        run: () => reader.print('latest'),
      },
      {
        id: 'thread:export-eml',
        icon: 'export',
        title: t('commands.exportEml'),
        context: 'mail',
        keywords: t('commands.keywords.export'),
        run: () => reader.exportThread('eml'),
      },
      {
        id: 'thread:export-markdown',
        icon: 'export',
        title: t('commands.exportMarkdown'),
        context: 'mail',
        keywords: t('commands.keywords.export'),
        run: () => reader.exportThread('markdown'),
      },
      {
        id: 'thread:export-text',
        icon: 'export',
        title: t('commands.exportText'),
        context: 'mail',
        keywords: t('commands.keywords.export'),
        run: () => reader.exportThread('text'),
      },
      ...reader.moveTargets.map((target): OmniCommand => ({
        id: `thread:move:${target.id}`,
        icon: 'folder',
        title: t('commands.moveToFolder', { folder: target.label }),
        context: 'mail',
        keywords: t('commands.keywords.moveToFolder'),
        searchOnly: true,
        run: () => reader.move(target.id),
      })),
    ];
  }

  function activeEmailAccountId(): string | undefined {
    return (
      accounts.emailAccounts.find((account) => account.id === accounts.activeAccountId)?.id
      ?? accounts.emailAccounts[0]?.id
    );
  }

  async function startOauth(provider: 'gmail' | 'outlook', label: string) {
    const accountId = activeEmailAccountId();
    if (!accountId) {
      toast.error(t('commands.noEmailAccount'));
      return;
    }
    try {
      const result = await backendStartOauthFlow({ accountId, provider });
      toast.info(
        result.browser_opened
          ? t('commands.oauthOpened', { provider: label })
          : t('commands.oauthStarted', { provider: label, url: result.authorization_url }),
      );
    } catch (error) {
      // "Sign-in could not be started" alone leaves nothing to do about it.
      const cause = transportErrorCause(error);
      toast.error(cause
        ? t('commands.oauthFailedCause', { provider: label, cause })
        : t('commands.oauthFailed', { provider: label }));
    }
  }

  const commands = computed<OmniCommand[]>(() => [
    {
      id: 'view:inbox',
      icon: 'inbox',
      title: t('commands.openInbox'),
      context: 'navigation',
      keywords: t('commands.keywords.openInbox'),
      run: () => router.push({ name: 'inbox' }),
    },
    {
      id: 'view:calendar',
      icon: 'calendar',
      title: t('commands.openCalendar'),
      context: 'navigation',
      keywords: t('commands.keywords.openCalendar'),
      run: () => router.push({ name: 'calendar' }),
    },
    {
      id: 'view:followups',
      icon: 'waiting',
      title: t('commands.openFollowUps'),
      context: 'navigation',
      keywords: t('commands.keywords.openFollowUps'),
      run: () => router.push({ name: 'followups' }),
    },
    {
      id: 'compose:new',
      icon: 'compose',
      title: t('commands.compose'),
      context: 'app',
      shortcut: hint('compose'),
      keywords: t('commands.keywords.compose'),
      run: () => composer.openNew(),
    },
    ...(targetIds().length > 0 ? [
      {
        id: 'thread:reply',
        icon: 'reply' as const,
        title: t('common.reply'),
        context: 'mail' as const,
        shortcut: hint('reply'),
        keywords: t('commands.keywords.reply'),
        run: () => threads.activeThreadId && threadReply.reply(threads.activeThreadId),
      },
      {
        id: 'thread:reply-all',
        icon: 'replyAll' as const,
        title: t('common.replyAll'),
        context: 'mail' as const,
        shortcut: hint('replyAll'),
        keywords: t('commands.keywords.replyAll'),
        run: () => threads.activeThreadId && composer.openReply(threads.activeThreadId, true),
      },
      {
        id: 'thread:forward',
        icon: 'forward' as const,
        title: t('common.forward'),
        context: 'mail' as const,
        shortcut: hint('forward'),
        keywords: t('commands.keywords.forward'),
        run: () => threads.activeThreadId && composer.openForward(threads.activeThreadId),
      },
      {
        id: 'thread:done',
        icon: 'done' as const,
        title: targetIds().length > 1
          ? t('commands.markDoneMany', { count: targetIds().length })
          : t('commands.markDone'),
        context: 'mail' as const,
        shortcut: hint('markDone'),
        keywords: t('commands.keywords.markDone'),
        run: () => void threadActions.archive(targetIds()),
      },
      {
        id: 'thread:snooze',
        icon: 'clock' as const,
        title: t('commands.snooze'),
        context: 'mail' as const,
        shortcut: hint('moveSnoozed'),
        keywords: t('commands.keywords.snooze'),
        run: () => ui.openReminder(targetIds()),
      },
      {
        id: 'thread:waiting',
        icon: 'waiting' as const,
        title: t('commands.waiting'),
        context: 'mail' as const,
        shortcut: hint('moveWaiting'),
        keywords: t('commands.keywords.waiting'),
        run: () => void threadActions.waitForReply(targetIds()),
      },
      {
        id: 'thread:aside',
        icon: 'aside' as const,
        title: t('commands.setAside'),
        context: 'mail' as const,
        shortcut: hint('setAside'),
        keywords: t('commands.keywords.setAside'),
        run: () => void threadActions.setAside(targetIds()),
      },
      {
        id: 'thread:inbox',
        icon: 'inbox' as const,
        title: t('commands.moveToInbox'),
        context: 'mail' as const,
        shortcut: hint('moveInbox'),
        keywords: t('commands.keywords.moveToInbox'),
        run: () => void threadActions.moveToInbox(targetIds()),
      },
      {
        id: 'thread:toggle-read',
        icon: 'envelope' as const,
        title: targetIds().some((id) => threads.find(id)?.unreadCount)
          ? t('commands.markRead')
          : t('commands.markUnread'),
        context: 'mail' as const,
        shortcut: hint('toggleRead'),
        keywords: t('commands.keywords.toggleRead'),
        run: () => {
          const ids = targetIds();
          const anyUnread = ids.some((id) => threads.find(id)?.unreadCount);
          return threadActions.setRead(ids, Boolean(anyUnread)).then(() => {});
        },
      },
      {
        id: 'thread:pin',
        icon: 'pin' as const,
        title: t('commands.pin'),
        context: 'mail' as const,
        shortcut: hint('pin'),
        keywords: t('commands.keywords.pin'),
        run: () => threads.activeThreadId && void threadActions.togglePinned(threads.activeThreadId),
      },
      {
        id: 'thread:topic',
        icon: 'topic' as const,
        title: t('commands.addToTopic'),
        context: 'mail' as const,
        shortcut: hint('addToTopic'),
        keywords: t('commands.keywords.addToTopic'),
        run: () => topics.openPicker({ itemType: 'thread', itemIds: targetIds() }),
      },
      {
        id: 'thread:copy-link',
        icon: 'link' as const,
        title: t('commands.copyLink'),
        context: 'mail' as const,
        shortcut: hint('copyLink'),
        keywords: t('commands.keywords.copyLink'),
        run: () => threads.activeThreadId && void messageLinks.copy(threads.activeThreadId),
      },
      {
        id: 'thread:trash',
        icon: 'trash' as const,
        title: targetIds().length > 1
          ? t('commands.trashMany', { count: targetIds().length })
          : t('common.delete'),
        context: 'mail' as const,
        shortcut: hint('trash'),
        keywords: t('commands.keywords.trash'),
        run: () => void threadActions.trash(targetIds()),
      },
      {
        id: 'thread:mute',
        icon: 'mute' as const,
        title: threads.activeThreadId && threads.find(threads.activeThreadId)?.isMuted
          ? t('thread.actions.unmute')
          : t('thread.actions.mute'),
        context: 'mail' as const,
        shortcut: hint('toggleMute'),
        keywords: t('commands.keywords.mute'),
        run: () => {
          const id = threads.activeThreadId;
          if (id) void threadActions.setMuted(id, !threads.find(id)?.isMuted);
        },
      },
      {
        id: 'thread:spam',
        icon: 'spam' as const,
        title: t('thread.toolbar.markSpam'),
        context: 'mail' as const,
        shortcut: hint('markSpam'),
        keywords: t('commands.keywords.spam'),
        run: () => void threadActions.markSpam(targetIds()),
      },
      ...readerCommands(),
    ] : []),
    ...(undoFab.entries.value.length > 0 ? [{
      id: 'undo:latest',
      icon: 'undo' as const,
      title: t('commands.undo', { label: undoFab.entries.value.at(-1)?.label ?? '' }),
      context: 'app' as const,
      shortcut: hint('undo'),
      keywords: t('commands.keywords.undo'),
      run: () => {
        const entry = undoFab.entries.value.at(-1);
        if (entry) void undoFab.run(entry);
      },
    }] : []),
    {
      id: 'theme:toggle',
      icon: 'theme',
      title: t('commands.toggleTheme'),
      context: 'app',
      keywords: t('commands.keywords.toggleTheme'),
      run: () => settings.toggleTheme(),
    },
    {
      id: 'sync:run',
      icon: 'sync',
      title: t('commands.syncMail'),
      context: 'app',
      keywords: t('commands.keywords.syncMail'),
      run: syncMail,
    },
    {
      id: 'oauth:gmail',
      icon: 'account',
      title: t('commands.oauthGmail'),
      context: 'app',
      keywords: t('commands.keywords.oauthGmail'),
      run: () => startOauth('gmail', 'Gmail'),
    },
    {
      id: 'oauth:outlook',
      icon: 'account',
      title: t('commands.oauthOutlook'),
      context: 'app',
      keywords: t('commands.keywords.oauthOutlook'),
      run: () => startOauth('outlook', 'Outlook'),
    },
    {
      id: 'help:shortcuts',
      icon: 'keyboard',
      title: t('commands.showShortcuts'),
      context: 'app',
      shortcut: hint('help'),
      keywords: t('commands.keywords.showShortcuts'),
      run: () => ui.toggleShortcuts(),
    },
    {
      id: 'settings:keybindings',
      icon: 'keyboard',
      title: t('commands.editShortcuts'),
      context: 'app',
      keywords: t('commands.keywords.editShortcuts'),
      run: () => router.push({ name: 'settings', query: { section: 'shortcuts' } }),
    },
    // One per section, found by name or by what the section is for.
    ...availableSettingsSections().flatMap((group) => group.items).map((section) => ({
      id: `settings:section:${section}`,
      icon: 'settings' as const,
      title: t('commands.openSettingsSection', { section: t(`settings.sections.${section}.label`) }),
      context: 'navigation' as const,
      keywords: `${t(`settings.sections.${section}.description`)} ${t('commands.keywords.openSettingsSection')}`,
      searchOnly: true,
      run: () => ui.openSettings(section),
    })),
  ]);

  return { commands };
}
