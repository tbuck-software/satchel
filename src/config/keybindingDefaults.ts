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
}

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
  { action: 'archive', defaultBinding: 'e', context: 'mail' },
  { action: 'moveWaiting', defaultBinding: 'w', context: 'mail' },
  { action: 'moveSnoozed', defaultBinding: 's', context: 'mail' },
  { action: 'moveInbox', defaultBinding: 'i', context: 'mail' },
  { action: 'setAside', defaultBinding: 'b', context: 'mail' },
  { action: 'toggleRead', defaultBinding: 'u', context: 'mail' },
  { action: 'trash', defaultBinding: '#', context: 'mail' },
  { action: 'pin', defaultBinding: '*', context: 'mail' },
  { action: 'addToTopic', defaultBinding: 't', context: 'mail' },
  { action: 'copyLink', defaultBinding: 'y', context: 'mail' },
  { action: 'toggleMute', defaultBinding: 'm', context: 'mail' },
  { action: 'markSpam', defaultBinding: '!', context: 'mail' },
  { action: 'print', defaultBinding: 'mod+p', context: 'mail' },
  { action: 'goInbox', defaultBinding: 'g i', context: 'navigation' },
  { action: 'goDrafts', defaultBinding: 'g d', context: 'navigation' },
  { action: 'goOutbox', defaultBinding: 'g o', context: 'navigation' },
  { action: 'goCalendar', defaultBinding: 'g c', context: 'navigation' },
  { action: 'goFollowUps', defaultBinding: 'g f', context: 'navigation' },
  { action: 'goContacts', defaultBinding: 'g p', context: 'navigation' },
] as const;

export const DEFAULT_KEYBINDINGS: Readonly<Record<string, string>> = Object.freeze(
  Object.fromEntries(KEYBINDING_SPECS.map((spec) => [spec.action, spec.defaultBinding])),
);
