/**
 * Keys defined by more than one part of the threads store.
 *
 * The façade spreads its parts into one object, and a spread lets the later
 * part win without a word. The compile-time check in `threads.ts` covers the
 * real parts; this is the same question asked at run time, for tests.
 */
export function collidingKeys(parts: Record<string, object>): string[] {
  const owners = new Map<string, string>();
  const collisions = new Set<string>();
  for (const [name, part] of Object.entries(parts)) {
    for (const key of Object.keys(part)) {
      if (owners.has(key) && owners.get(key) !== name) collisions.add(key);
      else owners.set(key, name);
    }
  }
  return [...collisions].sort();
}

/** Keys two surfaces both define. `never` when they are disjoint. */
export type Overlap<A, B> = Extract<keyof A, keyof B>;
