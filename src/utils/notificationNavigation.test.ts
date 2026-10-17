import { describe, expect, it } from 'vitest';
import { notificationAction, notificationReplyText, notificationThreadId, notificationRouteThreadId } from './notificationNavigation';

describe('notification action navigation', () => {
  it('returns the thread attached to a native notification action', () => {
    expect(notificationThreadId({ extra: { threadId: 'thread-42' } })).toBe('thread-42');
  });

  it('routes only explicit native notification taps', () => {
    expect(notificationRouteThreadId({ type: 'notification-tap', threadId: 'thread-42' })).toBe('thread-42');
    expect(notificationRouteThreadId({ type: 'something-else', threadId: 'thread-42' })).toBeNull();
    expect(notificationRouteThreadId({ type: 'notification-tap', threadId: 42 })).toBeNull();
  });

  it('ignores notifications without a valid navigation target', () => {
    expect(notificationThreadId({ extra: {} })).toBeNull();
    expect(notificationThreadId({ extra: { threadId: 42 } })).toBeNull();
  });
});

describe('which button was pressed', () => {
  it('recognises the mail actions', () => {
    expect(notificationAction({ actionId: 'satchel.archive' })).toBe('satchel.archive');
    expect(notificationAction({ actionId: 'satchel.read' })).toBe('satchel.read');
    expect(notificationAction({ actionId: 'satchel.reply' })).toBe('satchel.reply');
  });

  /**
   * Tapping the body reports nothing on Android and a platform-specific
   * "default" identifier elsewhere. Anything unrecognised has to mean "open
   * it", or a plain tap would silently do nothing.
   */
  it('treats a plain tap and anything unknown as no action', () => {
    expect(notificationAction({})).toBeNull();
    expect(notificationAction({ actionId: '' })).toBeNull();
    expect(notificationAction({ actionId: 'com.apple.UNNotificationDefaultActionIdentifier' })).toBeNull();
  });

  it('carries the text typed into a reply, trimmed', () => {
    expect(notificationReplyText({ inputValue: '  bin gleich da  ' })).toBe('bin gleich da');
    expect(notificationReplyText({})).toBe('');
  });
});
