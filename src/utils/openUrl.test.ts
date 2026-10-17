import { expect, it, vi } from 'vitest';
import { verifiedDeepLinkSource } from './openUrl';

it('rejects forged URLs and only opens the backend-confirmed deep link', async () => {
  let listener: (urls: string[]) => void = () => {};
  const open = vi.fn();
  const source = verifiedDeepLinkSource({
    getCurrent: async () => ['satchel://thread/trusted'],
    onOpenUrl: async (handler) => { listener = handler; return () => {}; },
  });
  await source.onOpenUrl(open);
  listener(['satchel://thread/forged']);
  await Promise.resolve();
  expect(open).not.toHaveBeenCalled();
  listener(['satchel://thread/trusted']);
  await Promise.resolve();
  expect(open).toHaveBeenCalledWith(['satchel://thread/trusted']);
});
