export interface NotificationActionPayload {
  extra?: Record<string, unknown>;
  /** Which button was pressed. Absent when the body itself was tapped. */
  actionId?: unknown;
  /** Text typed into a reply action, where the platform offers one. */
  inputValue?: unknown;
}

/** Identifiers shared with the Rust side and the registered action type. */
export const MAIL_ACTION_TYPE = 'satchel.mail';
export const MAIL_ACTIONS = {
  archive: 'satchel.archive',
  read: 'satchel.read',
  reply: 'satchel.reply',
} as const;

export type MailAction = (typeof MAIL_ACTIONS)[keyof typeof MAIL_ACTIONS];

/**
 * Which action a notification callback describes.
 *
 * Tapping the body reports no action on Android and a platform-specific
 * "default" identifier elsewhere, so anything unrecognised counts as "open it".
 */
export function notificationAction(payload: NotificationActionPayload): MailAction | null {
  const id = typeof payload.actionId === 'string' ? payload.actionId : '';
  return (Object.values(MAIL_ACTIONS) as string[]).includes(id) ? (id as MailAction) : null;
}

export function notificationReplyText(payload: NotificationActionPayload): string {
  return typeof payload.inputValue === 'string' ? payload.inputValue.trim() : '';
}

export interface NativeNotificationRoute {
  type?: unknown;
  threadId?: unknown;
}

export function notificationThreadId(notification: NotificationActionPayload): string | null {
  const value = notification.extra?.threadId ?? notification.extra?.thread_id;
  return typeof value === 'string' && value.trim() ? value : null;
}

export function notificationRouteThreadId(event: NativeNotificationRoute): string | null {
  return event.type === 'notification-tap'
    && typeof event.threadId === 'string'
    && event.threadId.trim()
    ? event.threadId
    : null;
}
