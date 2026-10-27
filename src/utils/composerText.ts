import { datetimeFormats } from '@/i18n/formats';
import REPLY_PREFIXES from '../../crates/satchel-core/src/email/reply-prefixes.json';
import FORWARD_PREFIXES from '../../crates/satchel-core/src/email/forward-prefixes.json';
import { currentLocale, t } from '@/i18n/runtime';
import type { Message } from '@/types/mail';
import { htmlToPlainText } from '@/utils/emailDocument';

export { normalizeVisibleText, normalizedAddresses } from './composerNormalization';
export { bytesToBase64 } from './base64';

/**
 * Text derivations for the composer: subject prefixes, forwarded headers,
 * recipient normalisation and the snapshot inputs used for dirty tracking.
 *
 * These are the parts of composing that are decided before anything is sent
 * and that a user notices immediately when they are wrong — a doubled subject
 * prefix, a duplicated recipient, an unsaved-changes prompt on an untouched
 * draft.
 */

// The composer shares Core's localized prefix vocabulary. Single-letter and
// reference labels stay out because they also begin ordinary subject text.
type PrefixKind = 'reply' | 'forward';

/**
 * One prefix at the head of a subject: an optional counter ("Re[2]:") and any
 * amount of space, including the non-breaking kind French clients put before
 * the colon.
 */
const LEADING_PREFIX =
  /^[\s\u00a0]*(\p{L}+)[\s\u00a0]*(?:\[\s*\d+\s*\]|\(\s*\d+\s*\))?[\s\u00a0]*:[\s\u00a0]*/u;

function classifyPrefix(token: string): PrefixKind | null {
  const normalized = token.toLowerCase();
  // Reply is checked first: "VS:" is a reply in Finnish and a forward in
  // Danish, and reply is the far more common reading.
  if (REPLY_PREFIXES.includes(normalized)) return 'reply';
  if (FORWARD_PREFIXES.includes(normalized)) return 'forward';
  return null;
}

/**
 * Peel recognised prefixes off the front of a subject.
 *
 * Stops at the first token it does not know, so "Wichtig: bitte lesen" and
 * "Von: Lena" keep their text — an unrecognised word before a colon is
 * subject, not metadata.
 */
export function splitSubjectPrefixes(value: string): { prefixes: Array<{ kind: PrefixKind; label: string }>; base: string } {
  const prefixes: Array<{ kind: PrefixKind; label: string }> = [];
  let rest = value.trim();

  for (;;) {
    const match = LEADING_PREFIX.exec(rest);
    if (!match?.[1] || !match[0]) break;
    const kind = classifyPrefix(match[1]);
    if (!kind) break;
    prefixes.push({ kind, label: match[1] });
    rest = rest.slice(match[0].length);
  }

  return { prefixes, base: rest.trim() };
}

/**
 * Prefix a subject for a reply or forward, collapsing any chain that arrived
 * with it.
 *
 * The result carries at most two prefixes: one for the action being performed,
 * and — if the incoming subject had one — a single prefix of the other kind, so
 * that forwarding a reply still reads as a reply.
 *
 * A reply mirrors the correspondent's own spelling: answering someone whose
 * client writes "AW:" keeps "AW:", which leaves the subject stable in their
 * list instead of alternating. A forward goes to someone who was not part of
 * that exchange, so it uses this client's own "Fwd:".
 *
 * "Re"/"Fwd" stay literal — they are wire format, not UI text, and the
 * recognition tables above must keep matching whatever language the app runs
 * in. Only the fallback for an empty subject follows the UI locale.
 */
export function prefixSubject(value: string, prefix: 'Re' | 'Fwd'): string {
  const { prefixes, base } = splitSubjectPrefixes(value);
  if (!base) {
    return t(prefix === 'Re' ? 'compose.subject.replyFallback' : 'compose.subject.forwardFallback');
  }

  const actionKind: PrefixKind = prefix === 'Re' ? 'reply' : 'forward';
  const carried = prefixes.find((entry) => entry.kind !== actionKind);

  // Mirroring only applies to the reply: the outermost matching prefix is the
  // one the last sender used.
  const head =
    prefix === 'Re'
      ? prefixes.find((entry) => entry.kind === actionKind)?.label ?? prefix
      : prefix;

  const carriedLabel = carried ? (carried.kind === 'reply' ? 'Re' : 'Fwd') : null;
  return [head, carriedLabel, base].filter(Boolean).join(': ');
}

/**
 * Format a timestamp for display, falling back to the raw value.
 *
 * The fallback matters: an unparseable date must not put the string
 * "Invalid Date" into a forwarded message's headers, where it is sent on to
 * the recipient. `Date` signals a bad input by producing a NaN time rather
 * than by throwing, so the check has to be explicit — a try/catch never fires.
 */
export function formatTimestamp(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(currentLocale(), { ...datetimeFormats[currentLocale()]!.dateTime, year: 'numeric' });
}

/**
 * The quoted header block placed above a forwarded message. This is text the
 * composer writes, so it follows the UI locale — unlike the prefix tables,
 * which have to recognise every language.
 */
export function buildForwardBody(message: Message): string {
  const from = message.from.name
    ? `${message.from.name} <${message.from.email}>`
    : message.from.email;
  return [
    '',
    '',
    t('compose.forward.separator'),
    t('compose.forward.from', { value: from }),
    t('compose.forward.date', { value: formatTimestamp(message.timestamp) }),
    t('compose.forward.subject', { value: message.subject }),
    t('compose.forward.to', {
      value: (message.to ?? []).map((participant) => participant.email).join(', '),
    }),
    ...(message.cc?.length
      ? [t('compose.forward.cc', { value: message.cc.map((participant) => participant.email).join(', ') })]
      : []),
    '',
    messageBodyText(message),
  ].join('\n');
}

export function messageBodyText(message: Message): string {
  return message.bodyText?.trim()
    ? message.bodyText
    : message.bodyHtml ? htmlToPlainText(message.bodyHtml) : '';
}

export function buildReplyBody(message: Message): string {
  const sender = message.from.name
    ? `${message.from.name} <${message.from.email}>`
    : message.from.email;
  return [
    '',
    '',
    t('compose.reply.intro', { date: formatTimestamp(message.timestamp), sender }),
    ...messageBodyText(message).split(/\r\n?|\n/).map((line) => `> ${line}`),
  ].join('\n');
}

/**
 * Append recipients that are not already present, comparing case-insensitively
 * and keeping the spelling that was there first.
 */
export function mergeRecipients(existing: string[], incoming: string[]): string[] {
  const seen = new Set(existing.map((value) => value.toLowerCase()));
  const next = [...existing];
  for (const value of incoming) {
    if (!seen.has(value.toLowerCase())) {
      next.push(value);
      seen.add(value.toLowerCase());
    }
  }
  return next;
}
