// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendSource, BackendWebhook } from './sources';

describe('mock feeds and webhooks', () => {
  beforeEach(() => {
    vi.resetModules();
  });

  it('starts empty, and adds and removes feeds like the app', async () => {
    const { sendCommand } = await import('./mock');
    expect(await sendCommand<BackendSource[]>('list_sources')).toEqual([]);

    const feed = await sendCommand<BackendSource>('create_source', {
      input: { name: ' Blog ', kind: 'rss', identity: 'https://blog.example.test/feed', config: {} },
    });
    expect(feed).toMatchObject({ display_name: 'Blog', kind: 'rss' });
    await expect(sendCommand('create_source', {
      input: { name: 'Bad', kind: 'rss', identity: 'ftp://example.test', config: {} },
    })).rejects.toThrow('feed URL must use http or https');

    await sendCommand('delete_source', { sourceId: feed.id });
    expect(await sendCommand<BackendSource[]>('list_sources')).toEqual([]);
  });

  it('accepts only HTTPS or local webhook URLs, and updates in place', async () => {
    const { sendCommand } = await import('./mock');
    expect(await sendCommand<BackendWebhook[]>('list_webhooks')).toEqual([]);
    await expect(sendCommand('upsert_webhook', {
      input: { url: 'http://hooks.example.test', events: [], enabled: true },
    })).rejects.toThrow('backend.errors.webhookUrlInvalid');

    const hook = await sendCommand<BackendWebhook>('upsert_webhook', {
      input: { url: 'https://hooks.example.test', events: ['thread.updated'], enabled: true },
    });
    await sendCommand('upsert_webhook', { input: { id: hook.id, url: hook.url, events: [], enabled: false } });
    expect(await sendCommand<BackendWebhook[]>('list_webhooks')).toMatchObject([{ id: hook.id, enabled: 0 }]);

    await sendCommand('delete_webhook', { id: hook.id });
    expect(await sendCommand<BackendWebhook[]>('list_webhooks')).toEqual([]);
  });
});
