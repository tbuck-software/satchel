import type { Message } from '@/types/mail';
import { preferredExternalReplyAddress } from '@/utils/mailRecipients';
import { buildForwardBody, buildReplyBody, messageBodyText, prefixSubject } from '@/utils/composerText';
import { extractPlainTextBody } from '@/utils/emailQuotes';

export function normalizeOwnedAddresses(addresses: Iterable<string>): Set<string> {
  return new Set(
    [...addresses]
      .map((address) => address.trim().toLowerCase())
      .filter(Boolean),
  );
}

export function findResendSource(
  messages: readonly Message[],
  ownAddresses: ReadonlySet<string>,
): Message | undefined {
  return [...messages]
    .reverse()
    .find((message) => ownAddresses.has(message.from.email.trim().toLowerCase()))
    ?? messages.at(-1);
}

/** Quote text rather than sender HTML so opening the editor cannot load trackers. */
function threadBody(messages: readonly Message[], forward = false) {
  const coveredQuotes: string[] = [];
  // Keep URLs and case in comparisons: mail that differs only by a link must
  // still be quoted. Ignore only quote depth and whitespace introduced by clients.
  const comparisonText = (text: string) => text
    .replace(/^(?:[ \t]*>[ \t]?)+/gm, '')
    .replace(/\s+/g, ' ').trim();
  const quotes = [...messages].reverse()
    .filter((message) => !message.isDraft)
    .filter((message) => {
      const text = messageBodyText(message);
      const comparable = comparisonText(text);
      if (comparable && coveredQuotes.some((quote) => quote.includes(comparable))) return false;
      // The newest message may already carry older messages. Preserve that
      // existing history, then add only messages it does not already quote.
      const history = extractPlainTextBody(text).quotes.map((quote) => quote.text).join('\n');
      if (history) coveredQuotes.push(comparisonText(history));
      return true;
    })
    .map((message) => forward ? buildForwardBody(message) : buildReplyBody(message));
  if (!quotes.length) return { body: null, bodyHtml: null };
  const html = quotes.map((quote) => {
    const paragraphs = quote.trim().split('\n').map((line) => {
      const paragraph = document.createElement('p');
      paragraph.textContent = forward ? line : line.replace(/^> ?/, '');
      return paragraph.outerHTML;
    }).join('');
    return `<blockquote>${paragraphs}</blockquote>`;
  }).join('');
  return { body: quotes.join(''), bodyHtml: `<p></p>${html}` };
}

export function forwardThreadPrefill(messages: readonly Message[], threadSubject: string) {
  const last = messages.at(-1);
  return {
    subject: prefixSubject(last?.subject || threadSubject, 'Fwd'),
    ...threadBody(messages, true),
  };
}

export function replyThreadPrefill(
  mode: 'reply' | 'reply-all',
  messages: readonly Message[],
  threadSubject: string,
  ownAddresses: ReadonlySet<string>,
) {
  const last = messages.at(-1);
  const external = [...messages]
    .reverse()
    .find((message) => !ownAddresses.has(message.from.email.trim().toLowerCase()))
    ?? last;
  const seen = new Set<string>();
  const uniqueExternal = (values: string[]) => values.filter((value) => {
    const normalized = value.trim().toLowerCase();
    if (!normalized || ownAddresses.has(normalized) || seen.has(normalized)) return false;
    seen.add(normalized);
    return true;
  });

  const to = uniqueExternal([
    preferredExternalReplyAddress(external?.replyTo, external?.from.email, ownAddresses) ?? '',
    ...(mode === 'reply-all' ? (external?.to ?? []).map((participant) => participant.email) : []),
  ]);
  const cc = mode === 'reply-all'
    ? uniqueExternal((external?.cc ?? []).map((participant) => participant.email))
    : [];

  return {
    subject: prefixSubject(last?.subject || threadSubject, 'Re'),
    to,
    cc,
    // The recipients come from this message, so the threading headers have to
    // as well. The newest message in the thread is often our own sent copy.
    replyToMessageId: external?.id ?? null,
    ...threadBody(messages),
  };
}

/** Template contents cannot fetch images while their URLs are being removed. */
export function resendEditorHtml(html: string | null | undefined): string | null {
  if (!html) return null;
  const template = document.createElement('template');
  template.innerHTML = html;
  const fragment = template.content;
  for (const element of fragment.querySelectorAll('source, video, audio, iframe, object, embed, link, meta, style, svg, math, template, base, input, script')) element.remove();
  for (const image of fragment.querySelectorAll('img')) {
    const source = image.getAttribute('src') ?? '';
    if (!/^(?:cid:|data:image\/(?:png|jpeg|gif|webp);base64,)/i.test(source)) image.remove();
    else {
      for (const attribute of Array.from(image.attributes)) {
        if (!['src', 'alt', 'title', 'width', 'height'].includes(attribute.name)) image.removeAttribute(attribute.name);
      }
    }
  }
  for (const element of fragment.querySelectorAll('[style], [background]')) {
    element.removeAttribute('style');
    element.removeAttribute('background');
  }
  return template.innerHTML;
}
