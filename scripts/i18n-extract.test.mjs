import { describe, expect, it } from 'vitest';
import { scriptFindings } from './i18n-extract.mjs';

/*
 * `--check` filters script findings through a German heuristic, which is right
 * for ids, CSS classes and backend keys and wrong for exactly one thing: a
 * sentence handed to `new Error(...)`. That sentence reaches the user, because
 * around 75 call sites interpolate `error.message` into a translated string —
 * so an English literal there is copy the check has to see.
 */

const kinds = (source) => scriptFindings(source).map((entry) => `${entry.kind} ${entry.text}`);

describe('i18n extractor: thrown sentences', () => {
  it('reports an English sentence thrown as an error', () => {
    expect(kinds("throw new Error('Something broke');")).toEqual(['error Something broke']);
  });

  it('reports it from a subclass too', () => {
    expect(kinds("throw new BackendError('Something broke');")).toEqual(['error Something broke']);
  });

  it('leaves a translated throw alone', () => {
    // `t(` is what sits before the literal, so this is an ordinary script
    // finding and the German heuristic drops it.
    expect(kinds("throw new Error(t('compose.window.popOutFailed'));"))
      .toEqual(['script compose.window.popOutFailed']);
  });

  it('leaves a message key alone', () => {
    expect(kinds("throw new BackendError('backend.errors.desktopUnavailable');")).toEqual([]);
  });

  it('does not mistake an ordinary string for a thrown one', () => {
    expect(kinds("const status = 'Something broke';")).toEqual(['script Something broke']);
  });
});
