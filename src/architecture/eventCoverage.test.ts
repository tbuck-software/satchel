import { readdirSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/*
 * Commands have `backend.ipc-coverage`; events had nothing — which is how
 * `mail:open-compose` was emitted into a tree with no listener while every
 * check stayed green. An emitter without a listener is a dead feature; a
 * listener without an emitter is a rename that landed on one side only.
 */

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));

/** Rust roots that may talk to the webview. */
const RUST_ROOTS = [join(REPOSITORY_ROOT, 'src-tauri', 'src'), join(REPOSITORY_ROOT, 'crates')];
const FRONTEND_ROOT = join(REPOSITORY_ROOT, 'src');

/**
 * Events emitted from Rust with no listener in the frontend.
 *
 * Every entry needs the condition under which it goes away again. The three
 * sync events that used to belong here were removed from Rust instead:
 * `connection:state` already tells the UI whether a sync is running, so three
 * more listeners would have shown nothing new.
 */
const KNOWN_UNHEARD: Record<string, string> = {};

/**
 * Events the frontend listens for that Rust never emits. Same contract. An
 * entry here means the emitter lives somewhere this scan cannot see.
 */
const KNOWN_UNEMITTED: Record<string, string> = {};

interface Extraction {
  /** Event names the scan could read. */
  events: string[];
  /** Call sites whose event name it could not resolve — always a failure. */
  unresolved: string[];
}

function sourcesUnder(root: string, extensions: string[]): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return sourcesUnder(path, extensions);
    if (!entry.isFile() || !extensions.includes(extname(entry.name))) return [];
    return [path];
  });
}

/**
 * The literals a Rust binding can hold.
 *
 * The name argument of an emit is usually a literal, but two call sites in the
 * tree pass an identifier: a file-level `const PROGRESS_EVENT: &str` and a
 * local `let event_name = if … { "a" } else { "b" }`. Both resolve by reading
 * every string literal out of the binding's initialiser, which is exact for
 * the first and a superset for the second — a superset is the safe direction,
 * since an over-collected name only makes the scan demand a listener.
 */
function bindingLiterals(source: string, token: string): string[] {
  const binding = new RegExp(
    `(?:const|let)\\s+(?:mut\\s+)?${token}\\s*(?::[^=]*)?=\\s*([\\s\\S]*?);`,
  ).exec(source);
  if (!binding) return [];
  return [...binding[1]!.matchAll(/"([^"\n]+)"/g)].map((match) => match[1]!);
}

/** Drops comments so a commented-out emitter is not read as one. */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^[ \t]*\/\/[^\n]*/gm, '');
}

/**
 * Every event name passed to `emit`, `emit_to` or `emit_filter`, in both the
 * method form (`app.emit("x", …)`) and the trait form
 * (`tauri::Emitter::emit(&app, "x", …)`).
 */
export function emittedEvents(sources: string[]): Extraction {
  const events = new Set<string>();
  const unresolved: string[] = [];
  // The method form names the event first, except `emit_to`, which names the
  // window label first. The trait form always names the handle first.
  const call = /(\.|Emitter::)\s*(emit|emit_to|emit_filter)\s*\(([^;]*?)\)/g;

  for (const raw of sources) {
    const source = stripComments(raw);
    for (const match of source.matchAll(call)) {
      const trait = match[1] === 'Emitter::';
      const position = (trait ? 1 : 0) + (match[2] === 'emit_to' ? 1 : 0);
      const argument = splitArguments(match[3]!)[position]?.trim() ?? '';

      if (argument.startsWith('"')) {
        events.add(argument.slice(1, -1));
        continue;
      }
      const resolved = bindingLiterals(source, argument.replace(/^&/, ''));
      if (resolved.length === 0) unresolved.push(argument);
      else for (const name of resolved) events.add(name);
    }
  }
  return { events: [...events].sort(), unresolved: unresolved.sort() };
}

/** Splits an argument list on the commas that are not inside brackets. */
function splitArguments(list: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const char of list) {
    if ('([{'.includes(char)) depth += 1;
    else if (')]}'.includes(char)) depth -= 1;
    if (char === ',' && depth === 0) {
      parts.push(current);
      current = '';
      continue;
    }
    current += char;
  }
  parts.push(current);
  return parts;
}

/**
 * Every event name passed to `listen(…)` or to the `listenToBackend` wrapper.
 *
 * A call whose first argument is not a literal must iterate a constant whose
 * name ends in `EVENTS` — `src/stores/outbox.ts` is the one case. That keeps
 * the scan complete without parsing the call graph, and the third test below
 * fails on any other shape rather than quietly missing it.
 */
export function listenedEvents(sources: string[]): Extraction {
  const events = new Set<string>();
  const unresolved: string[] = [];

  for (const raw of sources) {
    const source = stripComments(raw);
    const constants = [
      ...source.matchAll(/const\s+([A-Za-z_]*EVENTS)\s*(?::[^=]+)?=\s*\[([^\]]*)\]/g),
    ];
    const iterated = new Set<string>();
    for (const [, name, body] of constants) {
      for (const literal of body!.matchAll(/'([^'\n]+)'|"([^"\n]+)"/g)) {
        events.add(literal[1] ?? literal[2]!);
      }
      for (const use of source.matchAll(new RegExp(`${name!}\\s*\\.map\\(\\s*\\(?\\s*(\\w+)`, 'g'))) {
        iterated.add(use[1]!);
      }
    }

    for (const match of source.matchAll(
      /\blisten(?:ToBackend)?\s*(?:<[\s\S]*?>)?\s*\(\s*([^,\s)]+)/g,
    )) {
      const argument = match[1]!;
      if (/^['"`]/.test(argument)) {
        events.add(argument.slice(1, -1));
        continue;
      }
      if (!iterated.has(argument)) unresolved.push(argument);
    }
  }
  return { events: [...events].sort(), unresolved: unresolved.sort() };
}

