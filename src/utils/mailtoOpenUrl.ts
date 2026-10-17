import { parseMailtoLink, type MailtoTarget } from './externalLinks';
import { installOpenUrlHandler, type DeepLinkSource } from './openUrl';

export type { DeepLinkSource } from './openUrl';

/** `mailto:` links from a browser, another app or the system's default mail client setting. */
export function installMailtoOpenHandler(
  openCompose: (target: MailtoTarget) => void,
  loadSource?: () => Promise<DeepLinkSource>,
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null,
): Promise<() => void> {
  return installOpenUrlHandler(parseMailtoLink, openCompose, 'satchel.mailto.launchHandled', loadSource, storage);
}
