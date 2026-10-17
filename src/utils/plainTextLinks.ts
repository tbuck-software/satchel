export type PlainTextToken =
  | { kind: 'text'; text: string }
  | { kind: 'link'; text: string; href: string };

const LINK_PATTERN = /https?:\/\/[^\s<>"'`]+|[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,63}/gi;
const SIMPLE_TRAILING_PUNCTUATION = new Set(['.', ',', ';', ':', '!', '?', '\u2026']);
const CLOSING_PAIRS: Record<string, string> = {
  ')': '(',
  ']': '[',
  '}': '{',
};

function splitTrailingPunctuation(candidate: string): { url: string; trailing: string } {
  let end = candidate.length;

  while (end > 0) {
    const last = candidate[end - 1] ?? '';
    if (SIMPLE_TRAILING_PUNCTUATION.has(last)) {
      end -= 1;
      continue;
    }

    if (!last) break;
    const opening = CLOSING_PAIRS[last];
    if (!opening) break;

    const current = candidate.slice(0, end);
    const openingCount = current.split(opening).length - 1;
    const closingCount = current.split(last).length - 1;
    if (closingCount <= openingCount) break;
    end -= 1;
  }

  return {
    url: candidate.slice(0, end),
    trailing: candidate.slice(end),
  };
}

export function tokenizePlainTextLinks(input: string): PlainTextToken[] {
  const tokens: PlainTextToken[] = [];
  let cursor = 0;

  for (const match of input.matchAll(LINK_PATTERN)) {
    const start = match.index ?? 0;
    if (start > cursor) {
      tokens.push({ kind: 'text', text: input.slice(cursor, start) });
    }

    const candidate = match[0];
    const { url, trailing } = splitTrailingPunctuation(candidate);
    if (url) {
      const href = /^https?:\/\//i.test(url) ? url : `mailto:${url}`;
      tokens.push({ kind: 'link', text: url, href });
    }
    if (trailing) {
      tokens.push({ kind: 'text', text: trailing });
    }
    cursor = start + candidate.length;
  }

  if (cursor < input.length) {
    tokens.push({ kind: 'text', text: input.slice(cursor) });
  }

  return tokens.length > 0 ? tokens : [{ kind: 'text', text: input }];
}