export function eventParity(
  rustSources: string[],
  frontendSources: string[],
): { unheard: string[]; unemitted: string[] } {
  const emitted = emittedEvents(rustSources).events;
  const heard = new Set(listenedEvents(frontendSources).events);
  const emittedSet = new Set(emitted);
  return {
    unheard: emitted.filter((name) => !heard.has(name) && !(name in KNOWN_UNHEARD)),
    unemitted: [...heard].filter((name) => !emittedSet.has(name) && !(name in KNOWN_UNEMITTED)).sort(),
  };
}

const RUST_SOURCES = RUST_ROOTS.flatMap((root) => sourcesUnder(root, ['.rs']))
  // Test fixtures name events they never emit for real.
  .filter((path) => !path.includes(`${'tests'}/`) && !path.endsWith('tests.rs'))
  .map((path) => readFileSync(path, 'utf8'));

/**
 * The `listenToBackend` wrapper itself. Its `listen` call names the event
 * through its own parameter, which is plumbing rather than a call site — the
 * real names sit at its callers, and those the scan reads.
 */
const EVENT_WRAPPER = join(FRONTEND_ROOT, 'services', 'appEvents.ts');

const FRONTEND_FILES = sourcesUnder(FRONTEND_ROOT, ['.ts', '.vue']).filter(
  (path) => !path.includes('.test.') && path !== EVENT_WRAPPER,
);
const FRONTEND_SOURCES = FRONTEND_FILES.map((path) => readFileSync(path, 'utf8'));

describe('event coverage', () => {
  it('reports an emitter without a listener', () => {
    const rust = ['let _ = app.emit("thread:archived", ());'];
    const frontend = ["await listen('mail:incoming', () => {});"];

    expect(eventParity(rust, frontend).unheard).toEqual(['thread:archived']);
  });

  it('reports a listener without an emitter', () => {
    const rust = ['let _ = app.emit("mail:incoming", ());'];
    const frontend = [
      "await listen('mail:incoming', () => {});",
      "await listen<{ id: string }>('mail:incomming', () => {});",
    ];

    expect(eventParity(rust, frontend).unemitted).toEqual(['mail:incomming']);
  });

  it('resolves an emitter that names its event through a binding', () => {
    const source = [
      'const PROGRESS_EVENT: &str = "inline-cache:progress";',
      'let _ = app.emit(PROGRESS_EVENT, &snapshot);',
      'let event_name = if undo { "outbox:undo-window" } else { "outbox:scheduled" };',
      'let _ = tauri::Emitter::emit(&app, event_name, payload);',
    ].join('\n');

    expect(emittedEvents([source])).toEqual({
      events: ['inline-cache:progress', 'outbox:scheduled', 'outbox:undo-window'],
      unresolved: [],
    });
  });

  it('reports an emitter whose event name it cannot read', () => {
    const source = 'let _ = app.emit(whatever_the_caller_passed, ());';

    expect(emittedEvents([source]).unresolved).toEqual(['whatever_the_caller_passed']);
  });

  it('reads the listeners a constant carries and rejects any other indirection', () => {
    const named = [
      "export const OUTBOX_EVENTS = ['outbox:failed', 'outbox:sent'] as const;",
      'OUTBOX_EVENTS.map((eventName) => mod.listen(eventName, () => load()));',
    ].join('\n');
    const anonymous = 'for (const name of names) await mod.listen(name, handler);';

    expect(listenedEvents([named])).toEqual({
      events: ['outbox:failed', 'outbox:sent'],
      unresolved: [],
    });
    expect(listenedEvents([anonymous]).unresolved).toEqual(['name']);
  });

  it('ignores a commented-out emitter', () => {
    const source = ['// let _ = app.emit("thread:archived", ());', 'let _ = app.emit("x:y", ());'];

    expect(emittedEvents([source.join('\n')]).events).toEqual(['x:y']);
  });

  it('gives every allowlisted event a reason that names its exit condition', () => {
    const thin = [...Object.entries(KNOWN_UNHEARD), ...Object.entries(KNOWN_UNEMITTED)]
      .filter(([, reason]) => reason.trim().length < 10)
      .map(([name]) => name);

    expect(thin).toEqual([]);
  });

  it('carries no allowlist entry that is wired again', () => {
    const emitted = new Set(emittedEvents(RUST_SOURCES).events);
    const heard = new Set(listenedEvents(FRONTEND_SOURCES).events);

    expect(Object.keys(KNOWN_UNHEARD).filter((name) => heard.has(name))).toEqual([]);
    expect(Object.keys(KNOWN_UNEMITTED).filter((name) => emitted.has(name))).toEqual([]);
  });

  it('reads every emit and every listen in the tree', () => {
    expect(emittedEvents(RUST_SOURCES).unresolved).toEqual([]);
    expect(listenedEvents(FRONTEND_SOURCES).unresolved).toEqual([]);
    expect(emittedEvents(RUST_SOURCES).events.length).toBeGreaterThan(15);
  });

  it('has no unheard event and no listener without an emitter', () => {
    const { unheard, unemitted } = eventParity(RUST_SOURCES, FRONTEND_SOURCES);

    expect(unheard, 'emitted from Rust, nothing listens').toEqual([]);
    expect(unemitted, 'listened for, nothing emits it').toEqual([]);
  });
});
