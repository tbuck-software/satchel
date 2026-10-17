/**
 * URLs handed to Satchel from outside: a browser, another app, the Finder.
 *
 * The deep-link plugin delivers them on macOS and on mobile, to a running app
 * as an event and to a cold start through `getCurrent()`. Every scheme shares
 * that one channel, so each handler parses what it owns and ignores the rest.
 */
export interface DeepLinkSource {
  onOpenUrl(handler: (urls: string[]) => void): Promise<() => void>;
  getCurrent(): Promise<string[] | null>;
}

let deepLinkSource: Promise<DeepLinkSource> | null = null;

function loadDeepLinkSource(): Promise<DeepLinkSource> {
  deepLinkSource ??= import('@tauri-apps/plugin-deep-link').then(verifiedDeepLinkSource);
  return deepLinkSource;
}

export function verifiedDeepLinkSource(source: DeepLinkSource): DeepLinkSource {
  return {
    getCurrent: () => source.getCurrent(),
    onOpenUrl: (handler) => source.onOpenUrl((urls) => {
      void source.getCurrent().then((current) => {
        if (current && urls.length === current.length && urls.every((url, i) => url === current[i])) {
          handler(current);
        }
      }).catch((error) => console.warn('deep-link origin verification failed', error));
    }),
  };
}

export async function installOpenUrlHandler<T>(
  parse: (url: string) => T | null,
  open: (target: T) => void,
  /** `getCurrent()` keeps answering after a reload; this remembers it was used. */
  handledLaunchKey: string,
  loadSource: () => Promise<DeepLinkSource> = loadDeepLinkSource,
  storage: Pick<Storage, 'getItem' | 'setItem'> | null = globalThis.sessionStorage ?? null,
): Promise<() => void> {
  const source = await loadSource();
  const openAll = (urls: string[]) => {
    for (const url of urls) {
      const target = parse(url);
      if (target) open(target);
    }
  };

  const unlisten = await source.onOpenUrl(openAll);
  const launch = (await source.getCurrent()) ?? [];
  const launchKey = launch.join('\n');
  if (launchKey && storage?.getItem(handledLaunchKey) !== launchKey) {
    storage?.setItem(handledLaunchKey, launchKey);
    openAll(launch);
  }
  return unlisten;
}
