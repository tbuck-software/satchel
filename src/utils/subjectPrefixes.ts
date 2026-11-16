/**
 * Reply and forward prefixes of a subject ("Re: AW: Fwd: …"), apart from the
 * composer's other text helpers so that code which only strips them (the task
 * editor naming a task after a mail) does not load the HTML-to-text pipeline.
 */
import REPLY_PREFIXES from '../../crates/satchel-core/src/email/reply-prefixes.json';
import FORWARD_PREFIXES from '../../crates/satchel-core/src/email/forward-prefixes.json';

// The composer shares Core's localized prefix vocabulary. Single-letter and
// reference labels stay out because they also begin ordinary subject text.
export type PrefixKind = 'reply' | 'forward';

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
