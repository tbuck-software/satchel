import type { Message } from '@/types/mail';
import { removeComposerTextHistory } from '@/utils/composerHistory';
import { textFromHtml } from '@/utils/emailQuotes';

/**
 * What the user has written so far in a draft: the text above the history the
 * composer quoted under it. A reply draft is mostly that history, and a
 * preview of it would show the other person's words as if they were ours.
 */
export function draftPreviewText(message: Pick<Message, 'bodyText' | 'bodyHtml'>): string {
  const source = message.bodyText.trim() ? message.bodyText : textFromHtml(message.bodyHtml ?? '');
  const own = removeComposerTextHistory(source.replace(/\r\n?/g, '\n'));
  const lines = own.split('\n');
  // Quotes the composer did not introduce (pasted, or from another client).
  const quoteStart = lines.findIndex((line) => line.trimStart().startsWith('>'));
  return (quoteStart >= 0 ? lines.slice(0, quoteStart) : lines)
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}
