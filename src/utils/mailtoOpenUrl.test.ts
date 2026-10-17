import { describe, expect, it, vi } from 'vitest';
import type { MailtoTarget } from './externalLinks';
import { installMailtoOpenHandler, type DeepLinkSource } from './mailtoOpenUrl';

function source(launch: string[] | null) {
  let deliver: (urls: string[]) => void = () => {};
  const unlisten = vi.fn();
  const deepLinks: DeepLinkSource = {
    onOpenUrl: vi.fn(async (handler) => {
      deliver = handler;
      return unlisten;
    }),
    getCurrent: vi.fn(async () => launch),
  };
  return { deepLinks, deliver: (urls: string[]) => deliver(urls), unlisten };
}

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
  };
}

describe('mailto links from outside', () => {
  it('opens the composer for a link clicked while the app runs', async () => {
    const opened: MailtoTarget[] = [];
    const { deepLinks, deliver } = source(null);
    await installMailtoOpenHandler((target) => opened.push(target), async () => deepLinks, memoryStorage());

    deliver(['mailto:mara@example.com?subject=Hallo&body=Guten%20Tag']);

    expect(opened).toEqual([expect.objectContaining({
      recipients: ['mara@example.com'],
      subject: 'Hallo',
      body: 'Guten Tag',
    })]);
  });

  it('opens the composer for the link that launched the app, once per launch', async () => {
    const storage = memoryStorage();
    const launch = ['mailto:mara@example.com?subject=Kaltstart'];
    const opened: MailtoTarget[] = [];

    await installMailtoOpenHandler((target) => opened.push(target), async () => source(launch).deepLinks, storage);
    // A reload of the webview asks the plugin again and gets the same answer.
    await installMailtoOpenHandler((target) => opened.push(target), async () => source(launch).deepLinks, storage);

    expect(opened.map((target) => target.subject)).toEqual(['Kaltstart']);
  });

  it('ignores files and callbacks that share the channel, without opening anything', async () => {
    const open = vi.fn();
    const { deepLinks, deliver } = source(['file:///Users/me/invite.ics']);
    await installMailtoOpenHandler(open, async () => deepLinks, memoryStorage());

    deliver(['de.tbuck.satchel://oauth/callback?code=1', 'https://example.com', 'mailto:not-an-address']);

    expect(open).not.toHaveBeenCalled();
  });
});
