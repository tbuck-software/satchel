import type { Message } from '@/types/mail';

/**
 * The text a chat bubble shows. Delta Chat sends plain text and "MAY" split
 * off a footer with the line `-- ` ("the user-text-part and a footer-part");
 * "full quotes, footers or sth. like that MUST NOT go to the user-text-part"
 * (chatmail spec.md, "Incoming messages"). Quoted lines go, the footer goes.
 * A message that is nothing but a quote keeps it rather than showing nothing.
 */
export function chatBubbleText(body: string): string {
  const lines = body.replace(/\r\n?/g, '\n').split('\n');
  const footer = lines.findIndex((line) => line === '-- ' || line === '--');
  const userPart = footer >= 0 ? lines.slice(0, footer) : lines;
  const unquoted = userPart.filter((line) => !line.trimStart().startsWith('>'));
  const text = unquoted.join('\n').replace(/\n{3,}/g, '\n\n').trim();
  return text || userPart.join('\n').trim();
}

/**
 * A list row's preview: the snippet is the body on one line, so the footer
 * starts at " -- " rather than on a line of its own.
 */
export function chatSnippet(snippet: string): string {
  const [text = ''] = snippet.split(/\s--\s/);
  return text.replace(/^(?:>[^\n]*\s+)+/, '').trim() || snippet.trim();
}

export function isOwnMessage(message: Pick<Message, 'from'>, ownAddresses: ReadonlySet<string>): boolean {
  return ownAddresses.has(message.from.email.trim().toLowerCase());
}

export interface ChatBubble {
  id: string;
  own: boolean;
  senderName: string;
  senderEmail: string;
  text: string;
  timestamp: string;
  /** The previous bubble came from the same person: no name above this one. */
  continued: boolean;
}

export interface ChatDay {
  /** `YYYY-MM-DD` in local time. */
  key: string;
  date: Date;
  bubbles: ChatBubble[];
}

function localDayKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

/** Messages oldest first, grouped by local day. Drafts are not chat lines. */
export function chatDays(messages: readonly Message[], ownAddresses: ReadonlySet<string>): ChatDay[] {
  const sorted = messages
    .filter((message) => !message.isDraft)
    .slice()
    .sort((a, b) => Date.parse(a.timestamp) - Date.parse(b.timestamp) || a.id.localeCompare(b.id));
  const days: ChatDay[] = [];
  for (const message of sorted) {
    const date = new Date(message.timestamp);
    const key = localDayKey(date);
    let day = days.at(-1);
    if (!day || day.key !== key) {
      day = { key, date, bubbles: [] };
      days.push(day);
    }
    const previous = day.bubbles.at(-1);
    const sender = message.from.email.trim().toLowerCase();
    day.bubbles.push({
      id: message.id,
      own: isOwnMessage(message, ownAddresses),
      senderName: message.from.name || message.from.email,
      senderEmail: sender,
      text: chatBubbleText(message.bodyText),
      timestamp: message.timestamp,
      continued: previous?.senderEmail === sender,
    });
  }
  return days;
}

/** Today, yesterday, or the date. */
export function relativeDay(date: Date, now = new Date()): 'today' | 'yesterday' | null {
  const key = localDayKey(date);
  if (key === localDayKey(now)) return 'today';
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  return key === localDayKey(yesterday) ? 'yesterday' : null;
}
