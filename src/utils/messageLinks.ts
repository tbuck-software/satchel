import { installOpenUrlHandler, type DeepLinkSource } from './openUrl';

/**
 * `satchel://message/<Message-ID>` names a mail the same way on every device.
 * `satchel://thread/<id>` is the fallback for mail without a real Message-ID
 * and only resolves in the database it came from.
 *
 * The format is a contract with outside readers: docs/DB_READ_ACCESS.md.
 */
export type MessageLink =
  | { kind: 'message'; messageId: string }
  | { kind: 'thread'; threadId: string };

const SCHEME = 'satchel://';

/** `@` stays readable; `()` would end a Markdown link. */
function encodeSegment(value: string): string {
  return encodeURIComponent(value)
    .replace(/[()'*!]/g, (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)
    .replace(/%40/g, '@');
}

export function buildMessageLink(link: MessageLink): string {
  return link.kind === 'message'
    ? `${SCHEME}message/${encodeSegment(link.messageId)}`
    : `${SCHEME}thread/${encodeSegment(link.threadId)}`;
}

export function parseMessageLink(url: string): MessageLink | null {
  const match = /^satchel:\/\/(message|thread)\/([^?#]+?)\/?(?:[?#].*)?$/i.exec(url.trim());
  if (!match) return null;
  let value: string;
  try {
    value = decodeURIComponent(match[2]!).trim();
  } catch {
    return null;
  }
  if (match[1]!.toLowerCase() === 'message') {
    const messageId = value.replace(/^<|>$/g, '').trim();
    return messageId ? { kind: 'message', messageId } : null;
  }
  return value ? { kind: 'thread', threadId: value } : null;
}

export function messageLinkMarkdown(subject: string, url: string): string {
  const label = subject.replace(/\s+/g, ' ').trim().replace(/[\\[\]]/g, '\\$&');
  return label ? `[${label}](${url})` : `<${url}>`;
}

export function installMessageLinkOpenHandler(
  open: (link: MessageLink) => void,
  loadSource?: () => Promise<DeepLinkSource>,
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null,
): Promise<() => void> {
  return installOpenUrlHandler(parseMessageLink, open, 'satchel.messageLink.launchHandled', loadSource, storage);
}
