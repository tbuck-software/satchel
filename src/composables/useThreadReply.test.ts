import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '@/types/mail';
import type { BackendDraft } from '@/services/backend';

const harness = vi.hoisted(() => ({
  messages: [] as Message[],
  drafts: [] as Array<Partial<BackendDraft>>,
  openDraft: vi.fn(),
  openReply: vi.fn(),
}));

vi.mock('@/stores/composer', () => ({
  useComposerStore: () => ({ openDraft: harness.openDraft, openReply: harness.openReply }),
}));
vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ getByThread: () => harness.messages }),
}));
vi.mock('@/stores/drafts', () => ({
  useDraftsStore: () => ({
    get items() { return harness.drafts; },
    get sorted() { return harness.drafts; },
  }),
}));

import { useThreadReply } from './useThreadReply';

function message(id: string, isDraft = false): Message {
  return {
    id,
    threadId: 'thread-1',
    from: { name: 'Sarah Kim', email: 'sarah@example.com' },
    to: [],
    subject: 'Staging',
    bodyText: id,
    timestamp: '2026-10-27T08:00:00Z',
    isRead: true,
    isDraft,
    attachments: [],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.messages = [message('first')];
  harness.drafts = [];
});

describe('useThreadReply', () => {
  it('starts a reply when the thread has no draft', () => {
    useThreadReply().reply('thread-1');
    expect(harness.openReply).toHaveBeenCalledWith('thread-1', false);
    expect(harness.openDraft).not.toHaveBeenCalled();
  });

  it('goes back into the newest unsent draft instead of starting a second reply', () => {
    harness.messages = [message('first'), message('draft-old', true), message('draft-new', true)];
    useThreadReply().reply('thread-1');
    expect(harness.openDraft).toHaveBeenCalledWith('draft-new');
    expect(harness.openReply).not.toHaveBeenCalled();
  });

  it('finds the draft in the drafts list when the thread is not loaded', () => {
    harness.messages = [];
    harness.drafts = [{ id: 'listed', thread_id: 'thread-1', scheduled_for: null }];
    useThreadReply().reply('thread-1');
    expect(harness.openDraft).toHaveBeenCalledWith('listed');
  });

  it('leaves a scheduled draft to its send', () => {
    harness.messages = [message('first'), message('queued', true)];
    harness.drafts = [{ id: 'queued', thread_id: 'thread-1', scheduled_for: '2026-10-28T08:00:00Z' }];
    useThreadReply().reply('thread-1');
    expect(harness.openReply).toHaveBeenCalledWith('thread-1', false);
    expect(harness.openDraft).not.toHaveBeenCalled();
  });

  it('starts its own composer for reply all', () => {
    harness.messages = [message('first'), message('draft-1', true)];
    useThreadReply().reply('thread-1', true);
    expect(harness.openReply).toHaveBeenCalledWith('thread-1', true);
    expect(harness.openDraft).not.toHaveBeenCalled();
  });
});
