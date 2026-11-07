import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { LABS_FLAG_IDS } from '@/labs/flags';
import { LABS_FLAG_COPY } from '@/labs/flagCopy';

/*
 * A Labs flag lives in two lists: `src/labs/flags.ts` for the app and
 * `db::labs::LABS_FLAGS` for the backend, which refuses ids it does not know
 * (ADR 0020). A flag in one list only is a switch that cannot be saved, or one
 * no screen offers. Read straight from the Rust source; there is no build step
 * that would generate the list.
 */
const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const RUST_REGISTRY = join(REPOSITORY_ROOT, 'crates/satchel-core/src/db/labs.rs');

function rustFlagIds(source: string): string[] {
  const constants = new Map(
    [...source.matchAll(/pub const ([A-Z_]+): &str = "([^"]+)";/g)].map((match) => [match[1]!, match[2]!]),
  );
  const list = /pub const LABS_FLAGS: &\[&str\] = &\[([^\]]*)\];/.exec(source)?.[1];
  if (list === undefined) throw new Error('LABS_FLAGS not found in db/labs.rs');
  return list
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const literal = /^"([^"]+)"$/.exec(entry)?.[1];
      const value = literal ?? constants.get(entry);
      if (!value) throw new Error(`cannot resolve ${entry} in LABS_FLAGS`);
      return value;
    });
}

describe('labs registry', () => {
  it('names the same flags in TypeScript and Rust, in the same order', () => {
    expect(rustFlagIds(readFileSync(RUST_REGISTRY, 'utf8'))).toEqual(LABS_FLAG_IDS);
  });

  it('has a title and a description for every flag', () => {
    expect(Object.keys(LABS_FLAG_COPY).sort()).toEqual([...LABS_FLAG_IDS].sort());
  });

  it('reads constants and literals out of the Rust list', () => {
    const source = 'pub const CHATS: &str = "chats";\npub const LABS_FLAGS: &[&str] = &[CHATS, "maps"];';
    expect(rustFlagIds(source)).toEqual(['chats', 'maps']);
  });
});
