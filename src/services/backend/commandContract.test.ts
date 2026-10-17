import { describe, expect, it, vi } from 'vitest';
import { assertCommandPayload } from './commandContract';
import { sendCommand } from './mock';

describe('generated command argument contract', () => {
  it('rejects bad payloads before the mock handler runs', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    try {
      await expect(sendCommand('get_thread', { thread_id: 'thread' })).rejects.toThrow('invalid payload');
      expect(consoleError).toHaveBeenCalledWith(expect.stringContaining('thread_id'));
    } finally {
      consoleError.mockRestore();
    }
  });

  it('accepts top-level camelCase and nested snake_case', () => {
    expect(() => assertCommandPayload('get_thread', { threadId: 'thread' })).not.toThrow();
    expect(() => assertCommandPayload('set_threads_read', {
      input: { thread_ids: ['thread'], is_read: true },
    })).not.toThrow();
  });

  it('rejects renamed and missing top-level arguments', () => {
    expect(() => assertCommandPayload('get_thread', { thread_id: 'thread' })).toThrow('thread_id');
    expect(() => assertCommandPayload('get_thread')).toThrow('threadId');
  });

  it('rejects renamed nested arguments, including entries in arrays', () => {
    expect(() => assertCommandPayload('set_threads_read', {
      input: { threadIds: ['thread'], is_read: true },
    })).toThrow('threadIds');
    expect(() => assertCommandPayload('transition_threads', {
      input: { threads: [{ threadId: 'thread', from: 'inbox' }], to: 'done' },
    })).toThrow('threads[0].threadId');
  });

  it('accepts nullable Option fields and omitted serde defaults', () => {
    expect(() => assertCommandPayload('list_threads')).not.toThrow();
    expect(() => assertCommandPayload('list_threads', { input: { account_id: null } })).not.toThrow();
    expect(() => assertCommandPayload('send_message', {
      accountId: 'account', input: { to: ['friend@example.test'], subject: 'hello', body: 'body' },
    })).not.toThrow();
  });

  it('rejects unknown commands and malformed input objects', () => {
    expect(() => assertCommandPayload('invented_command')).toThrow('unknown Rust command');
    expect(() => assertCommandPayload('toString')).toThrow('unknown Rust command');
    expect(() => assertCommandPayload('__proto__')).toThrow('unknown Rust command');
    expect(() => assertCommandPayload('set_threads_read', { input: null })).toThrow('must be an object');
    expect(() => assertCommandPayload('set_threads_read', { input: { thread_ids: [], is_read: true, extra: 1 } })).toThrow('extra');
  });
});
