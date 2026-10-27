/**
 * Where a binding applies. This is the runtime rule, not a label: `global` and
 * `navigation` work on every page, `mailbox` and `mail` only where a mail list
 * or reader is on screen, and none of them while a dialog, menu or the palette
 * covers the page.
 */
export type KeybindingContext = 'global' | 'navigation' | 'mailbox' | 'mail';

export interface KeybindingSpec {
  action: string;
  defaultBinding: string;
  context: KeybindingContext;
  /**
   * A second key for another action, not an action of its own. It keeps its
   * own id so it can be rebound separately and saved bindings stay valid; the
   * help lists it on the row of the action it triggers.
   */
  aliasOf?: string;
  /**
   * A `g` sequence: the route it opens. Its label is the sidebar's
   * (`viewLabelKey`), so the help, the hint and the palette name it the same way.
   */
  view?: string;
}

const VIEW_LABEL_KEYS: Readonly<Record<string, string>> = {
  'new-senders': 'newSenders.title',
  calendar: 'nav.calendar',
  contacts: 'nav.contacts',
};

/** The name a view shares with the sidebar; the mail lists are `nav.folders.<route>`. */
export function viewLabelKey(route: string): string {
  return VIEW_LABEL_KEYS[route] ?? `nav.folders.${route}`;
}

/** Older names a view is still found by in the palette and the shortcut editor. */
export const VIEW_ALIAS_KEYS: Readonly<Record<string, readonly string[]>> = {
  later: ['folders.snoozed', 'folders.waitingForReply'],
  done: ['folders.archive'],
};

export const KEYBINDING_SPECS: readonly KeybindingSpec[] = [
  { action: 'openCommandPalette', defaultBinding: ':', context: 'global' },
  { action: 'help', defaultBinding: '?', context: 'global' },
  { action: 'compose', defaultBinding: 'n', context: 'global' },
  { action: 'undo', defaultBinding: 'z', context: 'global' },
  { action: 'nextThread', defaultBinding: 'j', context: 'mailbox' },
  { action: 'prevThread', defaultBinding: 'k', context: 'mailbox' },
  { action: 'openItem', defaultBinding: 'l', context: 'mailbox' },
  { action: 'leaveItem', defaultBinding: 'h', context: 'mailbox' },
  { action: 'toggleMessage', defaultBinding: 'o', context: 'mail' },
  { action: 'toggleAllMessages', defaultBinding: 'O', context: 'mail' },
  { action: 'previewAttachment', defaultBinding: 'space', context: 'mail' },
  { action: 'toggleSelection', defaultBinding: 'x', context: 'mailbox' },
  { action: 'selectAll', defaultBinding: 'a', context: 'mailbox' },
  { action: 'clearSelection', defaultBinding: 'c', context: 'mailbox' },
  { action: 'reply', defaultBinding: 'r', context: 'mail' },
  { action: 'replyAll', defaultBinding: 'A', context: 'mail' },
  { action: 'forward', defaultBinding: 'f', context: 'mail' },
  { action: 'markDone', defaultBinding: 'd', context: 'mail' },
  { action: 'archive', defaultBinding: 'e', context: 'mail', aliasOf: 'markDone' },
  { action: 'moveWaiting', defaultBinding: 'w', context: 'mail' },
  { action: 'moveSnoozed', defaultBinding: 's', context: 'mail' },
  { action: 'moveInbox', defaultBinding: 'i', context: 'mail' },
  { action: 'setAside', defaultBinding: 'b', context: 'mail' },
  { action: 'toggleRead', defaultBinding: 'u', context: 'mail' },
  { action: 'trash', defaultBinding: '#', context: 'mail' },
  { action: 'pin', defaultBinding: '*', context: 'mail' },
  { action: 'fileTo', defaultBinding: 'v', context: 'mail' },
  { action: 'addToTopic', defaultBinding: 't', context: 'mail', aliasOf: 'fileTo' },
  { action: 'copyLink', defaultBinding: 'y', context: 'mail' },
  { action: 'toggleMute', defaultBinding: 'm', context: 'mail' },
  { action: 'markSpam', defaultBinding: '!', context: 'mail' },
  { action: 'print', defaultBinding: 'mod+p', context: 'mail' },
  { action: 'goInbox', defaultBinding: 'g i', context: 'navigation', view: 'inbox' },
  { action: 'goLater', defaultBinding: 'g l', context: 'navigation', view: 'later' },
  { action: 'goAside', defaultBinding: 'g b', context: 'navigation', view: 'aside' },
  { action: 'goDone', defaultBinding: 'g e', context: 'navigation', view: 'done' },
  { action: 'goSent', defaultBinding: 'g t', context: 'navigation', view: 'sent' },
  { action: 'goDrafts', defaultBinding: 'g d', context: 'navigation', view: 'drafts' },
  { action: 'goOutbox', defaultBinding: 'g o', context: 'navigation', view: 'outbox' },
  { action: 'goNewSenders', defaultBinding: 'g n', context: 'navigation', view: 'new-senders' },
  { action: 'goCalendar', defaultBinding: 'g c', context: 'navigation', view: 'calendar' },
  { action: 'goContacts', defaultBinding: 'g p', context: 'navigation', view: 'contacts' },
] as const;

export const DEFAULT_KEYBINDINGS: Readonly<Record<string, string>> = Object.freeze(
  Object.fromEntries(KEYBINDING_SPECS.map((spec) => [spec.action, spec.defaultBinding])),
);
